//! Behaviour of `#[derive(Validate)]`, against the published
//! `armature-validation` runtime.
//!
//! Strict lints, so CI's clippy run also covers the generated code.
#![deny(clippy::pedantic, clippy::nursery)]
#![forbid(unused_variables)]
// Lints on the tests' own code, not the generated code.
#![allow(
    clippy::missing_errors_doc,
    clippy::unnecessary_wraps,
    clippy::items_after_statements,
    // Field types chosen to exercise `required` on string-like smart pointers.
    clippy::box_collection,
    clippy::redundant_allocation,
    clippy::struct_field_names,
    // `custom` validators take `&Field` by the derive's contract.
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref
)]

use armature_proc_macro::Validate;
use armature_validation::{Validate as _, ValidationError};

fn fields(errors: &[ValidationError]) -> Vec<&str> {
    errors.iter().map(|e| e.field.as_str()).collect()
}

/// A one-sided bound must reject NaN: `NaN < min` is false, so a plain
/// comparison would let it through.
#[test]
fn one_sided_range_rejects_nan() {
    #[derive(Validate)]
    struct Amounts {
        #[validate(range(min = 0.0))]
        at_least: f64,
        #[validate(range(max = 100.0))]
        at_most: f64,
        #[validate(range(min = 0.0, max = 1.0))]
        between: f64,
    }

    let nan = Amounts {
        at_least: f64::NAN,
        at_most: f64::NAN,
        between: f64::NAN,
    };
    let errors = nan.validate().unwrap_err();
    assert_eq!(fields(&errors), ["at_least", "at_most", "between"]);

    let ok = Amounts {
        at_least: 0.0,
        at_most: 100.0,
        between: 0.5,
    };
    assert!(ok.validate().is_ok());
}

/// Errors reach clients, so a rejected value must not be attached to them.
#[test]
fn range_errors_do_not_carry_the_value() {
    #[derive(Validate)]
    struct Age {
        #[validate(range(min = 18, max = 120))]
        age: u32,
    }
    let errors = Age { age: 7777 }.validate().unwrap_err();
    assert!(errors[0].value.is_none());
    assert!(!errors[0].message.contains("7777"));
}

#[test]
fn regex_rule_matches_and_works_in_generic_structs() {
    #[derive(Validate)]
    struct Tagged<T> {
        #[validate(regex = r"^[a-z]{3}$")]
        tag: String,
        _extra: T,
    }
    let ok = Tagged {
        tag: "abc".to_string(),
        _extra: 1u8,
    };
    assert!(ok.validate().is_ok());
    let bad = Tagged {
        tag: "abcd".to_string(),
        _extra: "x",
    };
    assert_eq!(fields(&bad.validate().unwrap_err()), ["tag"]);
}

#[test]
fn nested_errors_are_prefixed_with_the_parent_field() {
    #[derive(Validate)]
    struct Address {
        #[validate(required)]
        city: String,
    }
    #[derive(Validate)]
    struct Order {
        #[validate]
        billing: Address,
        #[validate(nested)]
        shipping: Address,
    }
    let order = Order {
        billing: Address {
            city: String::new(),
        },
        shipping: Address {
            city: String::new(),
        },
    };
    assert_eq!(
        fields(&order.validate().unwrap_err()),
        ["billing.city", "shipping.city"]
    );
}

fn no_spaces(value: &str) -> Result<(), ValidationError> {
    if value.contains(' ') {
        Err(ValidationError::new("", "must not contain spaces"))
    } else {
        Ok(())
    }
}

fn no_spaces_named(value: &str, field: &str) -> Result<(), ValidationError> {
    if value.contains(' ') {
        Err(ValidationError::new(
            field,
            format!("{field} must not contain spaces"),
        ))
    } else {
        Ok(())
    }
}

#[test]
fn custom_validators_learn_the_field_name() {
    #[derive(Validate)]
    struct Names {
        #[validate(custom = "no_spaces")]
        first: String,
        #[validate(custom_with_field = "no_spaces_named")]
        last: String,
    }
    let errors = Names {
        first: "a b".to_string(),
        last: "c d".to_string(),
    }
    .validate()
    .unwrap_err();
    assert_eq!(fields(&errors), ["first", "last"]);
    assert_eq!(errors[1].message, "last must not contain spaces");
}

