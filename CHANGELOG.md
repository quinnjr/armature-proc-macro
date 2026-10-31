# Changelog — `armature-proc-macro`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

Breaking: the next release is 0.7.0 (`#[derive(Validate)]` reports nested
errors under a new field path and rejects invalid regex patterns at compile
time).

### Security

- **`#[derive(Validate)]` `range` with only `min` or only `max` rejects NaN.**
  It emitted `__v < min` / `__v > max`, both false for NaN, so
  `#[validate(range(min = 0.0))]` accepted `f64::NAN` (and `"NaN".parse()` is
  how a query or form value becomes one). Bounds are now checked through
  `PartialOrd::partial_cmp`, failing an unordered value.
- **`range` errors no longer carry the submitted value.** The derive attached
  it with `with_value`; validation errors are sent to clients.
- **`regex` patterns are compiled once per field instead of on every
  `validate()` call.** Recompiling per request cost milliseconds of CPU for
  Unicode patterns such as `[\w.-]{3,32}` on every request to the endpoint.

### Changed

- **Breaking:** a `regex` pattern that does not parse is a compile error at
  the pattern, not a 400 on every request carrying the regex error text.
- **Breaking:** errors from a nested `#[validate]` field are prefixed with the
  parent field's name (`billing.city`), so two fields of the same type report
  distinct paths.
- An error returned by a `custom` validator with an empty `field` is
  attributed to the annotated field.
- The generated code finds `armature-validation` through the deriving crate's
  manifest (a renamed dependency, or the `armature` facade's re-export)
  instead of assuming `::armature_validation`.

### Added

- `custom_with_field = "fn"`, which passes the field name to the validator
  (`fn(&Field, &str)`), so one function can serve several fields.
- Struct-level `#[validate(crate = "path")]` to name the path to
  `armature-validation` explicitly.
- Dependencies: `proc-macro-crate` (crate-path resolution) and `regex-syntax`
  (compile-time pattern check).

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
