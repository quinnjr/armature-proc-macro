# Changelog — `armature-proc-macro`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

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
