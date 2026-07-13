# Linux multi-platform builds and remote OSC

## Goal

Provide release artifacts that let the application run on common Linux x86_64
hosts and ARM Linux development boards, while preserving the existing Windows
release and making the remote VRChat OSC deployment scenario explicit and
verifiable.

## Background

- `.github/workflows/release.yml:1` currently defines a Windows-only release;
  its sole job uses `windows-latest` at line 11 and runs for version tags.
- The application already reads `osc_ip` and `osc_port` from `config.toml`,
  parses `osc_ip` as an IPv4 address, binds its UDP socket to `0.0.0.0:0`, and
  sends every OSC bundle to the configured address. A Linux board can therefore
  send heart-rate data to VRChat on another LAN host without a new config field
  (`src/main.rs:40`, `src/main.rs:634`, and `src/main.rs:728`).
- Invalid `osc_ip` values currently fall back to `127.0.0.1` with a warning.
  Hostnames and IPv6 addresses are not supported.
- The configuration template currently exists only as a Rust string constant
  at `src/main.rs:72`; when `config.toml` is absent, that template is generated
  beside the executable.
- On Linux, locked `btleplug 0.11.8` (`Cargo.lock:111`) uses BlueZ and D-Bus.
  Building requires the native D-Bus development library through
  `libdbus-sys`/`pkg-config`, and running requires a working BlueZ/D-Bus
  environment and Bluetooth permissions.
- The current non-Windows `register_exit_handler` implementation always returns
  `false` at `src/main.rs:372`. A Linux build therefore always prints an
  exit-cleanup warning and does not send the configured remote target a cleared
  heart-rate state when it receives a normal termination signal.

## Requirements

- R1: Keep producing the existing Windows x86_64 release artifact on version
  tags.
- R2: Produce a Linux x86_64 release artifact on the same version tags.
- R3: Produce a Linux ARM64 (`aarch64`) release artifact suitable for modern
  64-bit development boards.
- R4: Give every downloadable artifact an unambiguous platform and architecture
  name and package it in a format normally usable on that platform.
- R5: Make Linux native build dependencies explicit and install them in CI so
  release builds are reproducible.
- R6: Document Linux runtime prerequisites and the remote OSC deployment flow,
  including setting `osc_ip` to the IPv4 address of the VRChat host, setting
  `osc_port` when non-default, and allowing the UDP traffic through the host
  firewall.
- R7: Preserve the current `osc_ip`/`osc_port` behavior and verify that normal
  sends and disconnect/exit clearing messages use the configured remote target.
- R8: Do not claim that a produced Linux artifact supports an architecture or
  runtime environment that CI has not actually built and checked.
- R9: On Linux, handle normal interactive and service termination signals so
  the configured remote OSC target receives the same cleared heart-rate state
  used by Windows exit cleanup before the process exits.
- R10: Validate Windows x86_64, Linux x86_64, and Linux ARM64 builds for pull
  requests and pushes to `main`; create and upload GitHub Release assets only
  for matching version tags.
- R11: Keep release-write permission and upload steps out of ordinary pull
  request and `main` validation jobs.
- R12: Build dynamically linked Linux release binaries against a userspace no
  newer than glibc 2.31 so they remain compatible with Ubuntu 20.04, Debian 11,
  and comparable 64-bit development-board distributions.
- R13: Maintain one tracked configuration example as the source for both
  first-run generation and release packaging; every archive must expose it as
  an immediately editable `config.toml` beside the executable.

## Acceptance Criteria

- [ ] Pushing a matching version tag builds and attaches Windows x86_64, Linux
  x86_64, and Linux ARM64 artifacts to one GitHub Release.
- [ ] Pull requests and pushes to `main` run the complete three-platform build
  matrix without creating a GitHub Release or uploading release assets.
- [ ] Each archive can be identified by OS, CPU architecture, and version from
  its filename and contains one top-level directory with the correctly named
  executable, a parseable `config.toml`, `README.md`, and `LICENSE`.
- [ ] Linux jobs install or otherwise provide the target-appropriate D-Bus
  development dependency required by `btleplug`.
- [ ] Both Linux artifacts require no glibc version newer than 2.31; CI checks
  the produced binaries rather than relying only on runner image assumptions.
- [ ] The release workflow fails rather than publishing a missing, stale, or
  incorrectly targeted binary.
- [ ] README instructions cover Linux Bluetooth/BlueZ prerequisites, executable
  permissions, and configuration of a remote VRChat host.
- [ ] Source-level tests or focused checks demonstrate that a configured valid
  IPv4 address and port become the OSC destination, while invalid IPv4 input
  retains the documented localhost fallback.
- [ ] A test parses the same tracked config template used for first-run
  generation and release packaging, preventing the two forms from drifting.
- [ ] Existing Windows behavior and the default `127.0.0.1:9000` configuration
  remain compatible.
- [ ] On Linux, a normal `Ctrl-C` or termination signal runs exit cleanup once,
  sends the cleared OSC state to the configured IP and port, and does not print
  the current unconditional "exit cleanup registration failed" warning.

## Out Of Scope

- Selecting a specific physical Bluetooth adapter on Windows.
- Building a 32-bit ARM (`armv7-unknown-linux-gnueabihf`) artifact in the first
  multi-platform release.
- Adding hostname or IPv6 OSC destinations unless separately requested.
- End-user container images, installers, or package-manager repositories. CI
  build containers are part of the glibc compatibility strategy.
- `systemd` service units or other unattended startup definitions in the first
  Linux release; this release documents interactive/manual launch only.