/// A crate reaching `armature-validation` through another path names it.
mod reexport {
    pub use armature_validation::*;
}

#[test]
fn crate_attribute_overrides_the_path() {
    #[derive(Validate)]
    #[validate(crate = "crate::reexport")]
    struct Login {
        #[validate(length(min = 8))]
        password: String,
    }
    assert!(
        reexport::Validate::validate(&Login {
            password: "short".to_string()
        })
        .is_err()
    );
}

/// No built-in rule echoes the submitted value, whatever runtime version is
/// underneath (0.7.0's validators attach it).
#[test]
fn builtin_rule_errors_do_not_carry_the_value() {
    #[derive(Validate)]
    struct Signup {
        #[validate(length(min = 12))]
        password: String,
        #[validate(email)]
        email: String,
        #[validate(url)]
        website: String,
        #[validate(regex = r"^\d{3}$")]
        code: String,
        #[validate(required)]
        name: String,
    }
    let errors = Signup {
        password: "hunter2".into(),
        email: "secret-token".into(),
        website: "ssn-123".into(),
        code: "x1".into(),
        name: " ".into(),
    }
    .validate()
    .unwrap_err();
    assert_eq!(errors.len(), 5);
    assert!(errors.iter().all(|e| e.value.is_none()), "{errors:?}");
}

#[test]
fn regex_full_anchors_the_whole_value() {
    #[derive(Validate)]
    struct Handle {
        #[validate(regex(full = "[a-z]+"))]
        handle: String,
        #[validate(regex(full = "(?m)^[a-z]+$"))]
        multiline: String,
    }
    let ok = Handle {
        handle: "abc".into(),
        multiline: "abc".into(),
    };
    assert!(ok.validate().is_ok());
    let bad = Handle {
        handle: "1<script>a".into(),
        multiline: "abc\n<script>".into(),
    };
    assert_eq!(
        fields(&bad.validate().unwrap_err()),
        ["handle", "multiline"]
    );
}

#[test]
fn option_fields_are_validated_when_present() {
    #[derive(Validate)]
    struct Profile {
        #[validate(length(max = 3))]
        nickname: Option<String>,
        #[validate(range(min = 18))]
        age: Option<u32>,
        #[validate(required)]
        bio: Option<String>,
    }
    let absent = Profile {
        nickname: None,
        age: None,
        bio: None,
    };
    assert_eq!(fields(&absent.validate().unwrap_err()), ["bio"]);
    let bad = Profile {
        nickname: Some("long".into()),
        age: Some(7),
        bio: Some("x".into()),
    };
    assert_eq!(fields(&bad.validate().unwrap_err()), ["nickname", "age"]);
}

#[test]
fn range_bounds_are_evaluated_once_and_compared_by_reference() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn lo() -> u64 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        10
    }
    #[derive(Validate)]
    struct Order {
        #[validate(range(min = lo()))]
        qty: u64,
        // Not `Copy`: compared through a reference, not moved out of `self`.
        #[validate(range(max = "m".to_string()))]
        code: String,
    }
    let errors = Order {
        qty: 1,
        code: "z".into(),
    }
    .validate()
    .unwrap_err();
    assert_eq!(fields(&errors), ["qty", "code"]);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

/// A custom validator generic over its error type still infers it.
fn generic_custom<E: From<ValidationError>>(value: &str) -> Result<(), E> {
    if value.is_empty() {
        Err(ValidationError::new("", "missing").into())
    } else {
        Ok(())
    }
}

#[test]
fn custom_validators_generic_in_the_error_type_compile() {
    #[derive(Validate)]
    struct S {
        #[validate(custom = "generic_custom")]
        v: String,
    }
    assert_eq!(
        fields(&S { v: String::new() }.validate().unwrap_err()),
        ["v"]
    );
}

#[test]
fn raw_identifier_fields_are_reported_unraw() {
    #[derive(Validate)]
    struct S {
        #[validate(required)]
        r#type: String,
    }
    let errors = S {
        r#type: String::new(),
    }
    .validate()
    .unwrap_err();
    assert_eq!(fields(&errors), ["type"]);
    assert!(errors[0].message.starts_with("type "));
}

