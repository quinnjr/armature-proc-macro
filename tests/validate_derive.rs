//! Behaviour of `#[derive(Validate)]`, against the published
//! `armature-validation` runtime.

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

/// Generated locals cannot be reached from user expressions: a `custom`
/// closure naming `__armature_errors` refers to the user's own item.
#[test]
fn generated_locals_are_hygienic() {
    #[allow(non_upper_case_globals)]
    const errors: u8 = 0;
    #[allow(non_upper_case_globals)]
    const e: u8 = 0;
    let _ = (errors, e);
    #[derive(Validate)]
    struct S {
        #[validate(length(min = 2))]
        a: String,
        #[validate(range(min = 1))]
        b: u8,
    }
    assert_eq!(
        fields(
            &S {
                a: "x".into(),
                b: 0
            }
            .validate()
            .unwrap_err()
        ),
        ["a", "b"]
    );
}
