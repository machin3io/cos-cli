# cos-cli

A CLI tool for managing windows and workspaces on the COSMIC Desktop Environment (Wayland).

> **Note:** This is a third-party, unofficial tool. It is not affiliated with System76 or the official COSMIC project.

> **Fork note:** This is a `machin3` fork (`workspace_control` branch). The upstream repo is [estin/cos-cli](https://github.com/estin/cos-cli).

## Features

#### Upstream
- **List Information**: View active applications, workspaces (with active state), and outputs.
- **Window Management**: Move applications between workspaces by their App ID.
- **Activate Application**: Bring a specific application to the foreground.
- **Window State**: Set window state (maximize, minimize, fullscreen, sticky).

#### Fork
- **Workspace Switching**: Switch directly to any workspace by name, or cycle with next/prev (with wrapping and optional dynamic workspace exclusion).
- **Skip-Empty Cycling**: Cycle through only workspaces with visible windows (requires daemon).
- **Workspace Toggle**: Switch back and forth between the last two workspaces via statefile history.
- **Move Focused Window**: Move the currently focused app to any workspace.
- **Minimize/Unminimize**: Minimize the focused app with per-workspace history, unminimize the last minimized app on the current workspace.
- **Per-Workspace Gaps**: Automatically apply window gap settings per workspace, with keybinds to adjust on the fly. Corner radii are automatically removed on zero-gap workspaces.
- **Daemon Mode**: Persistent background process that tracks windows across workspaces via Wayland handle IDs, enabling accurate focus detection, skip-empty cycling, and per-workspace queries.

## Daemon (recommended)

The daemon (`cos-cli daemon`) is a persistent background process that maintains a live Wayland connection, tracking all windows by their unique protocol handle IDs. It is strongly recommended for the best experience.

**With the daemon**, commands like `move-to`, `minimize`, and `unminimize` are executed by the daemon itself using stable handle IDs — no ambiguity when multiple windows share the same app ID or title. The daemon also enables:
- **Skip-empty cycling** — knows which workspaces have visible windows
- **Auto-maximize** — automatically maximizes the sole window on zero-gap workspaces, and undoes it when a second window appears
- **Accurate focus detection** — tracks activation timestamps to resolve COSMIC's multiple-activated-window quirk
- **Verbose logging** — all window events, focus changes, workspace switches, and command actions in one place

**Without the daemon**, basic commands still work (workspace switching, gap adjustment, toggle) but `move-to` falls back to less reliable matching, `minimize` and `unminimize` are unavailable, and features like skip-empty cycling and auto-maximize are disabled.

Start the daemon on login or as a systemd user service:
````console
cos-cli daemon
````

## Installation
Ensure you have the Rust toolchain installed.

````console
cargo install --git https://github.com/machin3io/cos-cli
````

For the upstream version (without workspace switching, minimize history, etc.):

````console
cargo install --git https://github.com/estin/cos-cli
````

## Usage
````console
cos-cli [COMMAND]
````

### Commands

#### `info`
List all available apps, workspaces, outputs and seats, including the state of each app window. Workspaces show their active state.
````console
cos-cli info
````
Example output:
````
Apps:
	[0] firefox (title: Gemini - Mozilla Firefox, state: [activated])
	[1] org.wezfurlong.wezterm (title: cos-cli, state: [maximized])
Workspaces:
	[0] Group
		Workspace: 1
		Workspace: 2 (active)
		Workspace: 3
Outputs:
	[0] Output: eDP-1
Seats:
	[0] Seat: seat0
````

With `--json` option it will output all info in JSON format.
````console
cos-cli info --json
````
Example output:
````json
{"apps":[{"index":0,"app_id":"firefox","title":"Gemini - Mozilla Firefox","state":["activated"]},{"index":1,"app_id":"org.wezfurlong.wezterm","title":"cos-cli","state":["maximized"]}],"workspaces":[{"index":0,"workspaces":[{"name":"1","active":false},{"name":"2","active":true},{"name":"3","active":false}]}],"outputs":[{"index":0,"name":"eDP-1"}],"seats":[{"index":0,"name":"seat0"}]}
````

Using `jq` to find app index by pattern and activate app
````console
cos-cli activate -i $(cos-cli info --json | jq '.apps[] | select(.app_id | test("wezterm")) | .index')
````

#### `workspace`
Switch to a workspace by name, or toggle between the last two workspaces.
````console
cos-cli workspace -w 2
cos-cli workspace -w 10
cos-cli workspace -w 3 -g 0
cos-cli workspace --toggle
````
Arguments:
*   `-w, --workspace <NAME>`
    The name of the target workspace
*   `-g, --workspace-group <INDEX>`
    The workspace group index from 'info' command (optional, only needed for multi-monitor setups)
*   `--toggle`
    Switch to the previous workspace
*   `--next`
    Switch to the next workspace (wraps around from last to first)
*   `--prev`
    Switch to the previous workspace (wraps around from first to last)
*   `--no-dynamic`
    Auto-detect the highest workspace by ignoring the trailing dynamic workspace COSMIC adds after pinned ones (use with `--next`/`--prev`)
*   `--max <N>`
    Explicitly set the highest workspace number for `--next`/`--prev` (overrides `--no-dynamic`)

Without `--no-dynamic` or `--max`, `--next`/`--prev` cycle through all workspaces including any dynamic ones.

##### Why this exists

COSMIC uses a dynamic workspace model where pinned (sticky) workspaces always have one additional dynamic workspace appended after the last pinned one. If you pin 10 workspaces, COSMIC creates an 11th.

COSMIC's built-in keyboard shortcuts only support Super+1 through Super+9 for direct workspace switching. There is no native binding for the 10th workspace. The "switch to last workspace" shortcut switches to the very last workspace, which in this case is the 11th (the dynamic one), not the 10th.

`cos-cli workspace -w 10` solves this by switching directly to the workspace named "10", regardless of how many workspaces exist.

##### Workspace toggle

`cos-cli workspace --toggle` switches between the last two workspaces. Every time `cos-cli workspace` switches to a new workspace, it saves the current workspace name to `/tmp/cos-cli-last-workspace`. The `--toggle` flag reads this file and switches back to the saved workspace.

**Important:** The toggle only tracks switches made through cos-cli. If you switch workspaces using the COSMIC top bar or other means, the statefile won't be updated and toggle won't know about it. For reliable toggle behavior, route all workspace switching through cos-cli keybinds:

````
Super+1 through Super+9   →  cos-cli workspace -w 1  through  -w 9
Super+0                   →  cos-cli workspace -w 10
Super+Escape              →  cos-cli workspace --toggle
Super+Right               →  cos-cli workspace --next --no-dynamic
Super+Left                →  cos-cli workspace --prev --no-dynamic
Super+Alt+Right           →  cos-cli workspace --next --skip-empty --no-dynamic
Super+Alt+Left            →  cos-cli workspace --prev --skip-empty --no-dynamic
````

#### `minimize`
Minimize the currently focused app with per-workspace history tracking. **Requires the daemon.**
````console
cos-cli minimize
````

The daemon identifies the focused window by its Wayland handle ID and minimizes it, tracking it in per-workspace history.

#### `unminimize`
Restore the most recently minimized app on the current workspace. **Requires the daemon.**
````console
cos-cli unminimize
````

The daemon tracks minimize history per workspace and restores the most recently minimized app. Relies on handle IDs for accurate window identification — earlier statefile-based approaches couldn't uniquely identify windows when multiple instances of the same app were open (e.g. multiple terminals).

````
Super+N        →  cos-cli minimize
Super+Ctrl+N   →  cos-cli unminimize
````

#### `gap`
Adjust window gaps on the current workspace.
````console
cos-cli gap --increase
cos-cli gap --decrease
````
Arguments:
*   `--increase`
    Increase the outer gap by 10
*   `--decrease`
    Decrease the outer gap by 10

Gap settings are stored per workspace in `~/.config/cosmic/cos-cli/gaps`. The file is plain text, one entry per line:

````
# per-workspace gaps: workspace:inner,outer
# "default" is used as fallback for unlisted workspaces
default:0,30
1:0,40
3:0,0
5:0,30
````

When switching workspaces (via any `cos-cli workspace` command), the gap for the target workspace is automatically applied by writing to `~/.config/cosmic/com.system76.CosmicTheme.Dark/v1/gaps`, which COSMIC hot-reloads. Corner radii are also adjusted: zero on zero-gap workspaces, restored to the configured value otherwise (see `zero_gap_radii` and `corner_radius` config).

On zero-gap workspaces with a single visible window, the window is automatically maximized to remove the remaining border and rounded corners. When a second window appears (new window, unminimize, move-to), only auto-maximized windows are unmaximized — manually maximized windows are left untouched. This behavior can be disabled via the `auto_maximize` config file.

````
Super+Ctrl+Shift+=  →  cos-cli gap --increase
Super+Ctrl+-        →  cos-cli gap --decrease
````

## Configuration

All config files live in `~/.config/cosmic/cos-cli/`:

*   **`gaps`** — per-workspace gap settings (inner,outer). Created automatically with defaults on first use.
*   **`auto_maximize`** — `true` (default) or `false`. Controls whether a sole visible window on a zero-gap workspace is automatically maximized.
*   **`zero_gap_radii`** — `true` (default) or `false`. When enabled, window corner radii are set to zero on workspaces with zero outer gaps, and restored to the configured radius when gaps are non-zero. Only writes the theme file when the value actually changes.
*   **`corner_radius`** — the corner radius value (default: `8`) to restore when gaps are non-zero. Adjustable by editing the file.

#### `move`
Move an application to a specific workspace.
````console
cos-cli move --app-id <ID> --workspace <NAME>
````
Arguments:
*   `-a, --app-id <ID>`
    The Application ID (partial match, case-insensitive)
*   `-i, --index <INDEX>`
    The Application index from 'info' command
*   `-w, --workspace <NAME>`
    The name of the target workspace
*   `-g, --workspace-group <INDEX>`
    The workspace group index from 'info' command (optional)
*   `-o, --output-index <INDEX>`
    The output index from 'info' command (optional)
*   `--wait <SECONDS>`
    Wait for the app to appear (optional, only for --app-id)

#### `activate`
Activate an application.
````console
cos-cli activate --index <INDEX>
````
Arguments:
*   `-i, --index <INDEX>`
    The Application index from 'info' command
*   `-s, --seat <INDEX>`
    The Seat index from 'info' command (optional)

#### `state`
Set the state of an application's window (e.g., maximize, minimize, fullscreen, sticky).
````console
cos-cli state (--app-id <ID> | --index <INDEX>) [--wait <SECONDS>] [--maximize|--unmaximize] [--minimize|--unminimize] [--fullscreen|--unfullscreen] [--sticky|--unsticky]
````
Arguments:
*   `-a, --app-id <ID>`
    The Application ID (partial match, case-insensitive)
*   `-i, --index <INDEX>`
    The Application index from 'info' command
*   `--wait <SECONDS>`
    Wait for the app to appear (optional, only for --app-id)
*   `--maximize`
    Maximize the application window
*   `--unmaximize`
    Unmaximize the application window
*   `--minimize`
    Minimize the application window
*   `--unminimize`
    Unminimize the application window
*   `--fullscreen`
    Set the application window to fullscreen
*   `--unfullscreen`
    Unset the application window from fullscreen
*   `--sticky`
    Make the application window sticky (visible on all workspaces)
*   `--unsticky`
    Unset the application window from being sticky

Examples:
````console
cos-cli state -i 0 --maximize
cos-cli state --app-id firefox --sticky --fullscreen --wait 5
cos-cli state -i 1 --unminimize
````

#### `daemon`
Start the background daemon for persistent window/workspace tracking.
````console
cos-cli daemon
cos-cli daemon --verbose
cos-cli daemon --restart
````
Arguments:
*   `--verbose`
    Log window events, focus changes, and workspace switches to stdout
*   `--log`
    Write all debug output to `$XDG_RUNTIME_DIR/cos-cli.log` (can be combined with `--verbose`)
*   `--restart`
    Kill the existing daemon before starting a new one

The daemon maintains a persistent Wayland connection and tracks all windows using unique Wayland handle IDs — which workspace they're on, their state (minimized, activated, etc.), title, and focus history. It listens on `$XDG_RUNTIME_DIR/cos-cli.sock` for IPC queries from other cos-cli commands.

Each window is identified by its Wayland protocol object ID, which is stable for the lifetime of the daemon. This avoids the ambiguity of matching by app_id or title, which are not unique (e.g. multiple terminal windows). Windows are assigned to the workspace that was active when they first appeared. Focus is tracked with activation timestamps, so when multiple windows report as activated (a COSMIC quirk), the most recently focused one on the active workspace is used.

`minimize` and `unminimize` require the daemon for accurate window identification via handle IDs. `move-to` queries the daemon when available, falling back to direct Wayland state when it's not.

#### `query`
Query the daemon's tracked state.
````console
cos-cli query                    # full state (windows + active workspace)
cos-cli query ping               # check if daemon is alive
cos-cli query active-workspace   # current workspace name
cos-cli query focused            # currently focused window (app_id|title)
cos-cli query visible-on 3       # count of visible (non-minimized) windows on workspace 3
cos-cli query windows-on 5       # list all windows on workspace 5
cos-cli query last-minimized 1   # most recently minimized window on workspace 1
cos-cli query shutdown           # stop the daemon
````

#### Debugging

Start the daemon with `--verbose` to see all window events, focus changes, workspace switches, and debug output from commands like `move-to`, `minimize`, and `unminimize` — everything appears in the daemon's terminal output.

````console
cos-cli daemon --verbose
````

Use `--log` to write the same output to `$XDG_RUNTIME_DIR/cos-cli.log` for later review. Can be used with or without `--verbose`:

````console
cos-cli daemon --log               # silent terminal, log file only
cos-cli daemon --verbose --log     # both terminal and log file
````