fn must_be_some(v: &Option<u32>) -> Result<(), ValidationError> {
    if v.is_some() {
        Ok(())
    } else {
        Err(ValidationError::new("", "is required"))
    }
}

/// `custom` gets the whole field, `Option` included, so it still runs on
/// `None`; `required` on an `Option` of a non-string type means "is `Some`".
#[test]
fn option_fields_keep_custom_and_required_semantics() {
    #[derive(Validate)]
    struct Prefs {
        #[validate(custom = "must_be_some")]
        a: Option<u32>,
        #[validate(required, range(max = 10))]
        b: Option<u32>,
    }
    let none = Prefs { a: None, b: None };
    assert_eq!(fields(&none.validate().unwrap_err()), ["a", "b"]);
    let big = Prefs {
        a: Some(1),
        b: Some(11),
    };
    assert_eq!(fields(&big.validate().unwrap_err()), ["b"]);
    let ok = Prefs {
        a: Some(1),
        b: Some(3),
    };
    assert!(ok.validate().is_ok());
}

#[test]
fn optional_nested_structs_are_validated_when_present() {
    #[derive(Validate)]
    struct Address {
        #[validate(required)]
        city: String,
    }
    #[derive(Validate)]
    struct Order {
        #[validate]
        shipping: Option<Address>,
    }
    assert!(Order { shipping: None }.validate().is_ok());
    let bad = Order {
        shipping: Some(Address {
            city: String::new(),
        }),
    };
    assert_eq!(fields(&bad.validate().unwrap_err()), ["shipping.city"]);
}

/// A hand-written nested impl that attaches a value has it cleared too.
#[test]
fn nested_errors_from_hand_written_impls_lose_their_value() {
    struct Inner(String);
    impl armature_validation::Validate for Inner {
        fn validate(&self) -> Result<(), Vec<ValidationError>> {
            Err(vec![
                ValidationError::new("name", "too short").with_value(self.0.clone()),
            ])
        }
    }
    #[derive(Validate)]
    struct Outer {
        #[validate]
        inner: Inner,
    }
    let errors = Outer {
        inner: Inner("s3cr3t".into()),
    }
    .validate()
    .unwrap_err();
    assert_eq!(fields(&errors), ["inner.name"]);
    assert!(errors[0].value.is_none());
}

/// A bound from another, non-`Copy` field is compared by reference.
#[test]
fn range_bounds_may_borrow_other_fields() {
    #[derive(Validate)]
    struct Window {
        #[validate(range(min = self.floor))]
        name: String,
        floor: String,
    }
    let w = Window {
        name: "a".into(),
        floor: "b".into(),
    };
    assert_eq!(fields(&w.validate().unwrap_err()), ["name"]);
}

/// A value over its maximum length is not then scanned by the other rules,
/// and a custom validator calling a built-in one does not echo the value.
#[test]
fn oversized_values_skip_scanning_rules_and_custom_values_are_cleared() {
    fn custom_email(v: &str) -> Result<(), ValidationError> {
        armature_validation::IsEmail::validate(v, "")
    }
    #[derive(Validate)]
    struct S {
        #[validate(email, regex = "^x$", length(max = 8))]
        a: String,
        #[validate(custom = "custom_email")]
        b: String,
    }
    let errors = S {
        a: "y".repeat(100),
        b: "SECRET-TOKEN".into(),
    }
    .validate()
    .unwrap_err();
    let constraints: Vec<_> = errors.iter().map(|e| e.constraint.as_str()).collect();
    assert_eq!(constraints, ["maxLength", "isEmail"]);
    assert_eq!(fields(&errors), ["a", "b"]);
    assert!(errors.iter().all(|e| e.value.is_none()));
}

