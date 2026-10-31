// Derive macro for `#[derive(Validate)]` used by `armature-validation`.
//
// Generates an `armature_validation::Validate` implementation that runs the
// built-in validators against annotated fields and recurses into nested
// `#[validate]`-annotated struct fields.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, quote_spanned};
use syn::{
    Data, DeriveInput, Expr, Fields, GenericArgument, LitInt, LitStr, Path, PathArguments, Type,
    ext::IdentExt, parse_macro_input, spanned::Spanned,
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
    /// The pattern as compiled: anchored already for `regex(full = ...)`.
    Regex(LitStr),
    Required,
    /// A custom function, and whether it is also passed the field name.
    Custom {
        func: Expr,
        with_field: bool,
    },
    Nested,
}

/// Names the generated code binds. Created at the mixed-site span, so
/// neither user items nor user-written attribute expressions (a `custom`
/// closure, a `range` bound) can see or collide with them.
struct Locals {
    errors: syn::Ident,
    value: syn::Ident,
    error: syn::Ident,
}

impl Locals {
    fn new() -> Self {
        let ident = |name: &str| format_ident!("{}", name, span = Span::mixed_site());
        Self {
            errors: ident("__armature_errors"),
            value: ident("__armature_value"),
            error: ident("__armature_error"),
        }
    }
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
    let locals = Locals::new();

    let mut field_checks: Vec<TokenStream2> = Vec::new();

    for field in fields {
        let field_ident = field.ident.as_ref().expect("named field");
        // `r#type` is reported to clients as `type`.
        let field_name = field_ident.unraw().to_string();

        let mut checks: Vec<Check> = Vec::new();
        for attr in &field.attrs {
            if !attr.path().is_ident("validate") {
                continue;
            }

            // Bare `#[validate]` means "recurse into this nested struct".
            if matches!(attr.meta, syn::Meta::Path(_)) {
                checks.push(Check::Nested);
                continue;
            }
            let before = checks.len();
            attr.parse_nested_meta(|meta| {
                let Some(ident) = meta.path.get_ident().map(|i| i.to_string()) else {
                    let path = &meta.path;
                    return Err(meta.error(format!(
                        "unknown validate rule `{}`",
                        quote!(#path).to_string().replace(' ', "")
                    )));
                };

                match ident.as_str() {
                    "length" => {
                        let (min, max) = parse_min_max(&meta)?;
                        reject_inverted(&meta, &min, &max)?;
                        checks.push(Check::Length { min, max });
                        Ok(())
                    }
                    "range" => {
                        let (min, max) = parse_min_max(&meta)?;
                        reject_inverted(&meta, &min, &max)?;
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
                        checks.push(Check::Regex(parse_regex_pattern(&meta)?));
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
            if checks.len() == before {
                return Err(syn::Error::new(
                    attr.span(),
                    "empty `#[validate()]`: name at least one rule",
                ));
            }
        }

        if checks.is_empty() {
            continue;
        }
        let required = checks.iter().any(|c| matches!(c, Check::Required));
        let body: Vec<TokenStream2> = checks
            .into_iter()
            .map(|check| emit_check(&krate, &locals, &field_name, check))
            .collect();
        let value = &locals.value;
        // An `Option<T>` field is validated when present and skipped when
        // `None` (unless `required`); anything else is validated as is.
        // Either way the checks see `__armature_value: &T`.
        field_checks.push(if option_inner(&field.ty).is_some() {
            // `required` on an optional field also rejects `None`.
            let missing = required.then(|| {
                let errors = &locals.errors;
                quote! {
                    else {
                        #errors.push(
                            #krate::ValidationError::new(
                                #field_name,
                                ::std::format!("{} should not be empty", #field_name),
                            )
                            .with_constraint("notEmpty"),
                        );
                    }
                }
            });
            quote_spanned! {field.ty.span()=>
                if let ::std::option::Option::Some(#value) = &self.#field_ident {
                    #(#body)*
                } #missing
            }
        } else {
            quote_spanned! {field.ty.span()=>
                {
                    let #value = &self.#field_ident;
                    #(#body)*
                }
            }
        });
    }

    let errors = &locals.errors;
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #krate::Validate for #name #ty_generics #where_clause {
            fn validate(&self) -> ::std::result::Result<(), ::std::vec::Vec<#krate::ValidationError>> {
                let mut #errors: ::std::vec::Vec<#krate::ValidationError> = ::std::vec::Vec::new();
                #(#field_checks)*
                if #errors.is_empty() {
                    ::std::result::Result::Ok(())
                } else {
                    ::std::result::Result::Err(#errors)
                }
            }
        }
    })
}

