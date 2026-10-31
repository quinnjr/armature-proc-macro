# Changelog — `armature-proc-macro`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

Breaking: the next release is 0.7.0. `#[derive(Validate)]` reports nested
errors under a new field path, rejects at compile time attribute forms it used
to accept (invalid or oversized regex patterns, unknown `regex(...)` keys,
duplicate or inverted bounds, empty `#[validate()]`, non-`usize` length
bounds, struct-level `#[validate(...)]` other than `crate`), and no longer
attaches submitted values to errors.

### Security

- **One-sided `range` rejects NaN.** `range(min = ..)` / `range(max = ..)`
  emitted `value < min` / `value > max`, both false for NaN, so
  `#[validate(range(min = 0.0))]` accepted `f64::NAN` (and `"NaN".parse()` is
  how a query or form value becomes one). Bounds are now checked through
  `PartialOrd::partial_cmp`, failing an unordered value.
- **Errors no longer carry the submitted value (breaking).** `range` attached
  it with `with_value`, and the built-in validators behind `length`, `email`,
  `url`, `regex` and `required` attach it in `armature-validation` 0.7.0; the
  generated code now clears it on every built-in rule's error, since
  validation errors are sent to clients. A value a `custom` validator attaches
  on purpose is kept.
- **`regex(pattern = ...)` no longer accepts any key.** `regex(full = ...)` or
  a typo such as `regex(patern = ...)` silently became a substring match; an
  unknown key is now a compile error, and `regex(full = "...")` is a real
  whole-value match (see Added).
- **`length` bounds are type-checked, not cast.** A non-literal bound was
  emitted as `expr as usize`, so `length(max = NEG)` with `const NEG: i64 = -1`
  became `usize::MAX` and lifted the limit. A non-`usize` bound is now a type
  error.
- **`regex` patterns are compiled once per rule instead of on every
  `validate()` call.**

### Changed

- **Breaking:** `regex` patterns are compiled when the derive expands, with
  the same engine and limits as the runtime, so a pattern that does not parse
  or is too large to compile is a compile error at the pattern rather than a
  failure on every request.
- **Breaking:** errors from a nested `#[validate]` field are prefixed with the
  parent field's name (`billing.city`), so two fields of the same type report
  distinct paths.
- **Breaking:** duplicate `min`/`max` keys, literal bounds with `min > max`, an
  empty `#[validate()]`, and struct-level `#[validate(...)]` other than
  `crate = "..."` (previously ignored) are compile errors.
- `range` compares through a reference (non-`Copy` fields work) and evaluates
  each bound expression once.
- An error returned by a `custom` validator with an empty `field` is
  attributed to the annotated field; the error type is fixed to
  `ValidationError`, so a validator generic over its error type still infers.
- Field names are reported unraw (`r#type` as `type`).
- Generated locals use mixed-site hygiene and every macro call is fully
  qualified, so user items named `errors` or `e`, or a user `format!`, cannot
  collide with or alter the generated code. The impl is
  `#[automatically_derived]`.
- The generated code finds `armature-validation` through the deriving crate's
  manifest: a renamed dependency, or the `armature-framework` facade (as
  `::armature::armature_validation`, or under its renamed key), instead of
  assuming `::armature_validation`.

### Added

- `Option<T>` fields: rules apply to the value when present; `required` also
  rejects `None`.
- `regex(full = "...")`, anchored to the whole value with `\A…\z` (which,
  unlike `^…$`, holds under `(?m)`).
- `custom_with_field = "fn"`, which passes the field name to the validator
  (`fn(&Field, &str)`), so one function can serve several fields.
- Struct-level `#[validate(crate = "path")]` to name the path to
  `armature-validation` explicitly.
- Dependencies: `proc-macro-crate` (crate-path resolution) and `regex`
  (expansion-time pattern compilation).

## [0.6.0] - 2026-10-29

### Changed

- Requires `armature-core` 0.12 (was `0.11`).

## [0.5.0] - 2026-10-17

### Security

- **Breaking:** controller-level `#[middleware]` now runs **before** controller-level `#[guard]`s (NestJS order: middleware -> guards -> handler), so an authentication middleware such as `JwtAuthMiddleware` attaches the verified `UserContext` before role/permission guards inspect it. Middleware now also sees (and can post-process) a guard's 403. Migration: none for most apps; middleware that relied on running only for already-authorized requests must check authorization itself.
- An unknown HTTP method in a route attribute is a compile error, and the module route registrar refuses to register an unknown method instead of silently serving it as `GET`.
- **Breaking:** `#[cache]` keys no longer collide. The default key names the module path, the `Self` type (for methods), the function, its attribute location and its return type, followed by every argument binding — including destructured ones like `Path(id): Path<u64>` — as length-prefixed JSON (arguments must implement `Serialize`; a value that fails to encode bypasses the cache). Custom `key = "..."` templates percent-escape each interpolated value, must reference every argument, and must separate placeholders with a delimiter such as `:`, so `("1:2", "3")` and `("1", "2:3")` no longer share an entry. Arguments bound to `_` or elided with `..` are compile errors. Migration: existing cache entries are simply missed; add `Serialize` to argument types or name ignored arguments.

## [0.4.1] - 2026-09-15

### Changed

- Requires `armature-core` 0.10 (was `0.9`).
- Migrated to `syn` 3. `#[routes]` now reads the negative-impl `!` from `ItemImpl::modifiers.polarity` (syn 3 removed it from `trait_`); expansion is unchanged.

## [0.4.0] - 2026-08-05

### Changed

- **Requires `armature-core` 0.9 (breaking).** The requirement moved `0.8` →
  `0.9`. `armature-core 0.9.0` itself moves `armature-h1` across a breaking
  0.x boundary; because `armature-core` types appear in this crate's own
  public API, the requirement change is breaking here too and the minor moves
  with it. Under Cargo's 0.x caret rules the 0.8 and 0.9 types are distinct
  and do not unify, so a consumer holding an `armature-core 0.8` type cannot
  pass it to this crate. Part of the `armature-core 0.9.0` release train; see
  `armature-core`'s CHANGELOG for the publish order.

## [0.3.1] - 2026-08-04

### Fixed

- Requirements on sibling armature crates name a minor instead of `0`. Under
  Cargo's 0.x rules `version = "0"` matches any release ever made, and edition
  2024 selects the MSRV-aware resolver, so a consumer declaring an older
  `rust-version` was handed the oldest version satisfying it — resolving
  `armature-core = "0"` on Rust 1.89 produced `armature-core 0.2.3` while an
  explicit `armature-core = "0.8"` elsewhere in the same graph pulled 0.8.2.
  Two copies of core, and a build failing on symbols the older one lacks. Each
  0.x minor in this family is a breaking change, so the requirement now names
  one. No API change.
