# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

cos-cli is a Rust CLI tool for managing windows and workspaces on the COSMIC Desktop Environment (Wayland). It communicates directly with the Wayland compositor via `wayland-client` and COSMIC-specific protocols.

## Build Commands

```bash
cargo build --release        # build release binary
cargo build                  # build debug binary
cargo check                  # type-check without building
cargo clippy                 # lint
```

There are no tests in this project.

## Architecture

The entire codebase is two files:

- **`src/main.rs`** — CLI argument parsing (`pico-args`), `AppState` struct, command execution logic (`info`, `move`, `activate`, `state`), custom error type, hand-rolled JSON output (no serde).
- **`src/dispatch.rs`** — Wayland `Dispatch` trait implementations for all protocol interfaces (registry, output, seat, workspace, toplevel). This is the event-handling layer that populates `AppState`.

### Flow

1. Connect to Wayland display, create event queue
2. Discover globals via `wl_registry`, bind to needed protocols
3. Run event roundtrips to populate `AppState` (apps, workspaces, outputs, seats)
4. Execute the requested subcommand against the populated state
5. Flush connection to send any requests (move, activate, state changes)

### Key types

- `AppState` — holds all discovered Wayland objects (apps, workspaces, outputs, seats)
- `App` — individual window handle with title, app_id, outputs, and state
- `AppFinderArgs` trait — shared interface for finding apps by index or partial app_id, with optional wait; implemented by `MoveArgs` and `StateArgs`

### Dependencies

- `wayland-client` / `wayland-protocols` — core Wayland protocol bindings
- `cosmic-protocols` (git dep from pop-os) — COSMIC-specific toplevel and workspace protocols
- `pico-args` — minimal argument parser

## CLI Subcommands

- **`info`** — list apps, workspaces, outputs, seats (supports `--json`)
- **`move`** — move app to workspace (by `--app-id` partial match or `--index`)
- **`activate`** — bring app to foreground (by `--index`)
- **`state`** — change window state: maximize, minimize, fullscreen, sticky (by `--app-id` or `--index`)

Both `move` and `state` support `--wait <SECONDS>` to poll for an app to appear before acting.