/// `T` if `ty` is written as `Option<T>` (or a path ending in `Option<T>`).
fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let last = path.path.segments.last()?;
    if last.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(inner) if args.args.len() == 1 => Some(inner),
        _ => None,
    }
}

/// The path generated code uses to reach `armature-validation`.
///
/// An explicit `#[validate(crate = "path")]` on the struct wins. Otherwise
/// the deriving crate's manifest decides: `armature-validation` under
/// whatever name it was given (`av = { package = "armature-validation" }`),
/// or, failing that, the `armature-framework` facade's re-export (its library
/// is named `armature` unless renamed). A crate that has neither, or whose
/// manifest cannot be read, gets `::armature_validation`.
fn crate_path(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let mut explicit: Option<Path> = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("validate") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("crate") {
                if explicit.is_some() {
                    return Err(meta.error("duplicate `crate = ...`"));
                }
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

    use proc_macro_crate::crate_name;
    Ok(resolve_crate_path(
        crate_name("armature-validation").ok(),
        crate_name("armature-framework").ok(),
    ))
}

/// The path to `armature-validation`, given how the deriving crate's
/// manifest names `armature-validation` and the `armature-framework` facade.
fn resolve_crate_path(
    validation: Option<proc_macro_crate::FoundCrate>,
    framework: Option<proc_macro_crate::FoundCrate>,
) -> TokenStream2 {
    use proc_macro_crate::FoundCrate;
    match (validation, framework) {
        (Some(FoundCrate::Name(name)), _) => {
            let name = extern_ident(&name);
            quote! { ::#name }
        }
        // `Itself` is the library itself (and its unit tests, examples and
        // doctests); `armature-validation` declares
        // `extern crate self as armature_validation` for it.
        (Some(FoundCrate::Itself) | None, Some(FoundCrate::Name(name))) => {
            // Not renamed: the extern name is the library name, `armature`.
            if name == "armature_framework" {
                quote! { ::armature::armature_validation }
            } else {
                let name = extern_ident(&name);
                quote! { ::#name::armature_validation }
            }
        }
        _ => quote! { ::armature_validation },
    }
}

/// An extern crate name as an identifier, raw if it is a keyword
/// (`r#match = { package = "armature-validation" }`).
fn extern_ident(name: &str) -> syn::Ident {
    syn::parse_str::<syn::Ident>(name)
        .unwrap_or_else(|_| syn::Ident::new_raw(name, Span::call_site()))
}

/// Parse `min = <expr>, max = <expr>` (either optional, each at most once)
/// from a nested meta list.
fn parse_min_max(meta: &syn::meta::ParseNestedMeta) -> syn::Result<(Option<Expr>, Option<Expr>)> {
    let mut min = None;
    let mut max = None;
    meta.parse_nested_meta(|inner| {
        let slot = if inner.path.is_ident("min") {
            &mut min
        } else if inner.path.is_ident("max") {
            &mut max
        } else {
            return Err(inner.error("expected `min` or `max`"));
        };
        if slot.is_some() {
            return Err(inner.error("duplicate bound"));
        }
        *slot = Some(inner.value()?.parse()?);
        Ok(())
    })?;
    Ok((min, max))
}

/// Literal bounds with `min > max` admit no value: reject them at compile time
/// rather than emitting a rule that fails every input.
fn reject_inverted(
    meta: &syn::meta::ParseNestedMeta,
    min: &Option<Expr>,
    max: &Option<Expr>,
) -> syn::Result<()> {
    fn literal(expr: &Option<Expr>) -> Option<f64> {
        match expr.as_ref()? {
            Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(i),
                ..
            }) => i.base10_digits().parse().ok(),
            Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Float(f),
                ..
            }) => f.base10_digits().parse().ok(),
            Expr::Unary(syn::ExprUnary {
                op: syn::UnOp::Neg(_),
                expr,
                ..
            }) => literal(&Some((**expr).clone())).map(|v| -v),
            _ => None,
        }
    }
    match (literal(min), literal(max)) {
        (Some(lo), Some(hi)) if lo > hi => Err(meta.error("`min` is greater than `max`")),
        _ => Ok(()),
    }
}

