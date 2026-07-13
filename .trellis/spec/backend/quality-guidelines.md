# Quality Guidelines

> Code quality standards for backend development.

---

## Overview

<!--
Document your project's quality standards here.

Questions to answer:
- What patterns are forbidden?
- What linting rules do you enforce?
- What are your testing requirements?
- What code review standards apply?
-->

(To be filled by the team)

---

## Forbidden Patterns

<!-- Patterns that should never be used and why -->

(To be filled by the team)

---

## Required Patterns

<!-- Patterns that must always be used -->

(To be filled by the team)

---

## Testing Requirements

<!-- What level of testing is expected -->

(To be filled by the team)

---

## Code Review Checklist

<!-- What reviewers should check -->

(To be filled by the team)

## Scenario: Cross-Platform Rust Release Artifacts

### 1. Scope / Trigger

Apply this contract whenever a change adds a supported operating system or CPU
architecture, changes release packaging, changes `config.toml`, or changes
platform-specific shutdown behavior. These paths form one release boundary:
the binary, packaged config, runtime behavior, CI evidence, and README claims
must agree.

### 2. Signatures

- Release trigger: a Git tag matching `v<major>.<minor>.<patch>[suffix]`.
- Validation triggers: pull requests to `main`, pushes to `main`, and manual
  workflow dispatch.
- Supported Rust targets:
  - `x86_64-pc-windows-msvc`
  - `x86_64-unknown-linux-gnu`
  - `aarch64-unknown-linux-gnu`
- OSC config boundary:
  - `osc_ip: String`, parsed strictly as IPv4
  - `osc_port: u16`
  - invalid `osc_ip` -> `127.0.0.1` with the configured port preserved
- Linux audit command:

  ```bash
  bash .github/scripts/check-linux-binary.sh \
    <binary> <expected-readelf-machine> 2.31
  ```

### 3. Contracts

- `config.example.toml` is the only tracked template. Runtime first-run
  generation uses `include_str!("../config.example.toml")`; release staging
  copies the same file as `config.toml`.
- Every archive contains one top-level directory with the executable,
  `config.toml`, `README.md`, and `LICENSE`.
- Pull-request and branch jobs have read-only contents permission and never
  upload release candidates.
- Only the tag-gated release job has `contents: write`, and it publishes only
  after all three platform archives exist.
- Linux builds run natively on their CPU architecture inside an Ubuntu 20.04
  userspace with target-native `libdbus-1-dev`; do not cross-compile D-Bus from
  a Windows or mismatched Linux host.
- Linux artifacts are dynamically linked and may require no GLIBC symbol newer
  than 2.31.
- Windows keeps `SetConsoleCtrlHandler`; Unix waits for SIGINT and SIGTERM and
  calls the shared once-only OSC cleanup before returning.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| Rust host differs from matrix target | Build job fails before compilation |
| Native D-Bus development package is missing | Linux build fails; do not bypass `pkg-config` |
| ELF machine differs from matrix architecture | Audit fails |
| Any required GLIBC version is greater than 2.31 | Audit fails |
| No GLIBC version metadata is found | Audit fails; do not claim GNU compatibility |
| Archive lacks any required file | Packaging job fails |
| Fewer or more than three tag assets are downloaded | Release job fails without publishing |
| `osc_ip` is invalid | Warn and use `127.0.0.1:<configured port>` |
| Unix receives SIGINT or SIGTERM | Send OSC clear state once, then exit normally |

### 5. Good / Base / Bad Cases

- Good: an ARM64 tag build runs on an ARM64 hosted runner in Ubuntu 20.04,
  passes the AArch64 and GLIBC audits, and publishes a tarball containing the
  canonical config.
- Base: a pull request builds and inspects all platform archives but performs
  no artifact upload or release mutation.
- Bad: an ARM64 artifact is cross-linked against an unverified sysroot, or a
  binary built on a newer userspace is published based only on its filename.

### 6. Tests Required

- Parse `config.example.toml` and assert equality with `Config::default()`.
- Assert a valid remote IPv4 and port resolve unchanged.
- Assert invalid IPv4 falls back to localhost without changing the port.
- Use a loopback UDP receiver to assert normal and cleared OSC bundles reach
  the resolved target.
- Run `cargo fmt --check`, locked tests, Clippy with warnings denied, and a
  locked release build on every supported native CI target.
- Run `actionlint` on workflow changes and exercise both the accepted and
  rejected GLIBC audit paths.

### 7. Wrong vs Correct

#### Wrong

```rust
// A second template can silently drift from release packaging.
const CONFIG_TEMPLATE: &str = "osc_ip = \"127.0.0.1\"";
```

```yaml
# A filename is not evidence that this is an ARM64 or glibc-compatible binary.
- run: cargo build --release
- run: mv target/release/app app-linux-aarch64
```

#### Correct

```rust
const CONFIG_TEMPLATE: &str = include_str!("../config.example.toml");
```

Build on a native runner, assert the Rust host triple, inspect the final ELF
machine and GLIBC requirements, inspect archive contents, and let only the
tag-gated release job publish the complete asset set.