/// `required` on an `Option` of any string type, not only `String`, still
/// rejects an empty value: the "is `Some`"-only form is limited to numbers,
/// `bool` and `char`.
#[test]
fn required_on_optional_string_types_checks_emptiness() {
    use std::sync::Arc;
    #[derive(Validate)]
    struct S {
        #[validate(required)]
        a: Option<Arc<str>>,
        #[validate(required)]
        b: Option<Box<str>>,
        #[validate(required)]
        c: Option<char>,
    }
    let empty = S {
        a: Some(Arc::from("")),
        b: Some(Box::from("")),
        c: Some(' '),
    };
    assert_eq!(fields(&empty.validate().unwrap_err()), ["a", "b"]);

    // Types that only deref to a string take the `Deref` level.
    struct Name(String);
    impl std::ops::Deref for Name {
        type Target = str;
        fn deref(&self) -> &str {
            &self.0
        }
    }
    #[derive(Validate)]
    struct D {
        #[validate(required)]
        a: Option<Arc<String>>,
        #[validate(required)]
        b: Option<Box<String>>,
        #[validate(required)]
        c: Option<std::rc::Rc<String>>,
        #[validate(required)]
        d: Option<Name>,
        #[validate(required)]
        e: Option<&'static str>,
    }
    let empty = D {
        a: Some(Arc::new(String::new())),
        b: Some(Box::new(String::new())),
        c: Some(std::rc::Rc::new(String::new())),
        d: Some(Name(String::new())),
        e: Some(""),
    };
    assert_eq!(
        fields(&empty.validate().unwrap_err()),
        ["a", "b", "c", "d", "e"]
    );
    let full = D {
        a: Some(Arc::new("x".into())),
        b: Some(Box::new("x".into())),
        c: Some(std::rc::Rc::new("x".into())),
        d: Some(Name("x".into())),
        e: Some("x"),
    };
    assert!(full.validate().is_ok());
}

/// `required` on an `Option` of a non-string type compiles and means
/// "is `Some`", whatever the type: the emptiness check is chosen by whether
/// the inner type is `AsRef<str>`, not by its name.
#[test]
fn required_on_optional_non_string_types_means_some() {
    #[derive(Debug)]
    struct Uuid([u8; 16]);
    #[derive(Validate)]
    struct S {
        #[validate(required)]
        id: Option<Uuid>,
        #[validate(required)]
        bytes: Option<Vec<u8>>,
        #[validate(required)]
        n: Option<std::primitive::u32>,
    }
    let none = S {
        id: None,
        bytes: None,
        n: None,
    };
    assert_eq!(fields(&none.validate().unwrap_err()), ["id", "bytes", "n"]);
    let some = S {
        id: Some(Uuid([0; 16])),
        bytes: Some(Vec::new()),
        n: Some(0),
    };
    assert!(some.validate().is_ok());
    let _ = some.id.map(|u| u.0);
}

/// A generic `Option<T>` field cannot be specialised on `T`, so `required`
/// asks for `T: AsRef<str>` and checks emptiness for every instantiation.
#[test]
fn required_on_generic_optional_field_checks_emptiness() {
    #[derive(Validate)]
    struct Named<T: AsRef<str>> {
        #[validate(required)]
        name: Option<T>,
    }
    let empty: Named<String> = Named {
        name: Some(String::new()),
    };
    assert_eq!(fields(&empty.validate().unwrap_err()), ["name"]);
    assert!(Named { name: Some("x") }.validate().is_ok());
    assert!(Named::<&str> { name: None }.validate().is_err());
}

/// Up to three `Deref` steps, references included, reach the emptiness check,
/// as deref coercion does for the same types outside an `Option`.
#[test]
fn required_on_optional_multi_deref_strings_checks_emptiness() {
    use std::rc::Rc;
    use std::sync::Arc;
    struct DStr(String);
    impl std::ops::Deref for DStr {
        type Target = str;
        fn deref(&self) -> &str {
            &self.0
        }
    }
    #[derive(Validate)]
    struct S {
        #[validate(required)]
        a: Option<&'static Arc<String>>,
        #[validate(required)]
        b: Option<Rc<DStr>>,
        #[validate(required)]
        c: Option<Arc<Box<String>>>,
        #[validate(required)]
        d: Option<&'static Rc<Box<String>>>,
    }
    let empty = S {
        a: Some(Box::leak(Box::new(Arc::new(String::new())))),
        b: Some(Rc::new(DStr(String::new()))),
        c: Some(Arc::new(Box::new(String::new()))),
        d: Some(Box::leak(Box::new(Rc::new(Box::new(String::new()))))),
    };
    assert_eq!(fields(&empty.validate().unwrap_err()), ["a", "b", "c", "d"]);
}

