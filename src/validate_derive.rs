// Derive macro for `#[derive(Validate)]` used by `armature-validation`.
//
// Generates an `armature_validation::Validate` implementation that runs the
// built-in validators against annotated fields and recurses into nested
// `#[validate]`-annotated struct fields.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Data, DeriveInput, Expr, Fields, LitInt, LitStr, Path, parse_macro_input, spanned::Spanned,
};

/// A single validation check to emit for a field.
enum Check {
    Length {
        min: Option<Expr>,
        max: Option<Expr>,
    },
    Range {
        min: Option<Expr>,
        max: Option<Expr>,
    },
    Email,
    Url,
    Regex(LitStr),
    Required,
    /// A custom function, and whether it is also passed the field name.
    Custom {
        func: Expr,
        with_field: bool,
    },
    Nested,
}

pub fn validate_derive_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return Err(syn::Error::new(
                    input.span(),
                    "#[derive(Validate)] only supports structs with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new(
                input.span(),
                "#[derive(Validate)] can only be applied to structs",
            ));
        }
    };

    let krate = crate_path(input)?;

    let mut field_checks: Vec<TokenStream2> = Vec::new();

    for field in fields {
        let field_ident = field.ident.as_ref().expect("named field");
        let field_name = field_ident.to_string();

        for attr in &field.attrs {
            if !attr.path().is_ident("validate") {
                continue;
            }

            let mut checks: Vec<Check> = Vec::new();

            // Bare `#[validate]` means "recurse into this nested struct".
            if matches!(attr.meta, syn::Meta::Path(_)) {
                checks.push(Check::Nested);
            } else {
                attr.parse_nested_meta(|meta| {
                    let ident = meta
                        .path
                        .get_ident()
                        .map(|i| i.to_string())
                        .unwrap_or_default();

                    match ident.as_str() {
                        "length" => {
                            let (min, max) = parse_min_max(&meta)?;
                            checks.push(Check::Length { min, max });
                            Ok(())
                        }
                        "range" => {
                            let (min, max) = parse_min_max(&meta)?;
                            checks.push(Check::Range { min, max });
                            Ok(())
                        }
                        "email" => {
                            checks.push(Check::Email);
                            Ok(())
                        }
                        "url" => {
                            checks.push(Check::Url);
                            Ok(())
                        }
                        "required" => {
                            checks.push(Check::Required);
                            Ok(())
                        }
                        "nested" => {
                            checks.push(Check::Nested);
                            Ok(())
                        }
                        "regex" => {
                            let pattern = parse_regex_pattern(&meta)?;
                            checks.push(Check::Regex(pattern));
                            Ok(())
                        }
                        "custom" | "custom_with_field" => {
                            // `custom = "path"` or `custom(path)`
                            let func: Expr = if meta.input.peek(syn::Token![=]) {
                                let lit: LitStr = meta.value()?.parse()?;
                                lit.parse()?
                            } else {
                                let content;
                                syn::parenthesized!(content in meta.input);
                                content.parse()?
                            };
                            checks.push(Check::Custom {
                                func,
                                with_field: ident == "custom_with_field",
                            });
                            Ok(())
                        }
                        other => Err(meta.error(format!("unknown validate rule `{}`", other))),
                    }
                })?;
            }

            for check in checks {
                field_checks.push(emit_check(&krate, field_ident, &field_name, check));
            }
        }
    }

    Ok(quote! {
        impl #impl_generics #krate::Validate for #name #ty_generics #where_clause {
            fn validate(&self) -> ::std::result::Result<(), ::std::vec::Vec<#krate::ValidationError>> {
                let mut errors: ::std::vec::Vec<#krate::ValidationError> = ::std::vec::Vec::new();
                #(#field_checks)*
                if errors.is_empty() {
                    ::std::result::Result::Ok(())
                } else {
                    ::std::result::Result::Err(errors)
                }
            }
        }
    })
}