/// Parse `regex = "..."`, `regex("...")`, `regex(pattern = "...")` or
/// `regex(full = "...")`, and compile the pattern now, at the pattern's
/// span: a typo, or a pattern too large to compile, should fail the build,
/// not every request at runtime.
///
/// `full` anchors the pattern to the whole value. The bare pattern is
/// compiled on its own first, so one with an unbalanced `)` cannot close the
/// anchoring group early.
fn parse_regex_pattern(meta: &syn::meta::ParseNestedMeta) -> syn::Result<LitStr> {
    let (lit, full): (LitStr, bool) = if meta.input.peek(syn::Token![=]) {
        (meta.value()?.parse()?, false)
    } else {
        let content;
        syn::parenthesized!(content in meta.input);
        let full = if content.peek(syn::Ident) {
            let key: syn::Ident = content.parse()?;
            let full = match key.to_string().as_str() {
                "pattern" => false,
                "full" => true,
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "expected `pattern = \"...\"` or `full = \"...\"`",
                    ));
                }
            };
            let _eq: syn::Token![=] = content.parse()?;
            full
        } else {
            false
        };
        (content.parse()?, full)
    };
    let bare = lit.value();
    let compiled = regex::Regex::new(&bare).and_then(|regex| {
        if full {
            regex::Regex::new(&anchored(&bare))
        } else {
            Ok(regex)
        }
    });
    if let Err(e) = compiled {
        return Err(syn::Error::new(
            lit.span(),
            format!("invalid regex pattern: {e}"),
        ));
    }
    Ok(if full {
        LitStr::new(&anchored(&bare), lit.span())
    } else {
        lit
    })
}

/// `pattern` anchored to the whole value. `\A`/`\z` mean the ends of the
/// value even under `(?m)`, unlike `^`/`$`.
fn anchored(pattern: &str) -> String {
    format!(r"\A(?:{pattern})\z")
}