/// A generic inner type that can never be a string (`Vec<T>`, a map) means
/// "is `Some`"; one that can (`T` behind `Box` or `Cow`) needs `T: AsRef<str>`
/// and checks emptiness.
#[test]
fn required_on_generic_wrappers() {
    use std::borrow::Cow;
    use std::collections::HashMap;
    #[derive(Validate)]
    struct S<'a, T: AsRef<str> + ToOwned + ?Sized> {
        #[validate(required)]
        boxed: Option<Box<T>>,
        #[validate(required)]
        cow: Option<Cow<'a, T>>,
        #[validate(required)]
        many: Option<Vec<Box<T>>>,
        #[validate(required)]
        map: Option<HashMap<String, Box<T>>>,
    }
    let s: S<'_, str> = S {
        boxed: Some(Box::from("")),
        cow: Some(Cow::Borrowed("")),
        many: Some(Vec::new()),
        map: Some(HashMap::new()),
    };
    assert_eq!(fields(&s.validate().unwrap_err()), ["boxed", "cow"]);
}

/// A type macro in a struct without type parameters is a concrete type and
/// goes through the usual specialisation.
#[test]
fn required_on_optional_macro_type_in_concrete_struct() {
    macro_rules! num {
        () => {
            u32
        };
    }
    #[derive(Validate)]
    struct M {
        #[validate(required)]
        f: Option<num!()>,
    }
    assert!(M { f: Some(0) }.validate().is_ok());
    assert!(M { f: None }.validate().is_err());
}

/// A never-a-string container behind a pointer still means "is `Some`".
#[test]
fn required_on_generic_containers_behind_pointers() {
    use std::sync::Arc;
    #[derive(Validate)]
    struct S<'a, T> {
        #[validate(required)]
        slice: Option<&'a [T]>,
        #[validate(required)]
        boxed: Option<Box<[T]>>,
        #[validate(required)]
        shared: Option<Arc<Vec<T>>>,
    }
    let s: S<'_, u8> = S {
        slice: Some(&[]),
        boxed: Some(Box::new([])),
        shared: Some(Arc::new(Vec::new())),
    };
    assert!(s.validate().is_ok());
}

/// A generic string wrapper behind a pointer is checked as the wrapper.
#[test]
fn required_on_boxed_generic_string_wrapper() {
    struct W<T>(T);
    impl<T: AsRef<str>> AsRef<str> for W<T> {
        fn as_ref(&self) -> &str {
            self.0.as_ref()
        }
    }
    #[derive(Validate)]
    struct S<T>
    where
        W<T>: AsRef<str>,
    {
        #[validate(required)]
        f: Option<Box<W<T>>>,
    }
    let s: S<String> = S {
        f: Some(Box::new(W(String::new()))),
    };
    assert_eq!(fields(&s.validate().unwrap_err()), ["f"]);
}

/// A where-clause cannot steer the check on a concrete type: `required` on
/// it is decided outside the struct's generics. `Foo` only derefs to a
/// string (no `AsRef<str>`), so the where-clause would otherwise hide it.
#[test]
fn required_on_where_bounded_concrete_type() {
    struct Foo(String);
    impl std::ops::Deref for Foo {
        type Target = String;
        fn deref(&self) -> &String {
            &self.0
        }
    }
    type Alias = Foo;
    #[derive(Validate)]
    struct S<T>
    where
        Alias: std::ops::Deref<Target = T>,
    {
        #[validate(required)]
        a: Option<Foo>,
        #[validate(required)]
        b: Option<&'static Alias>,
    }
    let s: S<String> = S {
        a: Some(Foo(String::new())),
        b: Some(Box::leak(Box::new(Foo(String::new())))),
    };
    assert_eq!(fields(&s.validate().unwrap_err()), ["a", "b"]);
}

