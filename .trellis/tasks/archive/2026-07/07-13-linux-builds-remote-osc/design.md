# Linux Multi-Platform Builds and Remote OSC Design

## Decision Summary

- Keep one task because the CI artifacts, canonical config template, Linux
  shutdown behavior, and documentation must ship together to form a usable
  Linux release.
- Build three release targets: Windows x86_64, Linux x86_64 GNU, and Linux
  aarch64 GNU. ARMv7 is explicitly deferred.
- Use native GitHub-hosted x86_64 and ARM64 runners. Do not cross-compile the
  Linux D-Bus dependency.
- Run both Linux builds inside an Ubuntu 20.04 job container, then audit the ELF
  version requirements to enforce the glibc 2.31 ceiling.
- Validate all targets on pull requests and `main`; upload GitHub Release assets
  only for version tags.
- Keep IPv4 `osc_ip` plus `osc_port` as the public network contract. No hostname
  or IPv6 expansion is needed.
- Add Unix SIGINT/SIGTERM handling through Tokio and reuse the existing
  once-only synchronous cleanup path.

## Build Matrix

| Artifact | Runner | Build userspace | Rust target | Archive |
| --- | --- | --- | --- | --- |
| Windows x86_64 | `windows-latest` | Runner host | `x86_64-pc-windows-msvc` | ZIP |
| Linux x86_64 | `ubuntu-22.04` | `ubuntu:20.04` | `x86_64-unknown-linux-gnu` | tar.gz |
| Linux ARM64 | `ubuntu-22.04-arm` | `ubuntu:20.04` | `aarch64-unknown-linux-gnu` | tar.gz |

GitHub's hosted-runner reference lists both `ubuntu-22.04-arm` and
`ubuntu-24.04-arm` as standard ARM64 labels. `ubuntu-22.04-arm` is selected to
avoid a moving `latest` label; the job container, rather than the VM image,
defines the linked userspace baseline.

The Ubuntu 20.04 container provides glibc 2.31 on both native architectures.
Each Linux job installs `build-essential`, `pkg-config`, `libdbus-1-dev`,
`binutils`, and the minimal tools needed by checkout and Rust setup. It asserts
that `rustc` reports the expected host/target before building with `--locked`.

## Workflow Topology

`.github/workflows/release.yml` becomes a combined validation and release
workflow with these triggers:

- pull requests targeting `main`;
- pushes to `main`;
- version tags matching the existing release convention;
- optional manual dispatch for CI diagnosis without publishing.

The workflow has separate Windows and Linux build jobs. The Linux job uses a
two-entry native-runner matrix. Every entry runs tests, produces a locked
release build, validates the binary architecture, stages the release contents,
and creates its archive.

Only tag jobs upload the archives as temporary workflow artifacts. A final
`release` job:

1. is gated on a version-tag ref;
2. depends on every build job;
3. alone receives `contents: write`;
4. downloads all three archives into one directory;
5. rejects a missing or unexpected asset set; and
6. sends the complete set to `softprops/action-gh-release`.

Normal pull-request and branch jobs use read-only contents permission and never
run release upload actions.

## Linux Compatibility Contract

The Linux executables remain dynamically linked because `btleplug` uses BlueZ
through D-Bus. The release documentation must state that a target machine needs
BlueZ, a running Bluetooth service/system D-Bus, the `libdbus-1` runtime, and
permission to access the Bluetooth controller.

Building in Ubuntu 20.04 is necessary but not sufficient evidence for the
declared baseline. A small shell script in the repository inspects the final
ELF version requirements with `readelf`, extracts every `GLIBC_x.y`
requirement, and fails if any version sorts above 2.31. The CI log prints the
discovered versions and the binary architecture for diagnosis.

## Artifact Contract

Archive names retain the project and version and add an unambiguous platform:

- `HeartRate-For-VRChat-<tag>-windows-x86_64.zip`
- `HeartRate-For-VRChat-<tag>-linux-x86_64.tar.gz`
- `HeartRate-For-VRChat-<tag>-linux-aarch64.tar.gz`

Each archive contains one top-level directory with:

- the platform executable;
- `config.toml`, copied from the canonical example for immediate editing;
- `README.md`; and
- `LICENSE`.

A new tracked `config.example.toml` becomes the single template source. Rust
embeds it with `include_str!`, while release staging copies it as `config.toml`.
This prevents the generated and packaged defaults from drifting. Local
`config.toml` is ignored by Git.

## OSC Configuration and Verification

The application already binds an ephemeral source socket on `0.0.0.0` and sends
to a `SocketAddrV4` made from `Config.osc_ip` and `Config.osc_port`. Refactor only
the address resolution into a focused helper so it can be tested without BLE:

- a valid IPv4 and port are preserved exactly;
- an invalid IPv4 logs the existing warning and falls back to
  `127.0.0.1` while preserving the configured port;
- `send_osc` and `clear_state` continue receiving the resolved address, so
  normal messages and clearing messages share one destination.

A loopback UDP test uses a dynamically assigned port to prove that the resolved
config destination receives an OSC bundle. A second test covers invalid-address
fallback. The canonical config example is parsed in a test to prevent release
of an invalid template.

The README remote-host procedure states that VRChat receives OSC on UDP 9000 by
default, `osc_ip` must be the LAN IPv4 address of the VRChat host, a customized
VRChat input port must also be set in `osc_port`, and the host firewall must
allow the inbound UDP traffic. VRChat's `--osc=inPort:senderIP:outPort` middle
field controls where VRChat sends outbound OSC and is not required merely to
receive this application's packets.

## Unix Shutdown

Windows keeps its synchronous `SetConsoleCtrlHandler` path unchanged. On Unix:

1. enable Tokio's `signal` feature;
2. wait for SIGINT (`Ctrl-C`) or SIGTERM alongside `main_loop` with
   `tokio::select!`;
3. when a shutdown signal wins, call `run_exit_cleanup()` synchronously; and
4. return normally so the process exits after cleanup.

`CLEANUP_DONE` continues to guarantee that cleanup is executed at most once.
The current non-Windows `register_exit_handler() -> false` stub and its
unconditional warning are removed. No `systemd` unit is added, although
SIGTERM handling leaves that deployment option open for a later task.

## Validation Strategy

- Unit tests: config template parsing, valid/invalid OSC address resolution,
  loopback UDP delivery, and cleared-state delivery.
- Windows CI: locked tests and release build.
- Both Linux CI entries: locked tests, release build, target architecture
  inspection, glibc symbol audit, and archive-content inspection.
- Workflow review: tag-only upload conditions, least-privilege permissions,
  exact three-asset gate, and no release side effects on PRs or `main` pushes.
- Hardware residual: CI cannot prove access to a real BlueZ controller or a
  remote VRChat instance; README prerequisites and a manual smoke-test procedure
  cover that boundary.

## Rollout and Rollback

The existing version-tag release entry point and Windows artifact remain. If a
Linux job is unavailable or fails, the release job does not run, preventing a
partial multi-platform release. Rollback is limited to reverting the workflow,
Tokio signal feature, config-template extraction, and README changes; there is
no data migration or persistent format change.

## Research Sources

- GitHub hosted runners:
  https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- VRChat OSC overview and default ports:
  https://docs.vrchat.com/docs/osc-overview
- Locked `btleplug 0.11.8` source in the local Cargo registry, including its
  Linux `bluez-async` and `dbus` target dependencies.