/// The path generated code uses to reach `armature-validation`.
///
/// An explicit `#[validate(crate = "path")]` on the struct wins. Otherwise
/// the deriving crate's manifest decides: `armature-validation` under
/// whatever name it was given (`av = { package = "armature-validation" }`),
/// or, failing that, the `armature` facade's re-export. A crate that has
/// neither, or whose manifest cannot be read, gets `::armature_validation`.
fn crate_path(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let mut explicit: Option<Path> = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("validate") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("crate") {
                let lit: LitStr = meta.value()?.parse()?;
                explicit = Some(lit.parse()?);
                Ok(())
            } else {
                Err(meta.error("expected `crate = \"path\"`"))
            }
        })?;
    }
    if let Some(path) = explicit {
        return Ok(quote! { #path });
    }

    use proc_macro_crate::{FoundCrate, crate_name};
    let ident = |name: &str| syn::Ident::new(name, proc_macro2::Span::call_site());
    Ok(match crate_name("armature-validation") {
        Ok(FoundCrate::Name(name)) => {
            let name = ident(&name);
            quote! { ::#name }
        }
        // `Itself` covers the crate's own tests and doctests, which are
        // separate crates that see the library as `armature_validation`.
        Ok(FoundCrate::Itself) | Err(_) => match crate_name("armature") {
            Ok(FoundCrate::Name(name)) => {
                let name = ident(&name);
                quote! { ::#name::armature_validation }
            }
            _ => quote! { ::armature_validation },
        },
    })
}

/// Parse `min = <expr>, max = <expr>` (either optional) from a nested meta list.
fn parse_min_max(meta: &syn::meta::ParseNestedMeta) -> syn::Result<(Option<Expr>, Option<Expr>)> {
    let mut min = None;
    let mut max = None;
    meta.parse_nested_meta(|inner| {
        if inner.path.is_ident("min") {
            min = Some(inner.value()?.parse()?);
            Ok(())
        } else if inner.path.is_ident("max") {
            max = Some(inner.value()?.parse()?);
            Ok(())
        } else {
            Err(inner.error("expected `min` or `max`"))
        }
    })?;
    Ok((min, max))
}

/// Parse `regex(pattern = "...")`, `regex("...")`, or `regex = "..."`, and
/// reject a pattern that does not parse, at the pattern's span: a typo should
/// fail the build, not every request at runtime.
fn parse_regex_pattern(meta: &syn::meta::ParseNestedMeta) -> syn::Result<LitStr> {
    let lit: LitStr = if meta.input.peek(syn::Token![=]) {
        meta.value()?.parse()?
    } else {
        let content;
        syn::parenthesized!(content in meta.input);
        if content.peek(syn::Ident) {
            // pattern = "..."
            let _ident: syn::Ident = content.parse()?;
            let _eq: syn::Token![=] = content.parse()?;
        }
        content.parse()?
    };
    if let Err(e) = regex_syntax::Parser::new().parse(&lit.value()) {
        return Err(syn::Error::new(
            lit.span(),
            format!("invalid regex pattern: {e}"),
        ));
    }
    Ok(lit)
}

fn emit_check(
    krate: &TokenStream2,
    field_ident: &syn::Ident,
    field_name: &str,
    check: Check,
) -> TokenStream2 {
    match check {
        Check::Length { min, max } => {
            let min_check = min.map(|m| {
                let m = coerce_usize(&m);
                quote! {
                    if let ::std::result::Result::Err(e) =
                        #krate::MinLength(#m).validate(&self.#field_ident, #field_name)
                    {
                        errors.push(e);
                    }
                }
            });
            let max_check = max.map(|m| {
                let m = coerce_usize(&m);
                quote! {
                    if let ::std::result::Result::Err(e) =
                        #krate::MaxLength(#m).validate(&self.#field_ident, #field_name)
                    {
                        errors.push(e);
                    }
                }
            });
            quote! { #min_check #max_check }
        }
        Check::Range { min, max } => {
            // Every bound is checked as "the comparison does not come out on
            // the right side", so an unordered value (a float NaN) fails
            // rather than slipping past `__v < lo`, which is false for NaN.
            let at_least = |lo: &Expr| {
                quote! {
                    !::core::matches!(
                        ::core::cmp::PartialOrd::partial_cmp(&__v, &(#lo)),
                        ::core::option::Option::Some(
                            ::core::cmp::Ordering::Greater | ::core::cmp::Ordering::Equal
                        )
                    )
                }
            };
            let at_most = |hi: &Expr| {
                quote! {
                    !::core::matches!(
                        ::core::cmp::PartialOrd::partial_cmp(&__v, &(#hi)),
                        ::core::option::Option::Some(
                            ::core::cmp::Ordering::Less | ::core::cmp::Ordering::Equal
                        )
                    )
                }
            };
            let cond = match (&min, &max) {
                (Some(lo), Some(hi)) => {
                    let below = at_least(lo);
                    let above = at_most(hi);
                    quote! { (#below) || (#above) }
                }
                (Some(lo), None) => at_least(lo),
                (None, Some(hi)) => at_most(hi),
                (None, None) => quote! { false },
            };
            let message = match (&min, &max) {
                (Some(min), Some(max)) => {
                    quote! { format!("{} must be between {} and {}", #field_name, #min, #max) }
                }
                (Some(min), None) => {
                    quote! { format!("{} must be at least {}", #field_name, #min) }
                }
                (None, Some(max)) => {
                    quote! { format!("{} must be at most {}", #field_name, #max) }
                }
                (None, None) => quote! { format!("{} is out of range", #field_name) },
            };
            // The submitted value is not attached: errors reach clients.
            quote! {
                {
                    let __v = self.#field_ident;
                    if #cond {
                        errors.push(
                            #krate::ValidationError::new(#field_name, #message)
                                .with_constraint("range"),
                        );
                    }
                }
            }
        }
        Check::Email => quote! {
            if let ::std::result::Result::Err(e) =
                #krate::IsEmail::validate(&self.#field_ident, #field_name)
            {
                errors.push(e);
            }
        },
        Check::Url => quote! {
            if let ::std::result::Result::Err(e) =
                #krate::IsUrl::validate(&self.#field_ident, #field_name)
            {
                errors.push(e);
            }
        },
        // Compiled once per field, on first use, rather than on every
        // `validate()`. The pattern was checked at expansion time, so the only
        // runtime failure left is a compiled-size limit; it fails closed with a
        // message that does not echo the pattern.
        Check::Regex(pattern) => quote! {
            {
                static __ARMATURE_VALIDATE_REGEX: ::std::sync::LazyLock<
                    ::std::option::Option<#krate::Matches>,
                > = ::std::sync::LazyLock::new(|| #krate::Matches::new(#pattern).ok());
                match &*__ARMATURE_VALIDATE_REGEX {
                    ::std::option::Option::Some(__m) => {
                        if let ::std::result::Result::Err(e) =
                            __m.validate(&self.#field_ident, #field_name)
                        {
                            errors.push(e);
                        }
                    }
                    ::std::option::Option::None => {
                        errors.push(
                            #krate::ValidationError::new(
                                #field_name,
                                ::std::format!("{} could not be validated", #field_name),
                            )
                            .with_constraint("regex"),
                        );
                    }
                }
            }
        },
        Check::Required => quote! {
            if let ::std::result::Result::Err(e) =
                #krate::NotEmpty::validate(&self.#field_ident, #field_name)
            {
                errors.push(e);
            }
        },
        // A custom error with an empty field is attributed to this field, so
        // one function can serve several fields without naming any of them.
        Check::Custom { func, with_field } => {
            let call = if with_field {
                quote! { (#func)(&self.#field_ident, #field_name) }
            } else {
                quote! { (#func)(&self.#field_ident) }
            };
            quote! {
                if let ::std::result::Result::Err(mut e) = #call {
                    if e.field.is_empty() {
                        e.field = ::std::string::String::from(#field_name);
                    }
                    errors.push(e);
                }
            }
        }
        // Nested errors are prefixed with this field's name, so two fields of
        // the same type (`billing`, `shipping`) report distinct paths.
        Check::Nested => quote! {
            if let ::std::result::Result::Err(__nested) =
                #krate::Validate::validate(&self.#field_ident)
            {
                errors.extend(__nested.into_iter().map(|mut e| {
                    e.field = if e.field.is_empty() {
                        ::std::string::String::from(#field_name)
                    } else {
                        ::std::format!("{}.{}", #field_name, e.field)
                    };
                    e
                }));
            }
        },
    }
}

/// Coerce a `min`/`max` length literal to `usize` for `MinLength`/`MaxLength`.
fn coerce_usize(expr: &Expr) -> TokenStream2 {
    if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(int),
        ..
    }) = expr
        && int.suffix().is_empty()
        && let Ok(lit) = syn::parse_str::<LitInt>(&format!("{}usize", int.base10_digits()))
    {
        return quote! { #lit };
    }
    quote! { (#expr) as usize }
}
