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

/// A single validation check to emit for a field, with the span of the rule
/// in the attribute so type errors point at it.
struct Rule {
    check: Check,
    span: Span,
}

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
    /// Set when a field fails `length(max = ..)`; its scanning rules are then
    /// skipped rather than run over an oversized value.
    too_long: syn::Ident,
}

impl Locals {
    fn new() -> Self {
        let ident = |name: &str| format_ident!("{}", name, span = Span::mixed_site());
        Self {
            errors: ident("__armature_errors"),
            value: ident("__armature_value"),
            error: ident("__armature_error"),
            too_long: ident("__armature_too_long"),
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
    let mut needs_not_empty_helpers = false;
    let type_params: Vec<syn::Ident> = generics.type_params().map(|p| p.ident.clone()).collect();
    // Type and const parameters: either can make a type's trait impls unknown
    // inside the generic `validate()` (`C<N>` may be `AsRef<str>` for one `N`).
    let generic_params: Vec<syn::Ident> = type_params
        .iter()
        .cloned()
        .chain(generics.const_params().map(|p| p.ident.clone()))
        .collect();
    let mut needs_generic_helper = false;

    for field in fields {
        let field_ident = field.ident.as_ref().expect("named field");
        // `r#type` is reported to clients as `type`.
        let field_name = field_ident.unraw().to_string();

        let mut checks: Vec<Rule> = Vec::new();
        for attr in &field.attrs {
            if !attr.path().is_ident("validate") {
                continue;
            }

            // Bare `#[validate]` means "recurse into this nested struct".
            if matches!(attr.meta, syn::Meta::Path(_)) {
                checks.push(Rule {
                    check: Check::Nested,
                    span: attr.span(),
                });
                continue;
            }
            let before = checks.len();
            attr.parse_nested_meta(|meta| {
                let span = meta.path.span();
                let mut push = |check: Check| checks.push(Rule { check, span });
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
                        if min.is_none() && max.is_none() {
                            return Err(meta.error("`length` needs `min`, `max` or both"));
                        }
                        push(Check::Length { min, max });
                        Ok(())
                    }
                    "range" => {
                        let (min, max) = parse_min_max(&meta)?;
                        reject_inverted(&meta, &min, &max)?;
                        if min.is_none() && max.is_none() {
                            return Err(meta.error("`range` needs `min`, `max` or both"));
                        }
                        push(Check::Range { min, max });
                        Ok(())
                    }
                    "email" => {
                        push(Check::Email);
                        Ok(())
                    }
                    "url" => {
                        push(Check::Url);
                        Ok(())
                    }
                    "required" => {
                        push(Check::Required);
                        Ok(())
                    }
                    "nested" => {
                        push(Check::Nested);
                        Ok(())
                    }
                    "regex" => {
                        push(Check::Regex(parse_regex_pattern(&meta)?));
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
                        push(Check::Custom {
                            func,
                            with_field: ident == "custom_with_field",
                        });
                        Ok(())
                    }
                    other => Err(meta.error(format!("unknown validate rule `{}`", other))),
                }
            })?;
            if checks.len() == before {
                let span = match &attr.meta {
                    syn::Meta::List(list) => list.delimiter.span().join(),
                    meta => meta.span(),
                };
                return Err(syn::Error::new(
                    span,
                    "empty `#[validate()]`: name at least one rule",
                ));
            }
        }

        if checks.is_empty() {
            continue;
        }
        // A rule repeated on one field only duplicates its errors, and a
        // repeated `length`/`range` hides which bounds apply. `regex` and
        // `custom` may legitimately appear more than once.
        let mut seen = std::collections::HashSet::new();
        for rule in &checks {
            let kind = match &rule.check {
                Check::Length { .. } => "length",
                Check::Range { .. } => "range",
                Check::Email => "email",
                Check::Url => "url",
                Check::Required => "required",
                Check::Nested => "nested",
                Check::Regex(_) | Check::Custom { .. } => continue,
            };
            if !seen.insert(kind) {
                return Err(syn::Error::new(
                    rule.span,
                    format!("duplicate `{kind}` rule on this field"),
                ));
            }
        }

        let inner = option_inner(&field.ty);
        let required = checks.iter().any(|r| matches!(r.check, Check::Required));
        // `required` on `Option<Option<T>>` could mean either layer; rather
        // than let the inner `None` pass as "present", reject it.
        if required && inner.and_then(option_inner).is_some() {
            return Err(syn::Error::new(
                field.ty.span(),
                "`required` on `Option<Option<_>>` is ambiguous; use `custom`",
            ));
        }
        let optional = match inner {
            None => Optional::No,
            Some(inner) => classify_optional(inner, &type_params, &generic_params),
        };
        needs_not_empty_helpers |= required && optional == Optional::Concrete;
        needs_generic_helper |= required && matches!(optional, Optional::Generic(_));
        let access = quote! { self.#field_ident };
        let (whole, mut unwrapped): (Vec<Rule>, Vec<Rule>) = checks
            .into_iter()
            .partition(|r| matches!(r.check, Check::Custom { .. }));
        // `length` first, so a value over its maximum is not then scanned by
        // `email`, `url` and `regex`.
        unwrapped.sort_by_key(|r| !matches!(r.check, Check::Length { .. }));
        let unwrapped_empty = unwrapped.is_empty();
        let emit = |rules: Vec<Rule>| -> Vec<TokenStream2> {
            rules
                .into_iter()
                .map(|rule| emit_check(&krate, &locals, &field_name, rule, optional))
                .collect()
        };
        let (whole, body) = (emit(whole), emit(unwrapped));
        let too_long = &locals.too_long;
        let body = quote! {
            let #too_long = false;
            #(#body)*
        };
        let value = &locals.value;
        // The built-in rules on an `Option<T>` field see the value when it is
        // `Some` and are skipped for `None` (unless `required`); on any other
        // field they see it as is. Either way they get `__armature_value: &T`.
        // `custom` rules always get the whole field, `Option` included, so a
        // custom function can still act on `None`.
        let custom = quote! {
            {
                let #value = &#access;
                #(#whole)*
            }
        };
        if !whole.is_empty() {
            field_checks.push(custom);
        }
        if unwrapped_empty && !required {
            continue;
        }
        field_checks.push(if inner.is_some() {
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
            quote! {
                if let ::std::option::Option::Some(#value) = &#access {
                    #body
                } #missing
            }
        } else {
            quote! {
                {
                    let #value = &#access;
                    #body
                }
            }
        });
    }

    let errors = &locals.errors;
    let helpers = needs_not_empty_helpers.then(|| not_empty_helpers(&krate));
    let generic_helper = needs_generic_helper.then(generic_required_helper);
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #krate::Validate for #name #ty_generics #where_clause {
            fn validate(&self) -> ::std::result::Result<(), ::std::vec::Vec<#krate::ValidationError>> {
                #helpers
                #generic_helper
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
/// Detection is syntactic: a type alias for `Option` is not seen through.
fn option_inner(ty: &Type) -> Option<&Type> {
    let ty = match ty {
        Type::Group(g) => return option_inner(&g.elem),
        Type::Paren(p) => return option_inner(&p.elem),
        ty => ty,
    };
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
        // The crate itself, before any facade: a facade would be a second
        // copy of it.
        (Some(FoundCrate::Itself), _) => quote! { ::armature_validation },
        (None, Some(FoundCrate::Name(name))) => {
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

/// An extern crate name as a path segment: raw if it is a keyword
/// (`r#match = { package = "armature-validation" }`). `self`, `super`,
/// `crate` and `Self` cannot be raw, and `_` cannot name a crate; those fall
/// back to the crate's real name, which the user can override with
/// `#[validate(crate = "...")]`.
fn extern_ident(name: &str) -> TokenStream2 {
    if let Ok(ident) = syn::parse_str::<syn::Ident>(name) {
        return quote! { #ident };
    }
    if matches!(name, "self" | "super" | "crate" | "Self" | "_") {
        return quote! { armature_validation };
    }
    let ident = syn::Ident::new_raw(name, Span::call_site());
    quote! { #ident }
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
    // Integers compare exactly as `i128` (an `f64` loses precision above
    // 2^53); floats as `f64`.
    #[derive(PartialEq, PartialOrd)]
    enum Num {
        Int(i128),
        Float(f64),
    }
    impl std::ops::Neg for Num {
        type Output = Num;
        fn neg(self) -> Num {
            match self {
                Num::Int(i) => Num::Int(-i),
                Num::Float(f) => Num::Float(-f),
            }
        }
    }
    fn literal(expr: &Option<Expr>) -> Option<Num> {
        match expr.as_ref()? {
            Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(i),
                ..
            }) => i.base10_parse::<i128>().ok().map(Num::Int),
            Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Float(f),
                ..
            }) => f.base10_parse::<f64>().ok().map(Num::Float),
            Expr::Unary(syn::ExprUnary {
                op: syn::UnOp::Neg(_),
                expr,
                ..
            }) => literal(&Some((**expr).clone())).map(std::ops::Neg::neg),
            _ => None,
        }
    }
    match (literal(min), literal(max)) {
        (Some(Num::Int(lo)), Some(Num::Int(hi))) if lo > hi => {
            Err(meta.error("`min` is greater than `max`"))
        }
        (Some(Num::Float(lo)), Some(Num::Float(hi))) if lo > hi => {
            Err(meta.error("`min` is greater than `max`"))
        }
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
    let invalid = |e: &dyn std::fmt::Display| {
        syn::Error::new(lit.span(), format!("invalid regex pattern: {e}"))
    };
    if full {
        // Parsing the bare pattern is enough to reject one that would close
        // the anchoring group early; only the anchored form is compiled.
        regex_syntax::Parser::new()
            .parse(&bare)
            .map_err(|e| invalid(&e))?;
        regex::Regex::new(&anchored(&bare)).map_err(|e| invalid(&e))?;
    } else {
        regex::Regex::new(&bare).map_err(|e| invalid(&e))?;
    }
    Ok(if full {
        LitStr::new(&anchored(&bare), lit.span())
    } else {
        lit
    })
}

/// `pattern` anchored to the whole value. `\A`/`\z` mean the ends of the
/// value even under `(?m)`, unlike `^`/`$`. A verbose-mode (`(?x)`) pattern
/// ending in a `#` comment would swallow the closing `)` and is rejected; end
/// it with a newline.
fn anchored(pattern: &str) -> String {
    format!(r"\A(?:{pattern})\z")
}

/// Whether a field is an `Option`, and if so how `required` checks its inner
/// type: through specialisation (`Concrete`), or, for a generic inner type
/// that might be a string, by requiring `AsRef<str>` (`Generic`). See
/// `classify_optional`.
#[derive(Clone, Copy, PartialEq)]
enum Optional {
    No,
    Concrete,
    /// Requires `AsRef<str>` of the inner type's core, under this many
    /// `&`/`Box`/`Arc`/`Rc`/`Cow` layers (0: of the whole inner type).
    Generic(usize),
}

/// How `required` treats an `Option` whose inner type is `ty`, in a struct
/// with type parameters `params`. Specialisation cannot see through a type
/// parameter, so a generic inner type must not reach it unless it can never
/// be a string:
///
/// - a parameter, an associated type of one, or (in a generic struct) a type
///   macro, behind any number of `&`, `Box`, `Arc`, `Rc` or `Cow` layers:
///   `Generic(layers)`, which requires the core type to be `AsRef<str>`;
/// - a container that is never a string (`Vec`, maps, sets, `Option`,
///   tuples, arrays, slices): `Concrete`, i.e. specialisation (→ "is `Some`");
/// - any other type that mentions a parameter (`MyWrapper<T>`,
///   `Pin<Box<T>>`): `Generic(0)`, which requires the whole type to be
///   `AsRef<str>` — a compile error unless it is, rather than a silent
///   "is `Some`";
/// - anything else: `Concrete`.
fn classify_optional(
    ty: &Type,
    type_params: &[syn::Ident],
    generic_params: &[syn::Ident],
) -> Optional {
    if let Some(layers) = generic_string_core(ty, type_params) {
        return Optional::Generic(layers);
    }
    if !mentions_type_param(ty, generic_params) || never_a_string(peel_pointers(ty)) {
        return Optional::Concrete;
    }
    Optional::Generic(0)
}

/// `ty` without its outer `&`/`Box`/`Arc`/`Rc`/`Cow` layers, so a container
/// behind a pointer (`&[T]`, `Box<[T]>`, `Arc<Vec<T>>`) is still recognised.
fn peel_pointers(ty: &Type) -> &Type {
    match ty {
        Type::Group(g) => peel_pointers(&g.elem),
        Type::Paren(p) => peel_pointers(&p.elem),
        Type::Reference(r) => peel_pointers(&r.elem),
        Type::Path(p) if p.qself.is_none() => {
            let Some(last) = p.path.segments.last() else {
                return ty;
            };
            if !matches!(
                last.ident.to_string().as_str(),
                "Box" | "Arc" | "Rc" | "Cow"
            ) {
                return ty;
            }
            let PathArguments::AngleBracketed(args) = &last.arguments else {
                return ty;
            };
            let mut types = args.args.iter().filter_map(|a| match a {
                GenericArgument::Type(t) => Some(t),
                _ => None,
            });
            match (types.next(), types.next()) {
                (Some(inner), None) => peel_pointers(inner),
                _ => ty,
            }
        }
        _ => ty,
    }
}

/// The trait the generic `required` branch calls through, so a missing
/// `AsRef<str>` is reported with a message naming the rule and the way out.
fn generic_required_helper() -> TokenStream2 {
    quote! {
        #[diagnostic::on_unimplemented(
            message = "`#[validate(required)]` on an `Option` of `{Self}` needs `{Self}: AsRef<str>`",
            label = "the derive cannot tell whether this generic type is a string",
            note = "bound the type parameter by `AsRef<str>`, or use `custom` for a presence-only check"
        )]
        trait __ArmatureRequiredStr {
            fn __armature_str(&self) -> &str;
        }
        impl<T: ?::core::marker::Sized + ::core::convert::AsRef<str>> __ArmatureRequiredStr for T {
            fn __armature_str(&self) -> &str {
                self.as_ref()
            }
        }
    }
}

/// Layers of `&`/`Box`/`Arc`/`Rc`/`Cow` over a type-parameter core, if any.
fn generic_string_core(ty: &Type, params: &[syn::Ident]) -> Option<usize> {
    match ty {
        Type::Group(g) => generic_string_core(&g.elem, params),
        Type::Paren(p) => generic_string_core(&p.elem, params),
        Type::Macro(_) => (!params.is_empty()).then_some(0),
        Type::Reference(r) => generic_string_core(&r.elem, params).map(|n| n + 1),
        Type::Path(p) => {
            if p.qself.is_some() {
                return mentions_type_param(ty, params).then_some(0);
            }
            let first = p.path.segments.first()?;
            if p.path.leading_colon.is_none() && params.contains(&first.ident) {
                return Some(0);
            }
            let last = p.path.segments.last()?;
            if !matches!(
                last.ident.to_string().as_str(),
                "Box" | "Arc" | "Rc" | "Cow"
            ) {
                return None;
            }
            let PathArguments::AngleBracketed(args) = &last.arguments else {
                return None;
            };
            // The pointee: `Box<T>`, `Arc<T>`, `Rc<T>`, `Cow<'a, T>` (skip the
            // lifetime). An allocator argument (`Box<T, A>`) is not unwrapped.
            let types: Vec<&Type> = args
                .args
                .iter()
                .filter_map(|a| match a {
                    GenericArgument::Type(t) => Some(t),
                    _ => None,
                })
                .collect();
            match types.as_slice() {
                [inner] => generic_string_core(inner, params).map(|n| n + 1),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Containers that are never a string whatever their parameters.
fn never_a_string(ty: &Type) -> bool {
    match ty {
        Type::Group(g) => never_a_string(&g.elem),
        Type::Paren(p) => never_a_string(&p.elem),
        Type::Tuple(_) | Type::Array(_) | Type::Slice(_) => true,
        Type::Path(p) if p.qself.is_none() => p.path.segments.last().is_some_and(|s| {
            matches!(
                s.ident.to_string().as_str(),
                "Vec"
                    | "VecDeque"
                    | "LinkedList"
                    | "BinaryHeap"
                    | "HashMap"
                    | "BTreeMap"
                    | "HashSet"
                    | "BTreeSet"
                    | "Option"
                    | "Result"
            )
        }),
        _ => false,
    }
}

/// Whether `ty` mentions any of `params` (the struct's type parameters).
fn mentions_type_param(ty: &Type, params: &[syn::Ident]) -> bool {
    struct Finder<'a> {
        params: &'a [syn::Ident],
        found: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for Finder<'_> {
        // A macro's tokens are opaque, so in a generic struct assume it may
        // name a parameter (failing closed) rather than that it does not.
        fn visit_macro(&mut self, _: &'ast syn::Macro) {
            if !self.params.is_empty() {
                self.found = true;
            }
        }
        fn visit_path(&mut self, path: &'ast syn::Path) {
            if path.leading_colon.is_none()
                && let Some(first) = path.segments.first()
                && self.params.contains(&first.ident)
            {
                self.found = true;
            }
            syn::visit::visit_path(self, path);
        }
    }
    let mut finder = Finder {
        params,
        found: false,
    };
    syn::visit::Visit::visit_type(&mut finder, ty);
    finder.found
}

/// Helpers for `required` on an `Option<T>` field, emitted once at the top of
/// `validate()` when needed. Autoref specialisation picks the first that
/// applies:
///
/// 1. `T: AsRef<str>` (`String`, `&str`, `Box<str>`, `Arc<str>`, `Cow<str>`,
///    an `AsRef<str>` newtype): emptiness check;
/// 2. – 4. one, two or three `Deref` steps to an `AsRef<str>` type
///    (`Arc<String>`, `&Arc<String>`, `Rc<DerefToStr>`,
///    `&Rc<Box<String>>`, ...): emptiness check through the derefs, as deref
///    coercion would do for the same type outside an `Option` (for up to
///    three steps; a deeper chain gets only the `Some` check);
/// 5. otherwise (a number, a `Uuid`, a nested struct): only the `Some`
///    check.
///
/// Method lookup on `&&&&W` tries `Self` = `&&&W`, `&&&&W` (autoref), `&&W`,
/// `&W`, then `W` (each further deref), so the five impls sit there in order.
fn not_empty_helpers(krate: &TokenStream2) -> TokenStream2 {
    let ret = quote! { ::std::option::Option<#krate::ValidationError> };
    let sized = quote! { ?::core::marker::Sized };
    let deref = quote! { ::core::ops::Deref };
    let as_str = quote! { ::core::convert::AsRef<str> };
    let check = |expr: TokenStream2| {
        quote! {
            fn __armature_check(&self, field: &str) -> #ret {
                #krate::NotEmpty::validate(::core::convert::AsRef::<str>::as_ref(#expr), field).err()
            }
        }
    };
    let level0 = check(quote! { self.0 });
    let level1 = check(quote! { &**self.0 });
    let level2 = check(quote! { &***self.0 });
    let level3 = check(quote! { &****self.0 });
    quote! {
        struct __ArmatureNotEmpty<'a, T: #sized>(&'a T);
        trait __ArmatureNotEmpty0 { fn __armature_check(&self, field: &str) -> #ret; }
        trait __ArmatureNotEmpty1 { fn __armature_check(&self, field: &str) -> #ret; }
        trait __ArmatureNotEmpty2 { fn __armature_check(&self, field: &str) -> #ret; }
        trait __ArmatureNotEmpty3 { fn __armature_check(&self, field: &str) -> #ret; }
        trait __ArmatureNotEmptyAny {
            fn __armature_check(&self, _field: &str) -> #ret {
                ::std::option::Option::None
            }
        }
        impl<T: #sized + #as_str> __ArmatureNotEmpty0 for &&&__ArmatureNotEmpty<'_, T> {
            #level0
        }
        impl<T> __ArmatureNotEmpty1 for &&&&__ArmatureNotEmpty<'_, T>
        where
            T: #sized + #deref,
            T::Target: #as_str,
        {
            #level1
        }
        impl<T> __ArmatureNotEmpty2 for &&__ArmatureNotEmpty<'_, T>
        where
            T: #sized + #deref,
            T::Target: #deref,
            <T::Target as #deref>::Target: #as_str,
        {
            #level2
        }
        impl<T> __ArmatureNotEmpty3 for &__ArmatureNotEmpty<'_, T>
        where
            T: #sized + #deref,
            T::Target: #deref,
            <T::Target as #deref>::Target: #deref,
            <<T::Target as #deref>::Target as #deref>::Target: #as_str,
        {
            #level3
        }
        impl<T: #sized> __ArmatureNotEmptyAny for __ArmatureNotEmpty<'_, T> {}
    }
}

fn emit_check(
    krate: &TokenStream2,
    locals: &Locals,
    field_name: &str,
    rule: Rule,
    optional: Optional,
) -> TokenStream2 {
    let Rule { check, span } = rule;
    let Locals {
        errors,
        value,
        error,
        too_long,
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
    let unless_too_long = |check: TokenStream2| quote! { if !#too_long { #check } };
    // The value binding, located at the rule: same hygiene, so it names the
    // same local, but a type error (`email` on a `u32`) points at the rule
    // rather than at `#[derive(Validate)]`.
    let at_rule = syn::Ident::new(&value.to_string(), Span::mixed_site().located_at(span));
    match check {
        Check::Length { min, max } => {
            let min_check = min.map(|m| {
                let m = as_usize(&m);
                push_builtin(
                    quote_spanned! {span=> #krate::MinLength(#m).validate(#at_rule, #field_name) },
                )
            });
            // A failed maximum also marks the field, so the scanning rules
            // after it are skipped.
            let max_check = max.map(|m| {
                let m = as_usize(&m);
                let call =
                    quote_spanned! {span=> #krate::MaxLength(#m).validate(#at_rule, #field_name) };
                // Shadows the flag rather than assigning it, so the generated
                // code has no `let mut` + `if` for user lints to flag.
                quote! {
                    let #too_long = if let ::std::result::Result::Err(mut #error) = #call {
                        #error.value = ::std::option::Option::None;
                        #errors.push(#error);
                        true
                    } else {
                        false
                    };
                }
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
                quote_spanned! {span=>
                    !::core::matches!(
                        ::core::cmp::PartialOrd::partial_cmp(#at_rule, #bound),
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
                    quote! { let #lo = &(#min); let #hi = &(#max); },
                    quote! { (#below) || (#above) },
                    quote! { ::std::format!("{} must be between {} and {}", #field_name, #lo, #hi) },
                ),
                (Some(min), None) => (
                    quote! { let #lo = &(#min); },
                    below,
                    quote! { ::std::format!("{} must be at least {}", #field_name, #lo) },
                ),
                (None, Some(max)) => (
                    quote! { let #hi = &(#max); },
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
        Check::Email => unless_too_long(push_builtin(
            quote_spanned! {span=> #krate::IsEmail::validate(#at_rule, #field_name) },
        )),
        Check::Url => unless_too_long(push_builtin(
            quote_spanned! {span=> #krate::IsUrl::validate(#at_rule, #field_name) },
        )),
        // Compiled once per rule, on first use, rather than on every
        // `validate()`. The pattern compiled at expansion time, so this
        // cannot fail short of resource exhaustion; if it does, the rule
        // fails closed with a message that does not echo the pattern.
        Check::Regex(pattern) => {
            let regex = format_ident!("__ARMATURE_VALIDATE_REGEX", span = Span::mixed_site());
            let matcher = format_ident!("__armature_matcher", span = Span::mixed_site());
            let matched =
                push_builtin(quote_spanned! {span=> #matcher.validate(#at_rule, #field_name) });
            unless_too_long(quote! {
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
            })
        }
        // Autoref specialisation cannot see through a type parameter: inside
        // the generic `validate()` it would always pick the "is `Some`"
        // fallback, even for `S<String>`. So a generic inner type that might
        // be a string requires `AsRef<str>` (of its core, under any
        // `&`/`Box`/`Arc`/`Rc`/`Cow` layers, or of the whole type); without
        // that bound it fails to compile rather than silently skipping the
        // emptiness check. See `classify_optional`.
        Check::Required if matches!(optional, Optional::Generic(_)) => {
            let Optional::Generic(layers) = optional else {
                unreachable!()
            };
            let mut inner = quote! { #at_rule };
            for _ in 0..layers {
                inner = quote! { &**(#inner) };
            }
            push_builtin(quote_spanned! {span=>
                #krate::NotEmpty::validate(__ArmatureRequiredStr::__armature_str(#inner), #field_name)
            })
        }
        // On an `Option` field (this branch runs for `Some`), the emptiness
        // check is chosen by the inner type; see `not_empty_helpers`.
        Check::Required if optional == Optional::Concrete => quote! {
            if let ::std::option::Option::Some(mut #error) =
                (&&&&__ArmatureNotEmpty(#value)).__armature_check(#field_name)
            {
                #error.value = ::std::option::Option::None;
                #errors.push(#error);
            }
        },
        Check::Required => {
            push_builtin(quote_spanned! {span=> #krate::NotEmpty::validate(#at_rule, #field_name) })
        }
        // A custom error with an empty field is attributed to this field, so
        // one function can serve several fields without naming any of them.
        // Its value is cleared like a built-in rule's: a custom function that
        // calls a built-in validator would otherwise pass the value on.
        Check::Custom { func, with_field } => {
            // The call itself stays at the macro's span (so lints such as
            // `redundant_closure_call` treat it as generated code, which an
            // inline `custom = "|v| ..."` closure is); only the argument is
            // located at the rule, which is where a signature mismatch shows.
            let call = if with_field {
                quote! { (#func)(#at_rule, #field_name) }
            } else {
                quote! { (#func)(#at_rule) }
            };
            // The error type is named, so a function generic over it
            // (`fn f<E: From<ValidationError>>(..) -> Result<(), E>`) still
            // infers it as `ValidationError`.
            quote! {
                if let ::std::result::Result::Err(#error) = #call {
                    let mut #error: #krate::ValidationError = #error;
                    #error.value = ::std::option::Option::None;
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
                #krate::Validate::validate(#at_rule)
            {
                #errors.extend(#nested.into_iter().map(|mut #error| {
                    // A hand-written nested impl may have attached it.
                    #error.value = ::std::option::Option::None;
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
    {
        // Built at the literal's span, so `overflowing_literals` still
        // rejects a value that does not fit.
        let lit = LitInt::new(&format!("{}usize", int.base10_digits()), int.span());
        return quote! { #lit };
    }
    let bound = format_ident!("__armature_bound", span = Span::mixed_site());
    quote_spanned! {expr.span()=>
        { let #bound: ::core::primitive::usize = #expr; #bound }
    }
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
            "::armature_validation"
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
