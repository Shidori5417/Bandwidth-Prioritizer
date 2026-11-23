# Bandwidth-Prioritizer (bwp)

[![CI](https://github.com/Shidori5417/Bandwidth-Prioritizer/actions/workflows/ci.yml/badge.svg)](https://github.com/Shidori5417/Bandwidth-Prioritizer/actions/workflows/ci.yml)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)

Network traffic shaper that **throttles non-critical apps while priority apps (Discord, Zoom, ...) are running**, and restores full bandwidth when they close.

## Status: prototype

This project is an early prototype. **It does not shape any network traffic yet.**

| Area | State |
|---|---|
| Engine state machine (Normal / PriorityMode), per-PID connection tracking, process-name resolution | Works (unit tested) |
| IPC (named pipe / Unix socket), `bwp` CLI and TUI, profiles, add/remove priority apps | Works |
| `config.toml` loading and hot reload, Windows Service install/start/stop | Works (Windows) |
| Token-bucket algorithm | Implemented, but not wired to any packet path |
| Packet capture and shaping (WinDivert) | **Not implemented.** The Windows controller accepts commands and logs a warning, it never opens a WinDivert handle (capturing without re-injecting would black-hole traffic) |
| Connection monitoring (ETW) | **Not implemented.** The ETW listener reports an error on startup |
| Connection monitoring (polling fallback) | **Stub.** The loop runs but `GetExtendedTcpTable` snapshots return an empty list, so no connection events are produced and Priority Mode never engages on its own |
| Linux (`tc htb`) | Placeholder. The workspace builds and its tests run on Linux, but only as a development convenience |

In short: the control plane is built, the data plane (seeing connections, limiting packets) is not. See [`progress.md`](progress.md) for the roadmap.

## Features

Intended behaviour (see [Status](#status-prototype) for what is actually implemented):

- Detects running priority processes and throttles every other process with a token-bucket limiter
- Background engine (`bwp-engine`) managed from a CLI, with an optional Windows Service install
- Live `ratatui` dashboard (`bwp top`)
- Named profiles (`gaming`, `meeting`, `download`) switchable at runtime
- Add/remove priority apps on the fly, no restart
- `config.toml` is watched and hot-reloaded

## Install / Build

Requirements: Rust >= 1.85 (edition 2024). The engine only runs on Windows, an elevated (Administrator) shell is needed for service management. The workspace also builds and tests on Linux, where `bwp-engine` just exits with an explanatory error.

```powershell
cargo build --release
```

`build_release.ps1` packages `bwp.exe`, `bwp-engine.exe` and the config into `target\bwp-windows-v<version>.zip` (the version comes from `Cargo.toml`). `WinDivert.dll` is not shipped yet because WinDivert is not used yet.

## Usage

```powershell
# Run the engine as an auto-start Windows Service (elevated)
.\target\release\bwp.exe service install
.\target\release\bwp.exe service start
.\target\release\bwp.exe service status     # also: stop, uninstall

# Or spawn the engine as a plain background process
.\target\release\bwp.exe start
.\target\release\bwp.exe stop

# Talk to the running engine
.\target\release\bwp.exe status             # state, active profile, throttled PIDs, enforced limit
.\target\release\bwp.exe top                # live TUI (q / Esc: quit, s: stop engine)
.\target\release\bwp.exe set gaming         # switch profile
.\target\release\bwp.exe add "zoom.exe"     # add a priority app
.\target\release\bwp.exe remove "zoom.exe"
```

## Configuration

The engine looks for its config in this order: the `BWP_CONFIG` environment variable, `config.toml` next to `bwp-engine.exe` (so it also works as a service, whose working directory is `System32`), then `config.toml` in the current directory. If none can be loaded it logs a warning and runs with built-in defaults. See the file in this repo:

```toml
# Top-level keys must come before the first [table] header.
critical_apps = ["discord", "zoom", "code", "teams", "slack"]  # case-insensitive, ".exe" optional
default_throttle_limit_kbps = 1000                              # cap for non-critical processes

[general]
poll_interval_ms = 500
log_level = "info"                 # trace | debug | info | warn | error
# log_file = "bwp.log"

[profiles.gaming]
description = "Low-latency gaming"
critical_apps = ["steam", "epicgameslauncher", "riotclientservices"]
throttle_limit_kbps = 500
```

Edits are picked up automatically (reloaded once the file has been quiet for 300 ms); no restart needed.

## Architecture

| Crate | Role |
|---|---|
| `bwp-core` | Config models and hot-reload, IPC protocol (named pipe on Windows, Unix socket on Linux), shared types and errors |
| `bwp-monitor` | Process and network connection monitoring trait; Windows implementation is a stub (see Status) |
| `bwp-controller` | `TrafficController` trait and token bucket; Windows (WinDivert) and Linux (`tc htb`) backends are placeholders |
| `bwp-engine` | Daemon: state machine (Normal / PriorityMode), IPC server, service entry |
| `bwp-cli` | `bwp` binary: `clap` CLI and `ratatui` TUI |

```
monitor --> engine state machine --> controller (token bucket / WinDivert)
config.toml (hot-reload) --^              ^-- IPC <-- bwp CLI / TUI
```

## Development

CI runs on Ubuntu and Windows (stable and nightly) with `RUSTFLAGS="-D warnings"`. To reproduce locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
```

Lint levels are configured in `[workspace.lints]` in `Cargo.toml` (`clippy::all`, `unwrap_used` and `expect_used` outside tests, `missing_docs`).

## License

Dual licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
