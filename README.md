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

## 🚀 Quick Start

### Prerequisites

- **Rust** ≥ 1.85 (2024 edition)
- **Windows**: Administrator privileges (for WFP)
- **Linux**: Root or `CAP_NET_ADMIN` capability (for tc/eBPF)

### Build

```bash
cargo build --workspace
```

### Run

```bash
# Start the daemon (foreground mode)
cargo run --bin bwp -- start --foreground

# Check status
cargo run --bin bwp -- status

# Add a critical app
cargo run --bin bwp -- add discord

# Switch profile
cargo run --bin bwp -- profile gaming
```

### Configuration

Edit `config.toml` in the project root. See the file for detailed inline documentation.

## 🧪 Testing

```bash
# Run all tests
cargo test --workspace

# Run with output
cargo test --workspace -- --nocapture
```

## 🔧 Development

```bash
# Format code
cargo fmt --all

# Run clippy lints
cargo clippy --workspace --all-targets -- -D warnings
```

## 📄 License

Dual licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
