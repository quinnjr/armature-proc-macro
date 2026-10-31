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
attaches submitted values to errors. The generated code needs
`armature-validation` 0.7 or later (`ValidationError::value`, `Matches`).
Code importing the derive through `armature_validation::Validate` gets
these fixes only once `armature-validation` depends on this release.

### Security

- **One-sided `range` rejects NaN.** `range(min = ..)` / `range(max = ..)`
  emitted `value < min` / `value > max`, both false for NaN, so
  `#[validate(range(min = 0.0))]` accepted `f64::NAN` (and `"NaN".parse()` is
  how a query or form value becomes one). Bounds are now checked through
  `PartialOrd::partial_cmp`, failing an unordered value.
- **Errors no longer carry the submitted value (breaking).** `range` attached
  it with `with_value`, and the built-in validators behind `length`, `email`,
  `url`, `regex` and `required` attach it in `armature-validation` 0.7.0; the
  generated code now clears it on every error it collects — built-in rules,
  `custom` validators (which may call a built-in validator) and nested
  `Validate` impls — since validation errors are sent to clients.
- **A `regex` pattern's text no longer reaches clients.** 0.6 turned an
  invalid pattern into a runtime error `invalid regex pattern: <parser
  output>`, exposing the pattern; patterns are now checked at compile time,
  and the remaining runtime fallback says only that the field could not be
  validated (constraint `matches`, was `regex`).
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
- **A field that fails `length(max = ..)` skips its `email`, `url` and
  `regex` rules**, so an oversized value is not scanned by every rule.

### Changed

- **Breaking:** `regex` patterns are compiled when the derive expands, with
  the same engine and limits as the runtime, so a pattern that does not parse
  or is too large to compile is a compile error at the pattern rather than a
  failure on every request.
- **Breaking:** errors from a nested `#[validate]` field are prefixed with the
  parent field's name (`billing.city`), so two fields of the same type report
  distinct paths.
- **Breaking:** duplicate `min`/`max` keys, literal bounds with `min > max`
  (integers compared exactly), `length()`/`range()` with no bounds, a rule
  repeated on one field (other than `regex` and `custom`), an empty
  `#[validate()]`, and struct-level `#[validate(...)]` other than
  `crate = "..."` (previously ignored) are compile errors.
- **Breaking:** field names are reported unraw (`r#type` as `type`), which
  changes the `field` clients see for raw-identifier fields.
- `range` compares the field and each bound by reference, so non-`Copy`
  fields and bounds such as `min = self.other` work, and evaluates each bound
  expression once. A one-sided float bound admits infinity (`range(min =
  0.01)` accepts `inf`); give both bounds to exclude it.
- A type error from a `length`, `email`, `url`, `regex`, `required`,
  or nested rule (`email` on a `u32`), and a `custom` argument-type
  mismatch, point at the rule, not at `#[derive(Validate)]`. A `range`
  comparison error or a `custom` return-type mismatch still points at the
  derive.
- Error order within a field changed: `custom` errors come first, then
  `length`, then the other rules in the order written.
- An error returned by a `custom` validator with an empty `field` is
  attributed to the annotated field; the error type is fixed to
  `ValidationError`, so a validator generic over its error type still infers.
- Generated locals use mixed-site hygiene and every macro call is fully
  qualified, so user items named `errors` or `e`, or a user `format!`, cannot
  collide with or alter the generated code. The impl is
  `#[automatically_derived]`.
- The generated code finds `armature-validation` through the deriving crate's
  manifest: a renamed dependency, or the `armature-framework` facade (as
  `::armature::armature_validation`, or under its renamed key), instead of
  assuming `::armature_validation`. A dependency renamed to a keyword is
  written as a raw identifier; one renamed to `self`, `crate` or similar
  falls back to `::armature_validation` (override with `crate = "..."`). If
  the crate is only a dev-dependency while the facade is a normal one, name
  the path with `crate = "..."`.

### Added