/// A where-clause on a container or an unrelated type does not change how
/// `required` treats it.
#[test]
fn required_with_unrelated_where_clauses() {
    #[derive(Validate)]
    struct S<T: std::fmt::Debug>
    where
        Vec<T>: std::fmt::Debug,
        u32: Clone,
    {
        #[validate(required)]
        many: Option<Vec<T>>,
        #[validate(required)]
        n: Option<u32>,
        #[validate(required)]
        name: Option<String>,
    }
    let s: S<u8> = S {
        many: Some(Vec::new()),
        n: Some(0),
        name: Some("x".into()),
    };
    assert!(s.validate().is_ok());
}

/// Lifetimes in a concrete inner type are fine in the isolated check, and
/// `Self` in a non-generic struct works.
#[test]
fn required_isolated_check_handles_lifetimes_and_self() {
    use std::borrow::Cow;
    #[derive(Validate)]
    struct S<'a> {
        #[validate(required)]
        name: Option<Cow<'a, str>>,
        #[validate(required)]
        tag: Option<&'a str>,
    }
    let s = S {
        name: Some(Cow::Borrowed("")),
        tag: Some(""),
    };
    assert_eq!(fields(&s.validate().unwrap_err()), ["name", "tag"]);
    #[derive(Validate)]
    struct Node {
        #[validate(required)]
        parent: Option<Box<Self>>,
    }
    assert!(Node { parent: None }.validate().is_err());
}

/// The isolated check keeps the struct's lifetimes: borrowed fields,
/// higher-ranked types and `Self` in a lifetime-only struct all compile.
#[test]
fn required_isolated_check_keeps_lifetimes() {
    use std::borrow::Cow;
    #[derive(Validate)]
    struct Comment<'a, 'b: 'a> {
        #[validate(required)]
        text: Option<Cow<'a, str>>,
        #[validate(required)]
        nested: Option<&'a &'b str>,
        #[validate(required)]
        parent: Option<Box<Self>>,
        #[validate(required)]
        hook: Option<for<'x> fn(&'x str)>,
    }
    fn noop(_: &str) {}
    let c = Comment {
        text: Some(Cow::Borrowed("")),
        nested: Some(&""),
        parent: None,
        hook: Some(noop),
    };
    assert_eq!(
        fields(&c.validate().unwrap_err()),
        ["text", "nested", "parent"]
    );
}

/// Type macros naming the struct's lifetimes or `Self` work in structs
/// without type parameters, and an isolated check declares only the
/// lifetimes its type uses (strict crates deny `unused_lifetimes`).
#[test]
#[deny(unused_lifetimes, single_use_lifetimes)]
fn required_with_macro_types_and_partial_lifetimes() {
    macro_rules! borrowed {
        ($l:lifetime) => {
            &$l str
        };
    }
    macro_rules! boxed_self {
        () => {
            Box<Self>
        };
    }
    #[derive(Validate)]
    struct M<'a> {
        #[validate(required)]
        b: Option<borrowed!('a)>,
        #[validate(required)]
        p: Option<boxed_self!()>,
    }
    let m = M {
        b: Some(""),
        p: None,
    };
    assert_eq!(fields(&m.validate().unwrap_err()), ["b", "p"]);

    #[derive(Validate)]
    struct G<'a, 'b, T> {
        #[validate(required)]
        name: Option<&'a str>,
        other: &'b T,
    }
    let g = G {
        name: Some(""),
        other: &0_u8,
    };
    assert_eq!(fields(&g.validate().unwrap_err()), ["name"]);
    let _ = g.other;
}

/// A true where-clause type predicate in a struct without type parameters
/// (`&'a Foo: Deref`, no `Target`) would shadow the real impl and drop the
/// in-place check to "is `Some`"; such structs isolate the check too.
#[test]
fn required_with_where_type_predicates_and_no_type_params() {
    use std::sync::Arc;
    struct Foo(String);
    impl std::ops::Deref for Foo {
        type Target = String;
        fn deref(&self) -> &String {
            &self.0
        }
    }
    #[derive(Validate)]
    struct L<'a>
    where
        &'a Foo: std::ops::Deref,
        Arc<&'a String>: std::ops::Deref,
    {
        #[validate(required)]
        a: Option<&'a Foo>,
        #[validate(required)]
        b: Option<Arc<&'a String>>,
    }
    let foo = Foo(String::new());
    let empty = String::new();
    let l = L {
        a: Some(&foo),
        b: Some(Arc::new(&empty)),
    };
    assert_eq!(fields(&l.validate().unwrap_err()), ["a", "b"]);

    #[derive(Validate)]
    #[allow(single_use_lifetimes)]
    struct N
    where
        for<'x> &'x Foo: std::ops::Deref,
    {
        #[validate(required)]
        a: Option<&'static Foo>,
    }
    let n = N {
        a: Some(Box::leak(Box::new(Foo(String::new())))),
    };
    assert_eq!(fields(&n.validate().unwrap_err()), ["a"]);
}

