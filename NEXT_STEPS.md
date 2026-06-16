# Next Steps


## 1. Floating Windows on Demand (priority)

The goal is to launch a terminal or ranger instance that floats immediately instead of tiling, without disturbing the existing tiling layout.

### Approach

COSMIC has native window rules that can match by app_id or title and set properties like floating. The same pattern used on AwesomeWM applies: launch the app with a custom title (e.g. `alacritty --title "floating terminal"`), and a COSMIC window rule auto-floats it.

This was attempted before but failed due to a config file naming issue (plural vs singular). There's a COSMIC issue about this — find it, confirm the correct config path, and set it up.

### Steps

- find the COSMIC issue / config docs about window rules and the naming fix
- set up the correct config file with rules matching "floating terminal" and "floating ranger" titles
- create keybinds that launch `alacritty --title "floating terminal"` etc.
- no cos-cli changes needed for this — it's purely COSMIC-native


## 2. Auto-Fullscreen by App ID

Auto-fullscreen specific applications instead of auto-maximizing them. Fullscreen removes the title bar and panel, and Alt+Tab doesn't break the fullscreen state (unlike maximize).

### Key differences from auto-maximize

- **app-ID-based**, not sole-window-based — Blender gets fullscreened even with a terminal on the same workspace (the launcher always opens a terminal alongside Blender)
- uses `set_fullscreen` / `unset_fullscreen` instead of `set_maximized` / `unset_maximized`
- tracks `auto_fullscreened` flag (same pattern as existing `auto_maximized`) for clean undo

### Config

New config file at `~/.config/cosmic/cos-cli/auto_fullscreen` listing app IDs, e.g.:

```
org.blender.Blender
firefox
```

Could be enabled by default with Blender and Firefox pre-populated.

### Gating / rule conditions

Multiple conditions can be combined per app ID:

- **gap-based gating (primary):** only auto-fullscreen when the workspace has zero gaps. This is the key ergonomic feature — increasing gaps via `cos-cli gap --increase` disables auto-fullscreen, effectively entering "debug mode" where you can see the terminal alongside Blender. Decreasing gaps re-enters fullscreen mode. Very convenient since gap toggling already has keymaps.
- **workspace-based gating (optional):** only apply on specific workspaces. Useful for e.g. a dedicated gaming workspace where everything should be fullscreened.
- **always mode (optional):** some app IDs might always fullscreen regardless of gaps or workspace.

### Config format ideas

Simple — one app_id per line, gap gating by default:

```
org.blender.Blender
firefox
```

Extended — with optional modifiers:

```
# gap gating (default behavior)
org.blender.Blender
firefox

# always fullscreen regardless of gaps
some.game:always

# only on specific workspace
another.app:workspace=5
```

Or keep it simple with just app IDs and a separate boolean config for gap gating (`auto_fullscreen_gap_gating` or similar). Start simple, extend later.

### Undo behavior

- when gaps are increased (condition no longer holds): unfullscreen any auto-fullscreened windows
- track which windows were auto-fullscreened vs manually fullscreened, same pattern as `auto_maximized`

### Integration with existing auto-maximize

- auto-fullscreen takes precedence over auto-maximize for matching app IDs
- non-matching apps still get auto-maximized via the existing sole-window + zero-gap logic
- both systems coexist cleanly


## 3. Other observations

- maximize and fullscreen can be active simultaneously on the same window
- fullscreen suppresses notifications (maximize doesn't)
- Alt+Tab doesn't break fullscreen but does break maximize
- COSMIC's tiling layout (master/slave zones, window placement order) is entirely internal to the compositor — no protocol exposure for controlling which zone a new window goes into