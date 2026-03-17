# Bandwidth-Prioritizer (bwp)

> 🚀 System-level network traffic shaper that **dynamically throttles non-critical applications** when priority apps are active.

[![CI](https://github.com/Shidori5417/Bandwidth-Prioritizer/actions/workflows/ci.yml/badge.svg)](https://github.com/Shidori5417/Bandwidth-Prioritizer/actions/workflows/ci.yml)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

---

## 🎯 Problem

You're in a Discord call or Zoom meeting, and a background download tanks your connection. Existing solutions require manual intervention or expensive commercial software.

**bwp** solves this automatically:
- Detects when critical apps (Discord, Zoom, VS Code Live Share, etc.) are active
- Dynamically throttles all *other* processes' bandwidth
- Restores full bandwidth gracefully when priority apps close

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────┐
│                Bandwidth-Prioritizer            │
│                                                 │
│  ┌─────────────┐    ┌──────────────────────┐   │
│  │ Process     │───▶│  Traffic Controller  │   │
│  │ Monitor     │    │  (WFP / eBPF / tc)   │   │
│  └─────────────┘    └──────────┬───────────┘   │
│         │                      │               │
│  ┌─────────────┐    ┌──────────▼───────────┐   │
│  │  Config     │    │   Throttle Engine    │   │
│  │  Manager    │    │   (Token Bucket)     │   │
│  └─────────────┘    └──────────────────────┘   │
│         │                                       │
│  ┌─────────────┐                               │
│  │  CLI / TUI  │                               │
│  └─────────────┘                               │
└─────────────────────────────────────────────────┘
```

## 📦 Crate Structure

| Crate | Description |
|---|---|
| `bwp-core` | Shared types, error hierarchy, configuration models |
| `bwp-monitor` | Process & network connection monitoring |
| `bwp-controller` | Traffic shaping (WFP on Windows, tc/eBPF on Linux) |
| `bwp-cli` | CLI (`clap`) and TUI (`ratatui`) interface |

## 🚀 Quick Start (Windows)

### Prerequisites

- **Rust** ≥ 1.85 (2024 edition)
- **Windows**: Administrator privileges (for WinDivert & Windows Service Management)

### Building the Release

```bash
cargo build --release
```
*(Note: `WinDivert.dll` is automatically copied to your output folder during the build process).*

### 🛠️ OS-Level Service Management

Bandwidth Prioritizer is designed to run silently as a background Windows Service. Run an **elevated PowerShell (Administrator)** window:

```bash
# 1. Install the engine as an Auto-Start Windows Service
.\target\release\bwp.exe service install

# 2. Start the background service
.\target\release\bwp.exe service start

# Check the background daemon's status
.\target\release\bwp.exe status
```

To stop or remove the service later, simply run `bwp service stop` and `bwp service uninstall`.

### 🖥️ Interacting via CLI / TUI

Once the Windows Service is running, any standard user can interact with the engine.

```bash
# 📊 Open the Live Ratatui Dashboard (TUI)
.\target\release\bwp.exe top

# 🎮 Switch dynamic profiles
.\target\release\bwp.exe profile gaming
.\target\release\bwp.exe profile streaming

# ➕ Add a new critical application on-the-fly
.\target\release\bwp.exe add "zoom.exe"
```

### Configuration (`config.toml`)

Edit `config.toml` in the project root to define your limits and profiles. The daemon implements hot-reloading! You don't need to restart the service when you modify the `.toml` file; the engine detects changes seamlessly.

---

## 🧪 Testing & Development

```bash
# Run all asynchronous and integration tests
cargo test --workspace

# Linting
cargo clippy --workspace --all-targets -- -D warnings
```

## 📄 License

Dual licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
