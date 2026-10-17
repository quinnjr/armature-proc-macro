use proc_macro::TokenStream;
use quote::{quote, quote_spanned};
use syn::{
    FnArg, ItemFn, Lit, Meta, Pat, ReturnType, Token, parse::Parse, parse::ParseStream,
    parse_macro_input, punctuated::Punctuated,
};

/// Parsed `#[cache(...)]` options.
///
/// Supports `ttl = <seconds>`, `key = "<template>"` and one or more
/// `tag = "<tag>"` entries.
#[derive(Default)]
struct CacheArgs {
    ttl_secs: Option<u64>,
    key_template: Option<String>,
    tags: Vec<String>,
}

impl Parse for CacheArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = CacheArgs::default();

        if input.is_empty() {
            return Ok(args);
        }

        let items = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        for item in items {
            let Meta::NameValue(nv) = item else {
                return Err(syn::Error::new_spanned(
                    item,
                    "expected `ttl = N`, `key = \"...\"` or `tag = \"...\"`",
                ));
            };
            let Some(ident) = nv.path.get_ident() else {
                continue;
            };
            match ident.to_string().as_str() {
                "ttl" => {
                    if let syn::Expr::Lit(expr_lit) = &nv.value
                        && let Lit::Int(lit_int) = &expr_lit.lit
                    {
                        args.ttl_secs = Some(lit_int.base10_parse()?);
                    } else {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "ttl must be an integer number of seconds",
                        ));
                    }
                }
                "key" => {
                    if let syn::Expr::Lit(expr_lit) = &nv.value
                        && let Lit::Str(lit_str) = &expr_lit.lit
                    {
                        args.key_template = Some(lit_str.value());
                    } else {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "key must be a string literal template",
                        ));
                    }
                }
                "tag" => {
                    if let syn::Expr::Lit(expr_lit) = &nv.value
                        && let Lit::Str(lit_str) = &expr_lit.lit
                    {
                        args.tags.push(lit_str.value());
                    } else {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "tag must be a string literal",
                        ));
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        &nv.path,
                        format!("unknown cache option `{other}` (expected ttl, key or tag)"),
                    ));
                }
            }
        }

        Ok(args)
    }
}

/// Collect every binding introduced by a parameter pattern, in source order.
///
/// Destructured parameters (`Path(id): Path<u64>`, `(a, b): (u32, u32)`,
/// `Point { x, y }: Point`) contribute each bound name, so every argument
/// value participates in the cache key. A pattern that discards data (`_`,
/// `..`) cannot be keyed: the discarded part could still influence the
/// result, and two calls differing only there would share a cache entry.
fn collect_bindings(pat: &Pat, out: &mut Vec<syn::Ident>) -> syn::Result<()> {
    match pat {
        Pat::Ident(pat_ident) => {
            out.push(pat_ident.ident.clone());
            Ok(())
        }
        Pat::Tuple(tuple) => tuple
            .elems
            .iter()
            .try_for_each(|elem| collect_bindings(elem, out)),
        Pat::TupleStruct(tuple) => tuple
            .elems
            .iter()
            .try_for_each(|elem| collect_bindings(elem, out)),
        Pat::Struct(pat_struct) => {
            if let Some(rest) = &pat_struct.rest {
                return Err(syn::Error::new_spanned(
                    rest,
                    "#[cache] cannot key an argument whose fields are elided with `..`; bind every field",
                ));
            }
            pat_struct
                .fields
                .iter()
                .try_for_each(|field| collect_bindings(&field.pat, out))
        }
        Pat::Slice(slice) => slice
            .elems
            .iter()
            .try_for_each(|elem| collect_bindings(elem, out)),
        Pat::Reference(reference) => collect_bindings(&reference.pat, out),
        Pat::Paren(paren) => collect_bindings(&paren.pat, out),
        Pat::Type(pat_type) => collect_bindings(&pat_type.pat, out),
        // A unit struct / unit variant pattern carries no data.
        Pat::Path(_) => Ok(()),
        Pat::Wild(_) => Err(syn::Error::new_spanned(
            pat,
            "#[cache] cannot key an argument bound to `_`; give it a name so it participates in the cache key",
        )),
        Pat::Rest(_) => Err(syn::Error::new_spanned(
            pat,
            "#[cache] cannot key an argument whose elements are elided with `..`; bind every element",
        )),
        other => Err(syn::Error::new_spanned(
            other,
            "#[cache] cannot derive a cache key from this argument pattern",
        )),
    }
}

