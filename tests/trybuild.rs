//! Compile-pass coverage: every newly exported decorator must compile when
//! used as documented.

#[test]
fn decorators_compile() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/catch_pass.rs");
    t.pass("tests/ui/guard_pass.rs");
    t.pass("tests/ui/middleware_pass.rs");
}

/// Compile-fail coverage for `#[derive(Validate)]`: a regex that does not
/// compile, a `crate = ...` override that does not resolve (proving it is
/// used), and attribute mistakes that would otherwise weaken a rule silently.
#[test]
fn validate_derive_rejects_bad_attributes() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/validate_bad_regex.rs");
    t.compile_fail("tests/ui/validate_bad_crate_path.rs");
    t.compile_fail("tests/ui/validate_bad_rules.rs");
    t.compile_fail("tests/ui/validate_hygiene.rs");
    // rustc's own wording below; kept out of the macro-error files above so
    // a toolchain change touches only these snapshots.
    t.compile_fail("tests/ui/validate_length_bound_type.rs");
    t.compile_fail("tests/ui/validate_wrong_type.rs");
    t.compile_fail("tests/ui/validate_oversized_regex.rs");
}