/// The reviewer's shapes: a lifetime-only struct whose where-clause bounds
/// a lifetime-carrying wrapper, and a `'static`-bounded lifetime in a
/// generic struct.
#[test]
fn required_where_clause_and_static_bound_shapes() {
    use std::sync::Arc;
    struct Wrapper<'a>(&'a str);
    impl std::ops::Deref for Wrapper<'_> {
        type Target = str;
        fn deref(&self) -> &str {
            self.0
        }
    }
    #[derive(Validate)]
    struct S<'a>
    where
        Wrapper<'a>: std::ops::Deref,
    {
        #[validate(required)]
        w: Option<Wrapper<'a>>,
        #[validate(required)]
        a: Option<Arc<Wrapper<'a>>>,
        #[validate(required)]
        b: Option<Box<Wrapper<'a>>>,
    }
    let s = S {
        w: Some(Wrapper("")),
        a: Some(Arc::new(Wrapper(""))),
        b: Some(Box::new(Wrapper(""))),
    };
    assert_eq!(fields(&s.validate().unwrap_err()), ["w", "a", "b"]);

    struct F<'a>(&'a str);
    impl AsRef<str> for F<'static> {
        fn as_ref(&self) -> &str {
            self.0
        }
    }
    #[derive(Validate)]
    struct G<'a: 'static, T> {
        #[validate(required)]
        f: Option<F<'a>>,
        t: T,
    }
    let g = G {
        f: Some(F("")),
        t: 0_u8,
    };
    assert_eq!(fields(&g.validate().unwrap_err()), ["f"]);
    let _ = g.t;
}

/// In a struct with a where-clause type predicate but no type or const
/// parameters, a type macro inner type is checked through `AsRef<str>`, and
/// a never-a-string container of `Self` still means "is `Some`".
#[test]
fn required_with_type_predicate_macros_and_self_containers() {
    macro_rules! id {
        ($t:ty) => {
            $t
        };
    }
    #[derive(Validate)]
    struct N<'a>
    where
        String: Sized,
    {
        #[validate(required)]
        name: Option<id!(String)>,
        #[validate(required)]
        kids: Option<Vec<Self>>,
        tag: &'a str,
    }
    let n = N {
        name: Some(String::new()),
        kids: Some(Vec::new()),
        tag: "",
    };
    assert_eq!(fields(&n.validate().unwrap_err()), ["name"]);
    let _ = n.tag;
}

/// The isolated check declares lifetimes a used one is bounded by, through
/// where-clause chains (`'a: 'b, 'b: 'c` with only `'a` and `'c` in the type).
#[test]
fn required_isolated_check_follows_lifetime_chains() {
    use std::marker::PhantomData;
    struct P<'a, 'c>(&'a str, PhantomData<&'c ()>);
    impl<'a: 'c, 'c> std::ops::Deref for P<'a, 'c> {
        type Target = str;
        fn deref(&self) -> &str {
            self.0
        }
    }
    #[derive(Validate)]
    struct X<'a, 'b, 'c, T>
    where
        'a: 'b,
        'b: 'c,
    {
        #[validate(required)]
        f: Option<P<'a, 'c>>,
        t: T,
        _p: PhantomData<&'b ()>,
    }
    let x = X {
        f: Some(P("", PhantomData)),
        t: 0_u8,
        _p: PhantomData,
    };
    assert_eq!(fields(&x.validate().unwrap_err()), ["f"]);
    let _ = x.t;
}
