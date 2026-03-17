# Project Overview

- **What the system does**: System-level network traffic shaper that dynamically throttles non-critical applications when priority apps are active.
- **Target users**: Users experiencing network contention during critical tasks (e.g., Discord calls, Zoom meetings, gaming).
- **Core problem it solves**: Background downloads tanking connection quality; existing solutions require manual intervention or expensive commercial software.
- **Main functional domains**: Process/network monitoring, Traffic shaping/throttling, Configuration management, IPC and CLI/TUI interfacing.

# Tech Stack

- **Languages**: Rust (edition 2024, version >= 1.85)
- **Frameworks**:
  - Tokio (Async runtime)
  - Clap (CLI parsing)
  - Ratatui / Crossterm (TUI dashboard)
- **Runtime**: Native binary (Windows/Linux)
- **Tooling**: Cargo, rustfmt, clippy
- **Infrastructure**: Native OS networking APIs (WinDivert on Windows, tc on Linux)
- **Database**: None (state is held in memory, config in TOML)
- **State management**: `DashMap` for lock-free caching, `ArcSwap` for zero-cost config hot-reloading.
- **UI libraries**: Ratatui for TUI.

# Project Structure

```
.
├── Cargo.toml            # Workspace configuration
├── config.toml           # Default configuration file
├── crates/
│   ├── bwp-cli/          # CLI and TUI frontend
│   ├── bwp-controller/   # Traffic shaping engine (WinDivert on Windows, tc on Linux)
│   ├── bwp-core/         # Shared types, error handling, config models
│   ├── bwp-engine/       # Central state machine, orchestration, IPC server
│   └── bwp-monitor/      # Process & network connection monitoring
└── README.md             # Project documentation
```

- **bwp-core**: Holds `AppConfig`, lock-free `ConfigManager`, IPC commands, error hierarchy.
- **bwp-monitor**: Traits and implementations for detecting active processes and network connections.
- **bwp-controller**: Abstract `TrafficController` and `ThrottleCommand`, with OS-specific implementations (e.g., WinDivert on Windows, tc on Linux).
- **bwp-engine**: Orchestrates the system. `Engine` handles events from `bwp-monitor` and applies limits via `bwp-controller`. Also hosts the IPC server.
- **bwp-cli**: The user interface. Sends IPC commands to the daemon. Contains the `ratatui` UI.

# Architecture

- **System design pattern**: Modular / Clean Architecture with a decoupled background daemon and frontend CLI/TUI.
- **Data flow**:
  - Monitor -> Engine: Network events (ConnectionOpened, ConnectionClosed) flow into the Engine's MPSC channel.
  - Config file -> Engine: Hot-reloaded `AppConfig` via `notify` and `ArcSwap`.
- **State flow**:
  - Engine maintains `priority_pids`, `normal_pids`, and `ThrottleState` (Normal, PriorityMode).
  - Updates occur based on network events and config matching.
- **API flow**:
  - CLI -> IPC Client -> IPC Server -> Engine -> IPC Response.
- **Separation of concerns**:
  - `bwp-monitor` only observes.
  - `bwp-controller` only enforces limits.
  - `bwp-engine` makes decisions.
  - `bwp-cli` handles user interaction.
  - `bwp-core` provides common types and IPC contracts.

# Key Conventions

- **Naming rules**: Standard Rust conventions (`snake_case` for variables/functions, `CamelCase` for structs/enums).
- **Folder conventions**: Workspace model with logic split into `crates/*`.
- **File suffix rules**: `.rs` for Rust, `.toml` for config.
- **Component patterns**: Heavy use of Traits for OS-specific implementations (e.g., `ProcessMonitor`, `TrafficController`).
- **Error handling patterns**: `thiserror` for library error types, `anyhow` for application-level error context in binaries. `BwpResult<T>` alias.
- **Dependency boundaries**: Frontend crates (`bwp-cli`) only depend on `bwp-core` (for IPC), never directly on engine logic or low-level monitors. Engine depends on core, monitor, and controller.

# Important Files

- **Entry points**:
  - `crates/bwp-cli/src/main.rs` (CLI/TUI binary, spawns daemon)
  - `crates/bwp-engine/src/main.rs` (Daemon binary)
- **Bootstrapping logic**: `bwp-cli` spawns `bwp-engine.exe` via `std::process::Command` in the background.
- **Environment config**: `config.toml` loaded by `ConfigManager`.
- **Core services**: `crates/bwp-engine/src/state_machine.rs` (`Engine::run_event_loop`).
- **Domain logic**: `crates/bwp-core/src/config.rs` (`AppConfig::effective_throttle_limit`), `crates/bwp-engine/src/state_machine.rs` (State transitions).

# Dependencies

- **tokio**: Async runtime for event loops, IPC, and concurrent tasks. Removing it breaks all async logic.
- **serde / toml**: Serialization and parsing for `config.toml`. Removing breaks configuration.
- **clap**: CLI argument parsing. Removing breaks the CLI interface.
- **ratatui / crossterm**: TUI dashboard rendering. Removing breaks the `top` command.
- **dashmap**: Lock-free concurrent hash map for caching PID to Executable Name. Removing degrades performance/concurrency in the engine.
- **arc-swap**: Atomic lock-free pointer swapping for hot-reloading configuration. Removing requires locks (RwLock/Mutex) leading to contention in fast-paths.
- **notify**: File system watching for hot-reloading `config.toml`. Removing breaks dynamic config updates.
- **windows**: Native Windows APIs for WinDivert, ETW, process management. Removing breaks Windows support.

# Development Workflow

- **Install**: Rust >= 1.85.
- **Dev mode**: `cargo run --bin bwp -- start --foreground`
- **Build**: `cargo build --workspace`
- **Test**: `cargo test --workspace`
- **Lint**: `cargo clippy --workspace --all-targets -- -D warnings`
- **Format**: `cargo fmt --all`
- **Deployment steps**: Compile release binaries (`cargo build --release`), require Administrator/root privileges to run the daemon.

# Do NOT Section

- **Files that must never be edited**: Avoid modifying `Cargo.lock` directly.
- **Generated files**: Build artifacts in `target/`.
- **Build artifacts**: `target/` directory.
- **Architectural boundaries that must not be violated**: `bwp-cli` must not directly invoke `bwp-controller` or `bwp-engine` functions; all communication must go through IPC. `bwp-engine` must not perform UI rendering.

# Task Execution Rules

- When given a task, do NOT re-explore the project
- Trust this `AGENTS.md` as the single source of truth
- Only modify relevant modules
- Do not refactor unrelated parts
- Do not change architecture without explicit instruction
