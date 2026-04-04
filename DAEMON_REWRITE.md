# Daemon Window Tracking Rewrite


## Problem

the daemon uses `app_id + title` to identify windows, but:
- titles change constantly (terminals update on every command, ranger, etc.)
- multiple windows share the same app_id (e.g. several CosmicTerm instances)
- activation-based workspace reassignment causes spam because COSMIC rapidly toggles activated state on focused windows
- without activation tracking, windows drift to wrong workspaces

this results in windows being misidentified, duplicated, or assigned to the wrong workspace.


## Root Cause

the daemon currently:
1. takes a snapshot of all apps from `wl_state.apps` on every event tick
2. diffs against `known_keys` (app_id + title pairs) to detect new/removed windows
3. uses activated state transitions to reassign workspace — but COSMIC sends activated events continuously for focused windows, causing spam and incorrect moves

the fundamental issue is that `app_id + title` is not a stable window identity.


## Fix: Use Wayland Handle IDs

each `ZcosmicToplevelHandleV1` has a unique protocol object ID that persists for the entire daemon connection lifetime. this is the correct identity key.


### 1. key the window HashMap on Wayland handle ID

extract the protocol object ID from the handle (e.g. via `ObjectId` or by deriving a key from the handle). use this as the HashMap key instead of a synthetic counter.


### 2. move tracking into dispatch handlers

instead of syncing by comparing snapshots in the event loop, handle events directly:

- **toplevel created** (`zcosmic_toplevel_info_v1::Event::Toplevel`) → add window to daemon state, assign to active workspace
- **toplevel closed** → remove window from daemon state
- **title changed** (`zcosmic_toplevel_handle_v1::Event::Title`) → update title on existing window (no identity confusion)
- **app_id changed** (`zcosmic_toplevel_handle_v1::Event::AppId`) → update app_id on existing window
- **state changed** (`zcosmic_toplevel_handle_v1::Event::State`) → update minimized/activated/etc. track minimize timestamps

this requires passing the daemon state (`Arc<Mutex<DaemonState>>`) into the dispatch handlers, likely via the wayland-client data mechanism (the `()` in `Dispatch<..., ()>` can be replaced with the shared state).


### 3. workspace assignment rules

only assign workspace in these cases:
- **new window appears** → assign to current active workspace
- **`move-window` IPC** → explicit reassignment by command
- **`set-workspace` IPC** → update active workspace tracking (don't touch existing windows)

**no activation-based reassignment** — this was the source of the spam and incorrect tracking. activation events are only used to update the window's state field, not its workspace.


### 4. simplified event loop

the main loop becomes minimal:
```rust
loop {
    event_queue.blocking_dispatch(&mut wl_state)?;
    // workspace tracking from ext_workspace events (already works)
    // window tracking now happens entirely in dispatch handlers
}
```

no more snapshot diffing, no more known_keys, no more comparing app lists.


## Files to Modify

- **`src/daemon.rs`** — rewrite DaemonState to key on handle ID, remove snapshot diffing from event loop, pass state to dispatch
- **`src/dispatch.rs`** — update toplevel event handlers to write directly to DaemonState. need to change the dispatch data type from `()` to the shared state
- **`src/main.rs`** — minimal changes, AppState might need the daemon state reference for dispatch


## Risks

- the wayland-client dispatch data mechanism might make it awkward to pass `Arc<Mutex<DaemonState>>` — may need a wrapper struct
- `blocking_dispatch` borrows `AppState` mutably, so daemon state access during dispatch needs to go through a separate `Arc<Mutex<>>`, not through AppState itself
- the existing non-daemon commands still use the same dispatch handlers, so changes to dispatch.rs need to be compatible with both modes (daemon and standalone)


## Verification

after the rewrite:
- `cos-cli query` should show correct per-workspace window assignments
- opening/closing windows should be tracked instantly without spam
- title changes should not cause window duplication or loss
- `visible-on` should return accurate counts
- `skip-empty` cycling should work reliably
- minimize/unminimize should work via daemon