- `Option<T>` fields (detected syntactically; a type alias is not seen
  through, so an alias for `Option` also escapes the `Option<Option<_>>`
  check below): built-in rules and `nested` apply to the value when it is
  `Some` and are skipped for `None`. `custom` validators still get the whole
  `&Option<T>`, as before, so they can act on `None`. `required` also
  rejects `None`, and:
  - checks emptiness when the inner type is, or reaches through up to three
    `Deref` steps (references included), an `AsRef<str>` type — `String`,
    `&str`, `Box<str>`, `Arc<str>`, `Cow<str>`, `Arc<String>`,
    `&Arc<String>`, a `Deref<Target = str>` newtype;
  - otherwise means "is `Some`" (`Option<u32>`, `Option<Uuid>`);
  - on a generic inner type, which cannot be specialised on, fails closed:
    `T`, `T::Assoc` or (in a generic struct) a type macro, behind any
    `&`/`Box`/`Arc`/`Rc`/`Cow` layers, must be `AsRef<str>`; a container
    that is never a string (`Vec<T>`, maps, sets, `Result`, tuples, arrays,
    slices — also behind a pointer) means "is `Some`"; any other type
    mentioning a type or const parameter, or `Self` (`Wrapper<T>`,
    `Pin<Box<T>>`, `C<N>`), must be `AsRef<str>` under its pointer layers.
    In a struct with type or const parameters or a where-clause type
    predicate, a concrete inner type that mentions no type or const
    parameter is checked in a nested function that declares only the
    lifetimes it uses and those they are bounded by (with the bounds,
    `'static` included), so the struct's type
    parameters and where-clauses cannot change which check applies; in such
    a struct without type or const parameters, an inner type naming `Self` or
    using a type macro must be `AsRef<str>` under its pointer layers (unless
    it is a tuple, array or slice). A field type that is only well-formed
    because of a where-clause type predicate fails to compile there; use
    `custom`. `Pin<..>` and type aliases are not seen through, so in
    practice they need `custom`; a user impl of `AsRef<str>` for `Box<W<T>>`
    itself no longer suffices (`W<T>` must be `AsRef<str>`). These fail to
    compile, with a message pointing to `custom` for a presence-only check,
    rather than skip the check. (The message uses
    `#[diagnostic::on_unimplemented]`, which a `#![no_implicit_prelude]`
    crate cannot resolve; such a crate should use `custom` there.)
  - on `Option<Option<_>>`, also behind `&`/`Box`/`Arc`/`Rc`/`Cow`/`Pin`,
    is a compile error.

  `Box`, `Arc`, `Rc`, `Cow` and the container names are recognised by name,
  and only when written bare or under `std`/`alloc`/`core` (so
  `io::Result<T>` or `sync::Arc<T>` in a generic struct needs the full
  `std::` path or `custom`; a crate shadowing `std` itself is not
  supported). In a struct with type or const parameters or a where-clause
  type predicate, where `required` is checked in place through such
  pointers, the generated code asserts that they dereference to the type
  underneath and checks that type, so a look-alike pointer in scope fails
  to compile or is checked through to its contents rather than skip the
  check. Beyond that, a user type with one of those names should behave
  like the standard one (a user `Box<T>` without `Deref` fails to compile with "cannot be
  dereferenced"). In particular, a user type in scope under a container's
  bare name that derefs to a string only for some arguments or only under a
  where-clause, used with a type parameter, gets only the `Some` check: the
  derive cannot isolate a type that mentions a type parameter.
- `regex(full = "...")`, anchored to the whole value with `\A…\z` (which,
  unlike `^…$`, holds under `(?m)`).
- `custom_with_field = "fn"`, which passes the field name to the validator
  (`fn(&Field, &str)`), so one function can serve several fields.
- Struct-level `#[validate(crate = "path")]` to name the path to
  `armature-validation` explicitly.
- Dependencies: `proc-macro-crate` (crate-path resolution), `regex`
  (expansion-time pattern compilation) and `regex-syntax` (checking the bare
  pattern of `regex(full = ...)`).

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
