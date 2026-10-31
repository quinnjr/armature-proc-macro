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
