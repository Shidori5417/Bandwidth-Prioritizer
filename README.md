# Bandwidth-Prioritizer (bwp)

[![CI](https://github.com/Shidori5417/Bandwidth-Prioritizer/actions/workflows/ci.yml/badge.svg)](https://github.com/Shidori5417/Bandwidth-Prioritizer/actions/workflows/ci.yml)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)

Network traffic shaper that **throttles non-critical apps while priority apps (Discord, Zoom, ...) are running**, and restores full bandwidth when they close.

> **Platform status:** the Windows backend (WinDivert packet shaping + Windows Service) is the working one. The Linux controller (`tc htb`) is currently a placeholder and does not shape traffic yet; the workspace does not build on Linux at the moment.

## Features

- Detects running priority processes and throttles every other process with a token-bucket limiter
- Background engine (`bwp-engine`) managed from a CLI, with an optional Windows Service install
- Live `ratatui` dashboard (`bwp top`)
- Named profiles (`gaming`, `meeting`, `download`) switchable at runtime
- Add/remove priority apps on the fly, no restart
- `config.toml` is watched and hot-reloaded

## Install / Build

Requirements: Rust >= 1.85 (edition 2024), Windows, an elevated (Administrator) shell for WinDivert and service management.

```powershell
cargo build --release
```

`WinDivert.dll` is copied next to the binaries by the build script. `build_release.ps1` packages `bwp.exe`, `bwp-engine.exe`, `WinDivert.dll` and the config into a zip.

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

The engine reads `config.toml` from its working directory (see the file in this repo):

```toml
[general]
poll_interval_ms = 500
log_level = "info"                 # trace | debug | info | warn | error
# log_file = "bwp.log"

critical_apps = ["discord", "zoom", "code", "teams", "slack"]
default_throttle_limit_kbps = 1000 # cap for non-critical processes

[profiles.gaming]
description = "Low-latency gaming"
critical_apps = ["steam", "epicgameslauncher", "riotclientservices"]
throttle_limit_kbps = 500
```

Edits are picked up automatically; no restart needed.

## Architecture

| Crate | Role |
|---|---|
| `bwp-core` | Config models and hot-reload, IPC protocol (named pipe on Windows, Unix socket on Linux), shared types and errors |
| `bwp-monitor` | Process and network connection monitoring (ETW / polling on Windows) |
| `bwp-controller` | Traffic shaping: token bucket + WinDivert (Windows), `tc htb` placeholder (Linux) |
| `bwp-engine` | Daemon: state machine (Normal / PriorityMode), IPC server, service entry |
| `bwp-cli` | `bwp` binary: `clap` CLI and `ratatui` TUI |

```
monitor --> engine state machine --> controller (token bucket / WinDivert)
config.toml (hot-reload) --^              ^-- IPC <-- bwp CLI / TUI
```

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## License

Dual licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
