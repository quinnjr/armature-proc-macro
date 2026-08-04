# Changelog — `armature-proc-macro`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** attribute macros reject what they used to discard. `#[body_limit(512kb)]` meant 512 *bytes* (rustc lexes it as a suffixed integer and the suffix was dropped), `#[timeout(hours = 2)]` meant two seconds, and unknown `#[module]`/`#[catch]` keys registered nothing — all silently.
- The parameter extractors work through `#[routes]`. `#[body]`, `#[param("id")]`, `#[query("page")]` and `#[header]` were documented but unreachable: route attributes were stripped before the extraction codegen could run, so a handler written as documented failed to compile.
- A handler carrying several route attributes registers all of them; every one after the first was dropped without a diagnostic.

### Changed — `0.2.0` → `0.2.1`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- The `Query` derive and the `#[query]` route-parameter extractor deserialize the raw query string instead of re-encoding already-decoded pairs, so a value containing a literal `&`, `=` or `%` round-trips as sent.

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