/// Every name bound by the function's non-receiver parameters, in order.
fn arg_bindings(input_fn: &ItemFn) -> syn::Result<Vec<syn::Ident>> {
    let mut out = Vec::new();
    for arg in &input_fn.sig.inputs {
        if let FnArg::Typed(pat_type) = arg {
            collect_bindings(&pat_type.pat, &mut out)?;
        }
    }
    Ok(out)
}

/// One piece of a parsed `key = "..."` template.
#[derive(Debug, PartialEq)]
enum TemplatePart {
    Literal(String),
    /// Index into the function's bindings.
    Value(usize),
}

/// Characters a template value can contain after escaping: the RFC 3986
/// unreserved set plus `%` (the escape introducer).
fn survives_escaping(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~' | '%')
}

/// Parse a custom key template against the function's bindings.
///
/// Placeholders are `{}` (next positional binding), `{N}` (binding N) and
/// `{name}` (binding by name); `{{` / `}}` are literal braces. Format specs are
/// rejected. Every binding must be referenced (as `format!` used to require),
/// and two placeholders must be separated by a literal containing at least one
/// character that escaped values can never contain (e.g. `:` or `/`), so that
/// the rendered key decodes to exactly one tuple of values —
/// `("1:2", "3")` and `("1", "2:3")` can no longer share an entry.
fn parse_template(
    template: &str,
    bindings: &[syn::Ident],
    span: proc_macro2::Span,
) -> syn::Result<Vec<TemplatePart>> {
    let err = |msg: String| syn::Error::new(span, msg);
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut next_positional = 0usize;
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '}' => return Err(err("unmatched `}` in cache key template (use `}}`)".into())),
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(ch) => name.push(ch),
                        None => {
                            return Err(err("unclosed `{` in cache key template".into()));
                        }
                    }
                }
                let name = name.trim();
                if name.contains(':') {
                    return Err(err(format!(
                        "format specs are not supported in cache key templates (`{{{name}}}`)"
                    )));
                }
                let index = if name.is_empty() {
                    let i = next_positional;
                    next_positional += 1;
                    i
                } else if let Ok(i) = name.parse::<usize>() {
                    i
                } else {
                    bindings.iter().position(|b| b == name).ok_or_else(|| {
                        err(format!(
                            "cache key template references `{name}`, which is not an argument of this function"
                        ))
                    })?
                };
                if index >= bindings.len() {
                    return Err(err(format!(
                        "cache key template placeholder {index} is out of range: the function has {} keyable argument(s)",
                        bindings.len()
                    )));
                }
                if !literal.is_empty() {
                    parts.push(TemplatePart::Literal(std::mem::take(&mut literal)));
                }
                parts.push(TemplatePart::Value(index));
            }
            other => literal.push(other),
        }
    }
    if !literal.is_empty() {
        parts.push(TemplatePart::Literal(literal));
    }

    // Two values must be split by a delimiter an escaped value cannot contain.
    let mut seen_value = false;
    let mut delimited = true;
    for part in &parts {
        match part {
            TemplatePart::Value(_) => {
                if seen_value && !delimited {
                    return Err(err(
                        "cache key template placeholders must be separated by a delimiter such as `:` or `/` \
                         (otherwise different arguments can render to the same key)"
                            .into(),
                    ));
                }
                seen_value = true;
                delimited = false;
            }
            TemplatePart::Literal(text) => {
                delimited |= text.chars().any(|c| !survives_escaping(c));
            }
        }
    }

    for (i, binding) in bindings.iter().enumerate() {
        if !parts.contains(&TemplatePart::Value(i)) {
            return Err(err(format!(
                "cache key template does not use argument `{binding}`; every argument must be part of the key \
                 or calls differing only in `{binding}` would share a cache entry"
            )));
        }
    }

    Ok(parts)
}

pub fn cache_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as CacheArgs);
    let input_fn = parse_macro_input!(item as ItemFn);
    match expand(args, input_fn) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(e) => TokenStream::from(e.to_compile_error()),
    }
}

