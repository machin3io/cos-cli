# cos-cli

A CLI tool for managing windows and workspaces on the COSMIC Desktop Environment (Wayland).

> **Note:** This is a third-party, unofficial tool. It is not affiliated with System76 or the official COSMIC project.

> **Fork note:** This is a `machin3` fork (`workspace_control` branch). The upstream repo is [estin/cos-cli](https://github.com/estin/cos-cli).

## Features
- **List Information**: View active applications, workspaces (with active state), and outputs.
- **Window Management**: Move applications between workspaces by their App ID.
- **Activate Application**: Bring a specific application to the foreground.
- **Window State**: Set window state (maximize, minimize, fullscreen, sticky).
- **Workspace Switching**: Switch directly to any workspace by name, or cycle with next/prev (with wrapping and optional dynamic workspace exclusion).
- **Workspace Toggle**: Switch back and forth between the last two workspaces via statefile history.
- **Minimize/Unminimize**: Minimize the focused app with per-workspace history, unminimize the last minimized app on the current workspace.

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
````

#### `minimize`
Minimize the currently focused app with per-workspace history tracking.
````console
cos-cli minimize
````

Finds the focused (activated) app, minimizes it, and pushes an entry onto a statefile at `/tmp/cos-cli-minimized` recording the workspace, app index, and app_id.

#### `unminimize`
Restore the most recently minimized app on the current workspace.
````console
cos-cli unminimize
````

Reads the minimize stack, finds the most recent entry matching the current active workspace, and unminimizes that app. Stale entries (apps that were closed or manually unminimized) are cleaned up automatically.

##### Why a statefile?

COSMIC's Wayland compositor uses two separate workspace protocols: `ext_workspace_v1` (for workspace switching) and the older `zcosmic_workspace_v1` (for per-app workspace associations via the toplevel protocol). On current COSMIC versions, the zcosmic workspace protocol is no longer advertised, which means there is no way to query which workspace an app belongs to via the Wayland protocol alone.

The statefile approach tracks minimize history per workspace by recording the active workspace at the time of minimization. This only tracks minimizations done through cos-cli — apps minimized via the UI or other means won't appear in the history.

````
Super+N        →  cos-cli minimize
Super+Ctrl+N   →  cos-cli unminimize
````

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
