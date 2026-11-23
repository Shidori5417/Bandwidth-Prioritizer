# Bandwidth-Prioritizer (bwp)

[![CI](https://github.com/Shidori5417/Bandwidth-Prioritizer/actions/workflows/ci.yml/badge.svg)](https://github.com/Shidori5417/Bandwidth-Prioritizer/actions/workflows/ci.yml)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)

Network traffic shaper that **throttles non-critical apps while priority apps (Discord, Zoom, ...) are running**, and restores full bandwidth when they close.

## Status: prototype

This project is an early prototype. **By default it does not shape any network traffic**: it detects priority apps and decides what should be throttled, but the packet-shaping backend is opt-in and experimental.

| Area | State |
|---|---|
| Engine state machine (Normal / PriorityMode), per-PID connection tracking, process-name resolution | Works (unit tested) |
| Connection monitoring on Windows (polling `GetExtendedTcpTable` / `GetExtendedUdpTable`, IPv4 and IPv6, 500 ms by default or `[general] poll_interval_ms`) | Implemented, so Priority Mode engages on its own. The snapshot diffing, byte-order conversion and buffer resize loop are unit tested on Linux; the FFI itself is only compile-checked by Windows CI and has not been run on a real machine by the author. Listening sockets and loopback-only sockets are ignored |
| Config hot reload | Works. After a reload the engine re-evaluates every tracked PID against the new `critical_apps`/profiles/limit (unit tested) |
| IPC (named pipe / Unix socket), `bwp` CLI and TUI, profiles, add/remove priority apps | Works |
| Windows Service install/start/stop | Works (Windows) |
| Token-bucket limiter, delay queue, flow parsing, port-to-PID mapping | Implemented and unit tested (platform independent) |
| **Packet shaping (WinDivert)** | **Experimental, opt-in, OFF by default, untested on real Windows.** Needs a special build and a config switch, see [Experimental packet shaping](#experimental-packet-shaping). The default build never opens a WinDivert handle |
| Connection monitoring via ETW | **Not implemented** (the listener reports an error and the monitor uses polling) |
| Linux (`tc htb`) | **Not implemented.** The controller is a no-op. The engine itself only runs on Windows; Linux builds and tests exist for development and CI |

In short: the control plane and connection monitoring are built; the data plane (limiting packets) exists only as an experimental opt-in. See [`progress.md`](progress.md) for the roadmap.

## Features

Intended behaviour (see [Status](#status-prototype) for what is actually implemented; actual packet limiting requires the experimental shaper):

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

`build_release.ps1` packages `bwp.exe`, `bwp-engine.exe` and the config into `target\bwp-windows-v<version>.zip` (the version comes from `Cargo.toml`). The default zip contains no WinDivert files. `.\build_release.ps1 -WithWinDivert` builds the experimental shaper instead, see below.

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

Edits are picked up automatically (reloaded once the file has been quiet for 300 ms); no restart needed. `[shaping]` is the exception: it is read at startup.

## Experimental packet shaping

Off by default. The default `bwp-engine.exe` never captures a packet, even if you set the config key. To try it you need all of:

1. A build with the Cargo feature: `.\build_release.ps1 -WithWinDivert` (downloads the official [WinDivert 2.2.2](https://github.com/basil00/WinDivert/releases/tag/v2.2.2) release, verifies its SHA-256, links against it and puts `WinDivert.dll`, `WinDivert64.sys` and the WinDivert license in `bwp-windows-v<version>-windivert.zip`). To build by hand: set `WINDIVERT_PATH` to the SDK's `x64` folder and run `cargo build --release --features bwp-engine/experimental-windivert`.
2. `[shaping] experimental_windivert = true` in `config.toml`, then restart the engine.
3. Administrator rights, and the two WinDivert files next to `bwp-engine.exe`. WinDivert is LGPL-3.0 / GPL-2.0 licensed; `WinDivert.dll` is linked dynamically.

How it works: while at least one process is throttled, a WinDivert NETWORK-layer handle with the filter `!loopback and (tcp or udp)` (the `pid` field does not exist at that layer) captures packets. Each packet is attributed to a PID through the connection table (local port to PID, refreshed every second and when an unknown flow shows up). **Every packet that does not belong to a throttled PID, including flows of unknown owner, is re-injected immediately.** Packets of throttled PIDs go through a per-PID token bucket and are delayed in a queue (at most 500 ms and 8 MiB overall), and dropped only beyond that. Capture runs on a dedicated blocking thread, the delay queue on a timer thread, with no busy loops. When the last throttle is lifted, on `ClearAll`, on any error and on shutdown, the handle is shut down first (so the driver stops diverting and traffic flows again), queued packets are flushed, and the handle is closed.

Caveats: never run on real hardware by the author, only compile-checked in CI; port-based attribution can miss flows that start between table refreshes and cannot tell apart UDP sockets sharing a port; inbound limiting only works through TCP back-pressure; the driver may be refused by Secure Boot/HVCI policies; antivirus tools may flag WinDivert. Details in the module docs of `crates/bwp-controller/src/windows/windivert.rs`.

## Architecture

| Crate | Role |
|---|---|
| `bwp-core` | Config models and hot-reload, IPC protocol (named pipe on Windows, Unix socket on Linux), shared types and errors |
| `bwp-monitor` | Process and network connection monitoring trait; Windows implementation polls the IP Helper connection tables and diffs snapshots into events |
| `bwp-controller` | `TrafficController` trait, token bucket and delay-queue shaper; the Windows backend is a no-op unless the experimental WinDivert feature is enabled, the Linux (`tc htb`) backend is a no-op placeholder |
| `bwp-engine` | Daemon: state machine (Normal / PriorityMode), IPC server, service entry |
| `bwp-cli` | `bwp` binary: `clap` CLI and `ratatui` TUI |

```
monitor (IP Helper polling) --> engine state machine --> controller (no-op, or experimental WinDivert shaper)
config.toml (hot-reload) --^              ^-- IPC <-- bwp CLI / TUI
```

## Development

CI runs on Ubuntu and Windows (stable and nightly) with `RUSTFLAGS="-D warnings"`. To reproduce locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
```

The experimental WinDivert feature is part of `--all-features`, so on Windows CI downloads the WinDivert SDK (checksum verified) and sets `WINDIVERT_PATH`; to run `--all-features` yourself on Windows, do the same. On Linux the feature compiles to nothing.

Lint levels are configured in `[workspace.lints]` in `Cargo.toml` (`clippy::all`, `unwrap_used` and `expect_used` outside tests, `missing_docs`).

## License

Dual licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
