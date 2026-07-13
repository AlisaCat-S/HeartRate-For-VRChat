# Linux Multi-Platform Builds and Remote OSC Implementation Plan

## Preconditions

- Keep the task in planning until the user reviews `prd.md`, `design.md`, and
  this plan and explicitly approves `task.py start`.
- Before editing application code, load `trellis-before-dev` and the relevant
  backend guidelines.
- Preserve unrelated untracked Trellis/platform files in the dirty worktree.

## Implementation Checklist

1. Establish one canonical configuration template.
   - Add `config.example.toml` with the current embedded defaults and comments.
   - Replace the Rust raw string with `include_str!("../config.example.toml")`.
   - Ignore root `config.toml` while keeping the example tracked.
   - Add a test that deserializes the example and checks the default OSC target.

2. Make the OSC destination contract directly testable.
   - Extract IPv4/port resolution from `main` without changing fallback behavior.
   - Add valid-address and invalid-address tests.
   - Add loopback UDP coverage showing that both normal and clearing OSC bundles
     are sent to the resolved configured port.

3. Implement graceful Unix shutdown.
   - Enable Tokio signal support in `Cargo.toml` and refresh `Cargo.lock`.
   - Add a Unix-only SIGINT/SIGTERM wait future.
   - Select between the application loop and the shutdown future in `main`.
   - Reuse `run_exit_cleanup` and preserve the Windows console handler.
   - Remove the unconditional non-Windows registration-failure path.

4. Add Linux binary compatibility validation.
   - Add a shell script under `.github/scripts/` that verifies the
     ELF architecture and rejects GLIBC requirements newer than 2.31.
   - Make missing `readelf`, missing version metadata, or a wrong architecture
     a hard failure with useful diagnostics.

5. Replace the Windows-only release workflow with the multi-platform pipeline.
   - Add PR, `main`, version-tag, and manual triggers.
   - Keep Windows x86_64 as a native locked build.
   - Add native Linux x86_64 and ARM64 matrix entries using Ubuntu 20.04 job
     containers on `ubuntu-22.04` and `ubuntu-22.04-arm` runners.
   - Install target-native D-Bus development packages in each Linux container.
   - Run tests, release builds, architecture/glibc checks, and archive checks in
     all build jobs.
   - Upload temporary workflow artifacts only for tags.
   - Add a gated release job with the only `contents: write` permission and an
     exact three-archive completeness check.

6. Document Linux and remote-host operation.
   - Add a download/architecture table and Linux manual-launch instructions.
   - Document BlueZ, D-Bus, runtime library, and Bluetooth permission checks.
   - Explain the executable-directory config location and packaged
     `config.toml`.
   - Document remote `osc_ip`/`osc_port`, VRChat UDP 9000, and host firewall
     requirements without implying that VRChat's outbound IP option is needed.
   - State the glibc 2.31 baseline, ARM64-only first release, and lack of a
     bundled `systemd` unit.

7. Run the full quality gate and inspect the final diff.
   - Verify formatting, tests, lint, locked builds, template consistency, and
     workflow syntax locally where supported.
   - Validate x86_64 Linux in an Ubuntu 20.04 container when Docker is available.
   - Treat the ARM64 hosted CI job as the authoritative native ARM build gate.
   - Confirm no ordinary PR/main job has release-write permission or upload
     side effects.

## Validation Commands

Run locally on Windows:

```powershell
cargo fmt --all -- --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
git diff --check
```

Run in each Linux CI entry after dependency installation:

```bash
cargo fmt --all -- --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked --target "$RUST_TARGET"
bash .github/scripts/check-linux-binary.sh \
  "target/$RUST_TARGET/release/HeartRate-For-VRChat" \
  "$EXPECTED_MACHINE" 2.31
```

Inspect release archives before upload:

```bash
tar -tzf "HeartRate-For-VRChat-${GITHUB_REF_NAME}-linux-${ARCH}.tar.gz"
```

The Windows packaging step performs the equivalent ZIP content check in
PowerShell. Workflow YAML should also be checked with `actionlint` when the tool
is available; otherwise the first PR run is the authoritative Actions parser
and runner-label validation.

## Review Gates

- Gate 1: Config extraction does not change any default or user-visible parse
  fallback.
- Gate 2: Unix signal handling is cfg-gated and does not alter the Windows
  `SetConsoleCtrlHandler` behavior.
- Gate 3: Both Linux builds are native architecture builds inside a glibc 2.31
  userspace and pass the explicit ELF audit.
- Gate 4: A release is all-or-nothing across the three expected assets.
- Gate 5: README claims do not exceed the platforms and runtime baseline proven
  by CI.

## Rollback Points

- Revert config extraction independently if template embedding or packaging
  diverges; the original embedded string can be restored without format change.
- Revert Unix signal selection independently while leaving build artifacts
  intact, but do not retain Linux exit-cleanup claims in that state.
- Revert the workflow as one unit if hosted ARM runner or container execution
  proves unavailable; do not publish partial Linux support.
