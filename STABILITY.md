# Stability and support policy

OxideBot publishes the `oxidebot-core`, `oxidebot-runtime`,
`oxidebot-macros`, `oxidebot`, `oxidebot-testkit`, and
`oxidebot-adapter-console`, and `oxidebot-adapter-telegram` crates as one
versioned release set.

## Stable surface

From `1.0.0` onward, every documented public item in those published crates is
covered by SemVer. This explicitly includes the `oxidebot::core` and
`oxidebot::runtime` submodules: they are public access paths to the stable
`oxidebot-core` and `oxidebot-runtime` crate APIs, not private implementation
escapes.

Items annotated `#[doc(hidden)]` are implementation support for generated code
and are not intended for direct application use. They remain subject to the
requirements of the macros that reference them.

## Versioning

- Patch releases contain compatible bug, documentation, security, and
  performance fixes.
- Minor releases may add compatible APIs and behavior behind opt-in features.
- Major releases may remove or change public APIs and always include a
  migration guide.
- Pre-release versions may make breaking changes. A beta freezes public API;
  an RC changes it only for release-blocking correctness or security issues.

## Supported Rust version

The minimum supported Rust version is declared by the workspace
`rust-version`. It is tested in CI. Raising it follows the same compatibility
policy as any other user-visible requirement and is called out in the
changelog.

## Adapter compatibility

An adapter is 1.0-compatible only when it declares a compatible OxideBot
dependency range and passes the adapter contract plus its platform integration
tests. The core project does not imply compatibility for adapters released
against an earlier major version.