fn emit_check(
    krate: &TokenStream2,
    locals: &Locals,
    field_name: &str,
    check: Check,
) -> TokenStream2 {
    let Locals {
        errors,
        value,
        error,
    } = locals;
    // Errors from the built-in validators are pushed without the submitted
    // value: errors reach clients, and some runtime versions attach it.
    let push_builtin = |result: TokenStream2| {
        quote! {
            if let ::std::result::Result::Err(mut #error) = #result {
                #error.value = ::std::option::Option::None;
                #errors.push(#error);
            }
        }
    };
    match check {
        Check::Length { min, max } => {
            let min_check = min.map(|m| {
                let m = as_usize(&m);
                push_builtin(quote! { #krate::MinLength(#m).validate(#value, #field_name) })
            });
            let max_check = max.map(|m| {
                let m = as_usize(&m);
                push_builtin(quote! { #krate::MaxLength(#m).validate(#value, #field_name) })
            });
            quote! { #min_check #max_check }
        }
        Check::Range { min, max } => {
            let lo = format_ident!("__armature_min", span = Span::mixed_site());
            let hi = format_ident!("__armature_max", span = Span::mixed_site());
            // Every bound is checked as "the comparison does not come out on
            // the right side", so an unordered value (a float NaN) fails
            // rather than slipping past `value < min`, which is false for NaN.
            let fails = |bound: &syn::Ident, pass: TokenStream2| {
                quote! {
                    !::core::matches!(
                        ::core::cmp::PartialOrd::partial_cmp(#value, &#bound),
                        ::core::option::Option::Some(#pass)
                    )
                }
            };
            let below = fails(
                &lo,
                quote! { ::core::cmp::Ordering::Greater | ::core::cmp::Ordering::Equal },
            );
            let above = fails(
                &hi,
                quote! { ::core::cmp::Ordering::Less | ::core::cmp::Ordering::Equal },
            );
            // Each bound expression is evaluated once, for both the check and
            // the message.
            let (bind, cond, message) = match (&min, &max) {
                (Some(min), Some(max)) => (
                    quote! { let #lo = #min; let #hi = #max; },
                    quote! { (#below) || (#above) },
                    quote! { ::std::format!("{} must be between {} and {}", #field_name, #lo, #hi) },
                ),
                (Some(min), None) => (
                    quote! { let #lo = #min; },
                    below,
                    quote! { ::std::format!("{} must be at least {}", #field_name, #lo) },
                ),
                (None, Some(max)) => (
                    quote! { let #hi = #max; },
                    above,
                    quote! { ::std::format!("{} must be at most {}", #field_name, #hi) },
                ),
                (None, None) => (
                    quote! {},
                    quote! { false },
                    quote! { ::std::format!("{} is out of range", #field_name) },
                ),
            };
            // The submitted value is not attached: errors reach clients.
            quote! {
                {
                    #bind
                    if #cond {
                        #errors.push(
                            #krate::ValidationError::new(#field_name, #message)
                                .with_constraint("range"),
                        );
                    }
                }
            }
        }
        Check::Email => push_builtin(quote! { #krate::IsEmail::validate(#value, #field_name) }),
        Check::Url => push_builtin(quote! { #krate::IsUrl::validate(#value, #field_name) }),
        // Compiled once per rule, on first use, rather than on every
        // `validate()`. The pattern compiled at expansion time, so this
        // cannot fail short of resource exhaustion; if it does, the rule
        // fails closed with a message that does not echo the pattern.
        Check::Regex(pattern) => {
            let regex = format_ident!("__ARMATURE_VALIDATE_REGEX", span = Span::mixed_site());
            let matcher = format_ident!("__armature_matcher", span = Span::mixed_site());
            let matched = push_builtin(quote! { #matcher.validate(#value, #field_name) });
            quote! {
                {
                    static #regex: ::std::sync::LazyLock<
                        ::std::option::Option<#krate::Matches>,
                    > = ::std::sync::LazyLock::new(|| #krate::Matches::new(#pattern).ok());
                    match &*#regex {
                        ::std::option::Option::Some(#matcher) => { #matched }
                        ::std::option::Option::None => {
                            #errors.push(
                                #krate::ValidationError::new(
                                    #field_name,
                                    ::std::format!("{} could not be validated", #field_name),
                                )
                                .with_constraint("matches"),
                            );
                        }
                    }
                }
            }
        }
        Check::Required => push_builtin(quote! { #krate::NotEmpty::validate(#value, #field_name) }),
        // A custom error with an empty field is attributed to this field, so
        // one function can serve several fields without naming any of them.
        // A value the function attached on purpose is kept.
        Check::Custom { func, with_field } => {
            let call = if with_field {
                quote! { (#func)(#value, #field_name) }
            } else {
                quote! { (#func)(#value) }
            };
            // The error type is named, so a function generic over it
            // (`fn f<E: From<ValidationError>>(..) -> Result<(), E>`) still
            // infers it as `ValidationError`.
            quote! {
                if let ::std::result::Result::Err(#error) = #call {
                    let mut #error: #krate::ValidationError = #error;
                    if #error.field.is_empty() {
                        #error.field = ::std::string::String::from(#field_name);
                    }
                    #errors.push(#error);
                }
            }
        }
        // Nested errors are prefixed with this field's name, so two fields of
        // the same type (`billing`, `shipping`) report distinct paths.
        Check::Nested => {
            let nested = format_ident!("__armature_nested", span = Span::mixed_site());
            quote! {
            if let ::std::result::Result::Err(#nested) =
                #krate::Validate::validate(#value)
            {
                #errors.extend(#nested.into_iter().map(|mut #error| {
                    #error.field = if #error.field.is_empty() {
                        ::std::string::String::from(#field_name)
                    } else {
                        ::std::format!("{}.{}", #field_name, #error.field)
                    };
                    #error
                }));
            }
            }
        }
    }
}

/// A `min`/`max` length bound as a `usize`. An unsuffixed literal becomes a
/// `usize` literal; anything else must already be a `usize`, rather than
/// being cast (`-1i64 as usize` is `usize::MAX`, which would lift the limit).
fn as_usize(expr: &Expr) -> TokenStream2 {
    if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(int),
        ..
    }) = expr
        && int.suffix().is_empty()
        && let Ok(lit) = syn::parse_str::<LitInt>(&format!("{}usize", int.base10_digits()))
    {
        return quote! { #lit };
    }
    quote_spanned! {expr.span()=> { let __armature_bound: usize = #expr; __armature_bound } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro_crate::FoundCrate;

    fn path(v: Option<FoundCrate>, f: Option<FoundCrate>) -> String {
        resolve_crate_path(v, f).to_string().replace(' ', "")
    }

    fn name(n: &str) -> Option<FoundCrate> {
        Some(FoundCrate::Name(n.to_string()))
    }

    #[test]
    fn direct_dependency_wins_under_its_own_name() {
        assert_eq!(
            path(name("armature_validation"), None),
            "::armature_validation"
        );
        assert_eq!(path(name("av"), name("armature_framework")), "::av");
    }

    #[test]
    fn facade_resolves_to_its_library_name_or_rename() {
        assert_eq!(
            path(None, name("armature_framework")),
            "::armature::armature_validation"
        );
        assert_eq!(path(None, name("fw")), "::fw::armature_validation");
        assert_eq!(
            path(Some(FoundCrate::Itself), name("armature_framework")),
            "::armature::armature_validation"
        );
    }

    #[test]
    fn falls_back_to_the_crate_name() {
        assert_eq!(path(None, None), "::armature_validation");
        assert_eq!(
            path(Some(FoundCrate::Itself), None),
            "::armature_validation"
        );
    }

    #[test]
    fn keyword_renames_become_raw_identifiers() {
        assert_eq!(path(name("match"), None), "::r#match");
    }
}
