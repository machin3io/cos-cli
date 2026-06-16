# TODO


## activate-previous — focus the previously-active window

wanted by the `fuzzylink` tool (`~/Archive/python/tools/fuzzylink/`): its `ctrl+e` keymap should focus the window that was active *before* the current one (the email/editor you switched away from to reach the floating terminal), then type a link into it. on X11/AwesomeWM this is `awful.client.focus.history.previous()`; the COSMIC/Wayland equivalent needs daemon support here. fuzzylink's Wayland branch is currently stubbed and just prints a reminder.

the daemon already has the pieces:
- it tracks `activated_at: Option<u64>` per window (`WindowInfo`), set on the `activated` state transition.
- `focused_window()` returns the window with the **max** `activated_at` on the active workspace.
- it can activate a handle via the toplevel manager — the pattern is the commented-out auto-activate block in `daemon.rs`: `manager.activate(&app.handle, &seat.0)` (seats live in `wl_state`; `process_unminimize_last` shows how to resolve the handle and flush).

plan:
1. `DaemonState::previous_window()` — like `focused_window()` but returns the **2nd**-highest `activated_at` (most recently activated window that isn't the current focused one). constrain to the active workspace to start (the common case: floating terminal over the email on the same workspace).
2. `PendingAction::ActivatePrevious` + a `do-activate-previous` socket command (mirror `do-unminimize` / `process_unminimize_last`): resolve the handle, `manager.activate(&handle, &seat.0)`, `conn.flush()`. return the activated `app_id|title` (or `none`) so fuzzylink can notify.
3. fuzzylink side: replace the stubbed Wayland branch with `cos-cli query do-activate-previous` → short sleep (activation is async, like the pass tool's PRE_SLEEP) → `wtype` the url.

caveat: confirm `wl_state.seats` is actually populated in the daemon (the commented auto-activate assumed `wl_state.seats.first()`).


## exclude transient windows from auto-maximize/unmaximize window count

transient/floating windows like the Tk picker (`pass-autotype`) and `gcr-prompter` (GPG password prompt) count as visible windows, triggering auto-unmaximize on the real tiled window when they appear, then auto-maximize again when they close. this causes noticeable flickering.

fix: add an app_id-based exclusion list to the config file (e.g. `~/.config/cos-cli/config.toml`). excluded app_ids would be skipped when counting visible windows in `process_auto_maximize`. known candidates: `Tk`, `gcr-prompter`.


## swap/reorder workspaces

swap the current workspace with the adjacent one (left or right), moving all windows along with it. e.g. if on workspace 3, swapping right would make workspace 3 take position 4 and workspace 4 take position 3, preserving all window layouts.

now possible with the daemon — query all windows on both workspaces, then use move-to to swap them. the daemon tracks per-window workspace assignments via handle IDs.

open question: does the ext_workspace_v1 protocol support reordering workspace positions natively, or do we have to move every window individually?

keybind plan: Super+Shift+Right / Super+Shift+Left.



## verify non-daemon commands still work after v3 migration

the original upstream commands (`info`, `move`, `activate`, `state`) predate our daemon work and relied on the v1 zcosmic_toplevel_info protocol for window discovery and state. after the v3 ext_foreign_toplevel_list migration, all state arrays in `info --json` are empty (confirmed — every window shows `"state":[]`). this likely means `activate`, `state`, and `move` (which use app index from `info`) may also be broken or degraded for non-daemon usage.

need to audit each original command and confirm whether they still function correctly without the daemon running, or whether the v3 migration has made the daemon a hard requirement for all commands, not just the new ones we added.


## foreign handle title events not forwarded after initial Done

after the v3 migration to `ext_foreign_toplevel_list`, the foreign handle's `Title` and `AppId` events that arrive after the first `Done` are stored in `foreign_toplevel_props` but never forwarded to the daemon. only the cosmic handle's title/app_id events update the daemon state.

this causes some windows (e.g. OrcaSlicer) to have an empty title in the daemon — the `focused` IPC command returns `OrcaSlicer|` with no title. the initial foreign handle `Done` fires before the `Title` event arrives, so the daemon gets an empty string.

fix: in `dispatch.rs`, the `ext_foreign_toplevel_handle_v1` handler for `Title` and `AppId` events should check if the foreign handle already has a cosmic handle (via `foreign_to_cosmic` map) and forward the update to the daemon via `on_title_changed` / `on_app_id_changed`. currently these post-Done events are silently dropped.

the `info --json` one-shot command is also affected — all windows show `"state":[]` because the activated state arrives via cosmic handle events that don't complete before the one-shot exits. this is inherent to the one-shot approach and not fixable without a longer roundtrip wait. the daemon path is the correct solution for consumers that need state.

autotyp3_cosmic2.py has been updated to query the daemon first (`focused` IPC), falling back to `info --json`. the python side also now accepts results where title is empty but app_id is present.


## autotyp3 sometimes gets the wrong or no match for tunnel

I think when I have ranger open on another workspace?
is its active window approach flawed? and can it be improved my using the daemon?


## improve daemon startup message with per-workspace window breakdown

the initial "3 windows on workspace 4" message is misleading — it reads as if all 3 windows are on workspace 4, when really it means "3 total windows, currently on workspace 4". the windows may be spread across multiple workspaces.

fix: print a per-workspace breakdown on startup, e.g. "workspace 1: 2 windows, workspace 4: 1 window" or similar. the daemon already tracks per-window workspace assignments, so the info is available.


## investigate dock/panel unhide_delay setting

Seems to be new from recent install
this or related may help with the slightly annoying dock animations when switching workspaces with different gaps
