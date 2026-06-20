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

The codebase is three files:

- **`src/main.rs`** — CLI argument parsing (`pico-args`), `AppState` struct, command execution logic (`info`, `move`, `activate`, `state`), custom error type, hand-rolled JSON output (no serde), and config-file readers (gaps, auto-maximize exclusions, etc.).
- **`src/dispatch.rs`** — Wayland `Dispatch` trait implementations for all protocol interfaces (registry, output, seat, workspace, toplevel). This is the event-handling layer that populates `AppState`.
- **`src/daemon.rs`** — the long-running daemon: window-identity tracking via Wayland handle IDs, IPC over a unix socket, and the pending-action queue that drives auto-maximize / move / minimize behavior (`process_auto_maximize` etc.).

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

## Runtime config & deployment

Runtime config lives in `~/.config/cosmic/cos-cli/` (`gaps`, `corner_radius`, `zero_gap_radii`, `auto_maximize_exclude`, …). The daemon self-generates each file with defaults if missing.

These files are **not** edited live on the target machine. They are version-controlled in the **tools** repo at `~/Archive/python/tools/install/data/home/.config/cosmic/cos-cli/`, which doubles as the sync "golden dir" (`GOLDEN_DIR`). COSMIC/cos-cli only runs on **atlas**; dev usually happens on **eremite** (an X machine), where `~/.config/cosmic` exists only in that golden snapshot. To change runtime behavior, edit/create the file in the tools repo's `install/data` tree and commit there (`install - …`) — a sync push eremite→atlas writes it into atlas's live `~/.config/cosmic/cos-cli/`. Seed any new file with the daemon's self-generated defaults so the push doesn't clobber them.

- **`auto_maximize_exclude`** — one `app_id` (or `app_id:title_substring`) per line; matches `app_id` exactly. The daemon auto-maximizes the sole visible window on a 0-gap workspace, and excluded windows are dropped from the visible count *before* that check (so a lone excluded window stays as-is). Floating windows all share `app_id` `Floating` (`kitty --class Floating …`), so one `Floating` line covers them.