fn expand(args: CacheArgs, input_fn: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    let fn_name = &input_fn.sig.ident;
    let fn_vis = &input_fn.vis;
    let fn_sig = &input_fn.sig;
    let fn_block = &input_fn.block;
    let fn_attrs = &input_fn.attrs;

    let bindings = arg_bindings(&input_fn)?;
    let ttl_secs = args.ttl_secs.unwrap_or(3600);
    let tags = args.tags;

    // Build the expression that computes the cache key at runtime. It
    // evaluates to `Option<String>`: `None` means an argument could not be
    // encoded, and the call then bypasses the cache instead of risking a key
    // that collides with another argument tuple.
    let cache_key_expr = if let Some(template) = &args.key_template {
        // Custom template: each interpolated value is its `Display` output,
        // percent-escaped so it can never contain the template's delimiters.
        let parts = parse_template(template, &bindings, proc_macro2::Span::call_site())?;
        let pushes = parts.iter().map(|part| match part {
            TemplatePart::Literal(text) => quote! { __key.push_str(#text); },
            TemplatePart::Value(i) => {
                let ident = &bindings[*i];
                quote_spanned! {ident.span()=>
                    __armature_cache_escape(&::std::string::ToString::to_string(&#ident), &mut __key);
                }
            }
        });
        quote! {{
            fn __armature_cache_escape(value: &str, out: &mut String) {
                for byte in value.bytes() {
                    if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                        out.push(byte as char);
                    } else {
                        const HEX: &[u8; 16] = b"0123456789ABCDEF";
                        out.push('%');
                        out.push(HEX[(byte >> 4) as usize] as char);
                        out.push(HEX[(byte & 0x0F) as usize] as char);
                    }
                }
            }
            let mut __key = String::new();
            #(#pushes)*
            Some(__key)
        }}
    } else {
        // Default key: a scope naming this exact function (module path, the
        // `Self` type for methods, function name, attribute location and
        // return type — so equally named functions on different types,
        // impls or generic instantiations never share entries), followed by
        // every argument binding as length-prefixed JSON. Length prefixes
        // make the encoding injective: no two argument tuples render alike.
        let has_receiver = input_fn
            .sig
            .inputs
            .first()
            .is_some_and(|arg| matches!(arg, FnArg::Receiver(_)));
        let self_type = if has_receiver {
            quote! { ::std::any::type_name::<Self>() }
        } else {
            quote! { "" }
        };
        let return_type = match &input_fn.sig.output {
            ReturnType::Type(_, ty) if !type_has_impl_trait(ty) => {
                quote! { ::std::any::type_name::<#ty>() }
            }
            _ => quote! { "" },
        };
        let fn_name_str = fn_name.to_string();
        let encode_args = bindings.iter().map(|ident| {
            quote_spanned! {ident.span()=>
                let __encoded = serde_json::to_string(&#ident)?;
                __key.push_str(&__encoded.len().to_string());
                __key.push(':');
                __key.push_str(&__encoded);
            }
        });
        quote! {{
            let __scope = [
                module_path!(),
                "::",
                #self_type,
                "::",
                #fn_name_str,
                concat!("@", line!(), ":", column!(), "->"),
                #return_type,
            ]
            .concat();
            let __build = || -> ::std::result::Result<String, serde_json::Error> {
                let mut __key = String::with_capacity(__scope.len() + 32);
                __key.push_str(&__scope.len().to_string());
                __key.push(':');
                __key.push_str(&__scope);
                #(#encode_args)*
                Ok(__key)
            };
            __build().ok()
        }}
    };

    let cache_code = if tags.is_empty() {
        // Simple cache without tags
        quote! {
            #(#fn_attrs)*
            #fn_vis #fn_sig {
                use std::time::Duration;

                // Generate cache key from the function arguments. `None`
                // (an argument failed to encode) bypasses the cache.
                let cache_key: Option<String> = #cache_key_expr;

                // Try to get from cache. The stored value is the serialized
                // success payload, so deserialize into it and wrap in `Ok`.
                if let Some(cache_key) = &cache_key {
                    if let Ok(Some(cached)) = __cache.get_json(cache_key).await {
                        if let Ok(value) = serde_json::from_str(&cached) {
                            return Ok(value);
                        }
                    }
                }

                // Execute function
                let result = (|| async #fn_block)().await;

                // Cache successful results
                if let (Some(cache_key), Ok(success_result)) = (&cache_key, &result) {
                    if let Ok(json) = serde_json::to_string(success_result) {
                        let _ = __cache.set_json(
                            cache_key,
                            json,
                            Some(Duration::from_secs(#ttl_secs))
                        ).await;
                    }
                }

                result
            }
        }
    } else {
        // Tagged cache
        let tag_literals = tags.iter().map(|t| t.as_str());
        quote! {
            #(#fn_attrs)*
            #fn_vis #fn_sig {
                use std::time::Duration;

                // Generate cache key from the function arguments. `None`
                // (an argument failed to encode) bypasses the cache.
                let cache_key: Option<String> = #cache_key_expr;

                // Try to get from cache. The stored value is the serialized
                // success payload, so deserialize into it and wrap in `Ok`.
                if let Some(cache_key) = &cache_key {
                    if let Ok(Some(cached)) = __tagged_cache.get(cache_key).await {
                        if let Ok(value) = serde_json::from_str(&cached) {
                            return Ok(value);
                        }
                    }
                }

                // Execute function
                let result = (|| async #fn_block)().await;

                // Cache successful results with tags
                if let (Some(cache_key), Ok(success_result)) = (&cache_key, &result) {
                    if let Ok(json) = serde_json::to_string(success_result) {
                        let tags: &[&str] = &[ #( #tag_literals ),* ];
                        let _ = __tagged_cache.set_with_tags(
                            cache_key,
                            json,
                            tags,
                            Some(Duration::from_secs(#ttl_secs))
                        ).await;
                    }
                }

                result
            }
        }
    };

    Ok(cache_code)
}

/// Whether a type mentions `impl Trait` (which `type_name::<T>()` cannot name).
fn type_has_impl_trait(ty: &syn::Type) -> bool {
    quote!(#ty)
        .to_string()
        .split_whitespace()
        .any(|tok| tok == "impl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::Span;

    fn idents(names: &[&str]) -> Vec<syn::Ident> {
        names
            .iter()
            .map(|n| syn::Ident::new(n, Span::call_site()))
            .collect()
    }

    fn bindings_of(sig: &str) -> syn::Result<Vec<syn::Ident>> {
        let f: ItemFn = syn::parse_str(&format!("{sig} {{}}")).unwrap();
        arg_bindings(&f)
    }

    #[test]
    fn template_parses_positional_named_and_escaped_braces() {
        let b = idents(&["a", "b"]);
        let parts = parse_template("x{{:{}:{b}}}", &b, Span::call_site()).unwrap();
        assert_eq!(
            parts,
            vec![
                TemplatePart::Literal("x{:".into()),
                TemplatePart::Value(0),
                TemplatePart::Literal(":".into()),
                TemplatePart::Value(1),
                TemplatePart::Literal("}".into()),
            ]
        );
    }

    #[test]
    fn adjacent_placeholders_are_rejected() {
        let b = idents(&["a", "b"]);
        assert!(parse_template("{}{}", &b, Span::call_site()).is_err());
        // Only unreserved characters in between: still ambiguous.
        assert!(parse_template("{}x{}", &b, Span::call_site()).is_err());
        assert!(parse_template("{}%{}", &b, Span::call_site()).is_err());
        assert!(parse_template("{}:{}", &b, Span::call_site()).is_ok());
        assert!(parse_template("{}/x/{}", &b, Span::call_site()).is_ok());
    }

    #[test]
    fn template_must_use_every_argument() {
        let b = idents(&["tenant", "id"]);
        let err = parse_template("item:{id}", &b, Span::call_site()).unwrap_err();
        assert!(err.to_string().contains("tenant"));
    }

    #[test]
    fn template_rejects_unknown_names_specs_and_ranges() {
        let b = idents(&["id"]);
        assert!(parse_template("{other}", &b, Span::call_site()).is_err());
        assert!(parse_template("{id:?}", &b, Span::call_site()).is_err());
        assert!(parse_template("{1}", &b, Span::call_site()).is_err());
        assert!(parse_template("{", &b, Span::call_site()).is_err());
        assert!(parse_template("}", &b, Span::call_site()).is_err());
    }

    #[test]
    fn destructured_parameters_contribute_every_binding() {
        let b = bindings_of(
            "async fn f(&self, Path(id): Path<u64>, (a, b): (u8, u8), P { x, y: z }: P, [p, q]: [u8; 2])",
        )
        .unwrap();
        let names: Vec<String> = b.iter().map(ToString::to_string).collect();
        assert_eq!(names, ["id", "a", "b", "x", "z", "p", "q"]);
    }

    #[test]
    fn unkeyable_parameters_are_compile_errors() {
        assert!(bindings_of("async fn f(_: u8)").is_err());
        assert!(bindings_of("async fn f((a, _): (u8, u8))").is_err());
        assert!(bindings_of("async fn f(P { x, .. }: P)").is_err());
        assert!(bindings_of("async fn f((a, ..): (u8, u8, u8))").is_err());
    }
}
