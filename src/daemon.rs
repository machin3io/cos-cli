use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::fs;
use std::path::{Path, PathBuf};

use wayland_client::{Connection, Proxy};

use crate::{AppState, State};


pub static VERBOSE: AtomicBool = AtomicBool::new(false);
static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

pub fn set_log_file(path: PathBuf) {
    if let Ok(mut lf) = LOG_FILE.lock() {
        // create/clear the log file
        let _ = fs::write(&path, "");
        println!("logging to {}", path.display());
        *lf = Some(path);
    }
}

pub fn daemon_log(msg: &str) {
    if verbose() {
        println!("{}", msg);
    }

    if let Ok(lf) = LOG_FILE.lock() {
        if let Some(ref path) = *lf {
            let mut content = fs::read_to_string(path).unwrap_or_default();
            content.push_str(msg);
            content.push('\n');
            let _ = fs::write(path, content);
        }
    }
}


// actions that the IPC handler queues for the event loop to execute
#[derive(Debug)]
pub enum PendingAction {
    AutoMaximize(String),     // workspace name
    UnmaximizeAll(String),    // workspace name
    MoveFocusedTo(String),    // target workspace name
    MinimizeFocused,
    UnminimizeLast,           // unminimize last minimized on active workspace
}


// persistent state tracked by the daemon, keyed on wayland handle object ID
pub struct DaemonState {
    pub active_workspace: String,
    pub windows: HashMap<String, WindowInfo>,
    pub pending_actions: Vec<PendingAction>,
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
    pub workspace: String,
    pub state: Vec<String>,
    pub minimized_at: Option<u64>,
    pub activated_at: Option<u64>,
    pub auto_maximized: bool,
    pub workspace_confirmed: bool,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            active_workspace: "1".to_string(),
            windows: HashMap::new(),
            pending_actions: Vec::new(),
        }
    }


    fn queue_auto_maximize(&mut self, workspace: String) {
        // deduplicate — don't queue if one is already pending for this workspace
        let already_pending = self.pending_actions.iter().any(|a| matches!(a, PendingAction::AutoMaximize(ws) if ws == &workspace));

        if !already_pending {
            self.pending_actions.push(PendingAction::AutoMaximize(workspace));
        }
    }


    // called from dispatch when a new toplevel appears
    pub fn on_window_created(&mut self, handle_id: &str) {
        let ws = self.active_workspace.clone();
        daemon_log(&format!("new window: {} on workspace {}", handle_id, ws));

        self.windows.insert(handle_id.to_string(), WindowInfo {
            app_id: String::new(),
            title: String::new(),
            workspace: ws.clone(),
            state: Vec::new(),
            minimized_at: None,
            activated_at: None,
            auto_maximized: false,
            workspace_confirmed: false,
        });

        self.queue_auto_maximize(ws);
    }


    // called from dispatch when a toplevel's title changes
    pub fn on_title_changed(&mut self, handle_id: &str, title: &str) {
        if let Some(w) = self.windows.get_mut(handle_id) {
            if w.title != title {
                daemon_log(&format!("  title: {} '{}' -> '{}'", w.app_id, w.title, title));
                w.title = title.to_string();
            }
        }
    }


    // called from dispatch when a toplevel's app_id changes
    pub fn on_app_id_changed(&mut self, handle_id: &str, app_id: &str) {
        if let Some(w) = self.windows.get_mut(handle_id) {
            w.app_id = app_id.to_string();
        }
    }


    // called from dispatch when a toplevel's state changes
    pub fn on_state_changed(&mut self, handle_id: &str, states: &[State]) {
        let mut auto_maximize_ws: Option<String> = None;

        if let Some(w) = self.windows.get_mut(handle_id) {
            let new_states: Vec<String> = states.iter().map(|s| s.to_string()).collect();

            let was_minimized = w.state.contains(&"minimized".to_string());
            let is_minimized = new_states.contains(&"minimized".to_string());
            let was_activated = w.state.contains(&"activated".to_string());
            let is_activated = new_states.contains(&"activated".to_string());

            w.state = new_states;

            // track activation timestamp
            if is_activated && !was_activated {
                w.activated_at = Some(now_millis());
                daemon_log(&format!("  focus: {} '{}' on workspace {}", w.app_id, w.title, w.workspace));

                // validate: a focused window cannot be minimized
                if is_minimized {
                    daemon_log(&format!("  state fix: {} '{}' clearing stale minimized state", w.app_id, w.title));
                    w.state.retain(|s| s != "minimized");
                    w.minimized_at = None;
                }

                // NOTE: workspace validation via activation is not reliable —
                // ####: COSMIC sends activation events for windows on other
                // ####: workspaces (multi-activation quirk). pre-existing windows
                // ####: get corrected by workspace_enter events on their next move
            } else if !is_activated && was_activated {
                w.activated_at = None;
            }

            // track minimize timestamp
            if is_minimized && !was_minimized {
                w.minimized_at = Some(now_millis());
                auto_maximize_ws = Some(w.workspace.clone());
            } else if !is_minimized && was_minimized {
                w.minimized_at = None;
                auto_maximize_ws = Some(w.workspace.clone());
            }
        }

        if let Some(ws) = auto_maximize_ws {
            self.queue_auto_maximize(ws);
        }
    }


    // called from dispatch when a toplevel is closed
    pub fn on_window_closed(&mut self, handle_id: &str) {
        if let Some(w) = self.windows.remove(handle_id) {
            daemon_log(&format!("window closed: {} '{}'", w.app_id, w.title));

            // check if remaining sole window should be auto-maximized
            self.pending_actions.push(PendingAction::AutoMaximize(w.workspace));
        }
    }


    // called from dispatch when workspace active state changes
    pub fn on_workspace_changed(&mut self, name: &str) {
        if name != self.active_workspace {
            daemon_log(&format!("workspace changed: {} -> {}", self.active_workspace, name));
            self.active_workspace = name.to_string();
        }
    }


    // called from dispatch when a toplevel enters a workspace
    pub fn on_workspace_enter(&mut self, handle_id: &str, workspace: &str) {
        if let Some(w) = self.windows.get_mut(handle_id) {
            let old_ws = w.workspace.clone();
            w.workspace_confirmed = true;

            if old_ws != workspace {
                daemon_log(&format!("  workspace enter: {} '{}' moved {} -> {}", w.app_id, w.title, old_ws, workspace));
                w.workspace = workspace.to_string();

                // trigger auto-maximize on both source and destination workspaces
                self.queue_auto_maximize(old_ws);
                self.queue_auto_maximize(workspace.to_string());
            } else {
                daemon_log(&format!("  workspace enter: {} '{}' on workspace {}", w.app_id, w.title, workspace));
            }
        }

    }


    // called from dispatch when a toplevel leaves a workspace
    pub fn on_workspace_leave(&mut self, handle_id: &str, workspace: &str) {
        if let Some(w) = self.windows.get(handle_id) {
            daemon_log(&format!("  workspace leave: {} '{}' left workspace {}", w.app_id, w.title, workspace));
        }
    }


    pub fn windows_on_workspace(&self, workspace: &str) -> Vec<&WindowInfo> {
        self.windows.values().filter(|w| w.workspace == workspace).collect()
    }


    pub fn visible_windows_on_workspace(&self, workspace: &str) -> Vec<&WindowInfo> {
        self.windows.values().filter(|w| {
            w.workspace == workspace && !w.state.contains(&"minimized".to_string())
        }).collect()
    }


    pub fn focused_window(&self) -> Option<(&String, &WindowInfo)> {
        // the focused window must be activated and on the active workspace
        // if multiple are activated, pick the most recently activated one
        self.windows.iter()
            .filter(|(_, w)| {
                w.workspace == self.active_workspace
                    && w.state.contains(&"activated".to_string())
                    && w.activated_at.is_some()
            })
            .max_by_key(|(_, w)| w.activated_at.unwrap())
    }


    pub fn sole_visible_on_workspace(&self, workspace: &str) -> Option<(&String, &WindowInfo)> {
        let visible: Vec<(&String, &WindowInfo)> = self.windows.iter()
            .filter(|(_, w)| w.workspace == workspace && !w.state.contains(&"minimized".to_string()))
            .collect();

        if visible.len() == 1 { Some((visible[0].0, visible[0].1)) } else { None }
    }


    pub fn last_minimized_on_workspace(&self, workspace: &str) -> Option<(&String, &WindowInfo)> {
        self.windows.iter()
            .filter(|(_, w)| w.workspace == workspace && w.state.contains(&"minimized".to_string()) && w.minimized_at.is_some())
            .max_by_key(|(_, w)| w.minimized_at.unwrap())
    }

}


pub fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}


// extract a stable string ID from a wayland handle for use as a HashMap key
pub fn handle_id(handle: &cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1) -> String {
    format!("{:?}", handle.id())
}


pub fn socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    Path::new(&runtime_dir).join("cos-cli.sock")
}




pub fn send_command(cmd: &str) -> Result<String, String> {
    let path = socket_path();

    let mut stream = UnixStream::connect(&path).map_err(|e| format!("failed to connect to daemon: {}", e))?;

    stream.write_all(cmd.as_bytes()).map_err(|e| format!("failed to send command: {}", e))?;
    stream.write_all(b"\n").map_err(|e| format!("failed to send newline: {}", e))?;
    stream.flush().map_err(|e| format!("failed to flush: {}", e))?;

    stream.shutdown(std::net::Shutdown::Write).map_err(|e| format!("failed to shutdown write: {}", e))?;

    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| format!("failed to read response: {}", e))?;

    Ok(response.trim().to_string())
}


pub fn run_daemon() -> Result<(), Box<dyn std::error::Error>> {
    let path = socket_path();

    // clean up stale socket
    if path.exists() {
        let _ = fs::remove_file(&path);
    }

    let state = Arc::new(Mutex::new(DaemonState::new()));

    // start the IPC listener in a separate thread
    let ipc_state = Arc::clone(&state);
    let listener = UnixListener::bind(&path)?;
    println!("daemon listening on {}", path.display());

    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let state = Arc::clone(&ipc_state);
                    thread::spawn(move || handle_client(stream, state));
                }
                Err(e) => {
                    eprintln!("connection error: {}", e);
                }
            }
        }
    });


    // connect to wayland and run the event loop
    let conn = Connection::connect_to_env()?;
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut wl_state = AppState {
        cosmic_toplevel_manager: None,
        cosmic_toplevel_info: None,
        workspace_manager: None,
        workspace_group: Vec::new(),
        apps: Vec::new(),
        foreign_toplevel_done: std::collections::HashSet::new(),
        foreign_toplevel_props: std::collections::HashMap::new(),
        foreign_to_cosmic: std::collections::HashMap::new(),
        outputs: Vec::new(),
        seats: Vec::new(),
        daemon_state: Some(Arc::clone(&state)),
    };

    let _registry = conn.display().get_registry(&qh, ());

    // initial roundtrips to populate state
    event_queue.roundtrip(&mut wl_state)?;
    event_queue.roundtrip(&mut wl_state)?;

    // log initial state
    {
        let ds = state.lock().unwrap();
        println!("initial state: {} windows on workspace {}", ds.windows.len(), ds.active_workspace);
    }

    // NOTE: all window tracking happens in the dispatch handlers (dispatch.rs)
    // ####: the event loop processes pending actions queued by IPC handlers
    println!("entering event loop...");

    loop {
        event_queue.blocking_dispatch(&mut wl_state)?;

        // process pending actions from IPC
        let actions: Vec<PendingAction> = {
            let mut ds = state.lock().unwrap();
            std::mem::take(&mut ds.pending_actions)
        };

        for action in actions {
            match action {
                PendingAction::MoveFocusedTo(workspace) => {
                    process_move_focused_to(&state, &wl_state, &conn, &workspace);
                }
                PendingAction::MinimizeFocused => {
                    process_minimize_focused(&state, &wl_state, &conn);
                }
                PendingAction::UnminimizeLast => {
                    process_unminimize_last(&state, &wl_state, &conn);
                }
                PendingAction::AutoMaximize(workspace) => {
                    process_auto_maximize(&state, &wl_state, &conn, &workspace);
                }
                PendingAction::UnmaximizeAll(workspace) => {
                    process_unmaximize_all(&state, &wl_state, &conn, &workspace);
                }
            }
        }
    }
}


fn process_auto_maximize(
    daemon_state: &Arc<Mutex<DaemonState>>,
    wl_state: &crate::AppState,
    conn: &Connection,
    workspace: &str,
) {
    if !crate::is_auto_maximize_enabled() {
        return;
    }

    let ds = daemon_state.lock().unwrap();

    let (_, outer) = crate::get_gaps_for_workspace(workspace);

    if outer != 0 {
        return;
    }

    let exclusions = crate::auto_maximize_exclusions();

    // log excluded windows on this workspace
    for (_, w) in ds.windows.iter() {
        if w.workspace == workspace && !w.state.contains(&"minimized".to_string()) && crate::is_excluded(&w.app_id, &w.title, &exclusions) {
            daemon_log(&format!("  excluded from window count: {} '{}' on workspace {}", w.app_id, w.title, workspace));
        }
    }

    let visible: Vec<(&String, &WindowInfo)> = ds.windows.iter()
        .filter(|(_, w)| w.workspace == workspace && !w.state.contains(&"minimized".to_string()) && !crate::is_excluded(&w.app_id, &w.title, &exclusions))
        .collect();

    if visible.len() == 1 {
        // sole visible window on zero-gap workspace — auto-maximize it
        let hid = visible[0].0.clone();
        let app_id = visible[0].1.app_id.clone();
        let title = visible[0].1.title.clone();
        drop(ds);

        if let Some(manager) = &wl_state.cosmic_toplevel_manager {
            if let Some(app) = wl_state.apps.iter().find(|a| handle_id(&a.handle) == hid) {
                if !app.state.contains(&crate::State::Maximized) {
                    daemon_log(&format!("  auto-maximize: {} '{}' on workspace {}", app_id, title, workspace));
                    manager.set_maximized(&app.handle);
                    conn.flush().ok();

                    // mark as auto-maximized so we only undo our own maximization
                    let mut ds = daemon_state.lock().unwrap();
                    if let Some(w) = ds.windows.get_mut(&hid) {
                        w.auto_maximized = true;
                    }
                }

                // NOTE: auto-activate disabled — the compositor appears to send
                // ####: activated state natively on workspace switch. this was
                // ####: originally added pre-v3 when focus events were unreliable.
                // ####: remove entirely if no issues arise without it
                //
                // if let Some(seat) = wl_state.seats.first() {
                //     daemon_log(&format!("  auto-activate: {} '{}' on workspace {}", app_id, title, workspace));
                //     manager.activate(&app.handle, &seat.0);
                //     conn.flush().ok();
                // }
            }
        }

    } else if visible.len() > 1 {
        // multiple visible windows — only unmaximize those we auto-maximized
        let auto_maximized: Vec<(String, String, String)> = visible.iter()
            .filter(|(_, w)| w.auto_maximized)
            .map(|(id, w)| (id.to_string(), w.app_id.clone(), w.title.clone()))
            .collect();

        drop(ds);

        if !auto_maximized.is_empty() {
            if let Some(manager) = &wl_state.cosmic_toplevel_manager {
                for (hid, app_id, title) in &auto_maximized {
                    if let Some(app) = wl_state.apps.iter().find(|a| handle_id(&a.handle) == *hid) {
                        daemon_log(&format!("  auto-unmaximize: {} '{}' on workspace {}", app_id, title, workspace));
                        manager.unset_maximized(&app.handle);
                    }
                }
                conn.flush().ok();
            }

            // clear the flag
            let mut ds = daemon_state.lock().unwrap();
            for (hid, _, _) in &auto_maximized {
                if let Some(w) = ds.windows.get_mut(hid) {
                    w.auto_maximized = false;
                }
            }
        }
    }
}


fn process_unmaximize_all(
    daemon_state: &Arc<Mutex<DaemonState>>,
    wl_state: &crate::AppState,
    conn: &Connection,
    workspace: &str,
) {
    let ds = daemon_state.lock().unwrap();

    // find all maximized windows on the workspace
    let maximized: Vec<String> = ds.windows.iter()
        .filter(|(_, w)| w.workspace == workspace && w.state.contains(&"maximized".to_string()))
        .map(|(id, _)| id.clone())
        .collect();

    if maximized.is_empty() {
        return;
    }

    drop(ds);

    if let Some(manager) = &wl_state.cosmic_toplevel_manager {
        for hid in &maximized {
            if let Some(app) = wl_state.apps.iter().find(|a| handle_id(&a.handle) == *hid) {
                daemon_log(&format!("  auto-unmaximize: {} '{}' on workspace {}", app.app_id.as_deref().unwrap_or("?"), app.title.as_deref().unwrap_or("?"), workspace));
                manager.unset_maximized(&app.handle);
            }
        }
        conn.flush().ok();
    }
}


fn process_move_focused_to(
    daemon_state: &Arc<Mutex<DaemonState>>,
    wl_state: &crate::AppState,
    conn: &Connection,
    target_workspace: &str,
) {
    let ds = daemon_state.lock().unwrap();

    // find the focused window, fall back to sole visible window on the workspace
    // (COSMIC may not have sent the activation event yet after a workspace switch)
    let mut used_fallback = false;
    let focused = ds.focused_window()
        .or_else(|| { used_fallback = true; ds.sole_visible_on_workspace(&ds.active_workspace) })
        .map(|(id, w)| (id.clone(), w.app_id.clone(), w.title.clone()));
    drop(ds);

    let Some((hid, app_id, title)) = focused else {
        daemon_log("  move-to: no focused window found");
        return;
    };

    if used_fallback {
        daemon_log(&format!("  move-to: using sole visible window {} '{}' (no activation event yet)", app_id, title));
    }

    let Some(manager) = &wl_state.cosmic_toplevel_manager else { return; };

    // find the app by handle ID
    let Some(app) = wl_state.apps.iter().find(|a| handle_id(&a.handle) == hid) else {
        daemon_log(&format!("  move-to: handle not found for {} '{}'", app_id, title));
        return;
    };

    // find the target workspace handle
    let Some(ws) = wl_state.workspace_group.iter().flat_map(|v| v.iter()).find(|ws| ws.name == target_workspace) else {
        daemon_log(&format!("  move-to: workspace {} not found", target_workspace));
        return;
    };

    let output = if wl_state.outputs.is_empty() {
        daemon_log("  move-to: no outputs found");
        return;
    } else {
        wl_state.outputs[0].0.clone()
    };

    daemon_log(&format!("  move-to: {} '{}' -> workspace {}", app_id, title, target_workspace));
    manager.move_to_ext_workspace(&app.handle, &ws.handle, &output);
    conn.flush().ok();

    // update daemon state and trigger auto-maximize on the source workspace
    let mut ds = daemon_state.lock().unwrap();
    let source_ws = ds.windows.get(&hid).map(|w| w.workspace.clone());
    if let Some(w) = ds.windows.get_mut(&hid) {
        w.workspace = target_workspace.to_string();
    }
    if let Some(ws) = source_ws {
        ds.queue_auto_maximize(ws);
    }
}


fn process_minimize_focused(
    daemon_state: &Arc<Mutex<DaemonState>>,
    wl_state: &crate::AppState,
    conn: &Connection,
) {
    let ds = daemon_state.lock().unwrap();

    let focused = ds.focused_window().map(|(id, w)| (id.clone(), w.app_id.clone(), w.title.clone(), w.workspace.clone()));
    drop(ds);

    let Some((hid, app_id, title, workspace)) = focused else {
        daemon_log("  minimize: no focused window found");
        return;
    };

    let Some(manager) = &wl_state.cosmic_toplevel_manager else { return; };

    let Some(app) = wl_state.apps.iter().find(|a| handle_id(&a.handle) == hid) else {
        daemon_log(&format!("  minimize: handle not found for {} '{}'", app_id, title));
        return;
    };

    daemon_log(&format!("  minimize: {} '{}' on workspace {}", app_id, title, workspace));
    manager.set_minimized(&app.handle);
    conn.flush().ok();
}


fn process_unminimize_last(
    daemon_state: &Arc<Mutex<DaemonState>>,
    wl_state: &crate::AppState,
    conn: &Connection,
) {
    let ds = daemon_state.lock().unwrap();

    let active_ws = ds.active_workspace.clone();

    let last = ds.last_minimized_on_workspace(&active_ws)
        .map(|(id, w)| (id.clone(), w.app_id.clone(), w.title.clone()));
    drop(ds);

    let Some((hid, app_id, title)) = last else {
        daemon_log(&format!("  unminimize: no minimized windows on workspace {}", active_ws));
        return;
    };

    let Some(manager) = &wl_state.cosmic_toplevel_manager else { return; };

    let Some(app) = wl_state.apps.iter().find(|a| handle_id(&a.handle) == hid) else {
        daemon_log(&format!("  unminimize: handle not found for {} '{}'", app_id, title));
        return;
    };

    daemon_log(&format!("  unminimize: {} '{}' on workspace {}", app_id, title, active_ws));
    manager.unset_minimized(&app.handle);
    conn.flush().ok();
}


fn handle_client(stream: UnixStream, state: Arc<Mutex<DaemonState>>) {
    let mut reader = BufReader::new(&stream);
    let mut writer = &stream;

    let mut line = String::new();

    if reader.read_line(&mut line).is_err() {
        return;
    }

    let cmd = line.trim();
    let ds = state.lock().unwrap();

    let response = match cmd {
        "ping" => "pong".to_string(),

        "active-workspace" => ds.active_workspace.clone(),

        cmd if cmd.starts_with("windows-on ") => {
            let ws = &cmd[11..];
            let windows = ds.windows_on_workspace(ws);

            if windows.is_empty() {
                "none".to_string()
            } else {
                windows.iter().map(|w| {
                    let state = if w.state.is_empty() { String::new() } else { format!(" [{}]", w.state.join(",")) };
                    format!("{}:{}{}", w.app_id, w.title, state)
                }).collect::<Vec<_>>().join("\n")
            }
        }

        cmd if cmd.starts_with("visible-on ") => {
            let ws = &cmd[11..];
            let windows = ds.visible_windows_on_workspace(ws);
            windows.len().to_string()
        }

        cmd if cmd.starts_with("set-workspace ") => {
            let ws = &cmd[14..];
            drop(ds);
            let mut ds = state.lock().unwrap();
            ds.active_workspace = ws.to_string();
            "ok".to_string()
        }

        cmd if cmd.starts_with("move-window ") => {
            // move-window app_id:target_workspace
            let parts: Vec<&str> = cmd[12..].splitn(2, ':').collect();

            if parts.len() == 2 {
                let app_id = parts[0];
                let target_ws = parts[1];

                drop(ds);
                let mut ds = state.lock().unwrap();

                if let Some(w) = ds.windows.values_mut().find(|w| w.app_id == app_id) {
                    w.workspace = target_ws.to_string();
                    "ok".to_string()
                } else {
                    "error:window not found".to_string()
                }
            } else {
                "error:bad format".to_string()
            }
        }

        "info" => {
            let mut out = format!("workspace:{}\n", ds.active_workspace);

            for (id, w) in &ds.windows {
                out.push_str(&format!("window:{}:{}:{}:{}:{}\n", id, w.app_id, w.workspace, w.state.join(","), w.title));
            }

            out
        }

        cmd if cmd.starts_with("do-move-to ") => {
            let ws = &cmd[11..];
            drop(ds);
            let mut ds = state.lock().unwrap();
            ds.pending_actions.push(PendingAction::MoveFocusedTo(ws.to_string()));
            "ok".to_string()
        }

        "do-minimize" => {
            drop(ds);
            let mut ds = state.lock().unwrap();
            ds.pending_actions.push(PendingAction::MinimizeFocused);
            "ok".to_string()
        }

        "do-unminimize" => {
            drop(ds);
            let mut ds = state.lock().unwrap();
            ds.pending_actions.push(PendingAction::UnminimizeLast);
            "ok".to_string()
        }

        cmd if cmd.starts_with("auto-maximize ") => {
            let ws = &cmd[14..];
            drop(ds);
            let mut ds = state.lock().unwrap();
            ds.pending_actions.push(PendingAction::AutoMaximize(ws.to_string()));
            "ok".to_string()
        }

        cmd if cmd.starts_with("unmaximize-all ") => {
            let ws = &cmd[15..];
            drop(ds);
            let mut ds = state.lock().unwrap();
            ds.pending_actions.push(PendingAction::UnmaximizeAll(ws.to_string()));
            "ok".to_string()
        }

        cmd if cmd.starts_with("sole-window-on ") => {
            let ws = &cmd[15..];

            if let Some((id, w)) = ds.sole_visible_on_workspace(ws) {
                format!("{}|{}|{}", id, w.app_id, w.title)
            } else {
                "none".to_string()
            }
        }

        "focused" => {
            if let Some((_, w)) = ds.focused_window() {
                format!("{}|{}", w.app_id, w.title)
            } else {
                "none".to_string()
            }
        }

        cmd if cmd.starts_with("last-minimized ") => {
            let ws = &cmd[15..];

            if let Some((_, w)) = ds.last_minimized_on_workspace(ws) {
                format!("{}|{}", w.app_id, w.title)
            } else {
                "none".to_string()
            }
        }

        cmd if cmd.starts_with("log ") => {
            daemon_log(&cmd[4..]);
            "ok".to_string()
        }

        "shutdown" => {
            let _ = writer.write_all(b"bye\n");
            let _ = writer.flush();
            std::process::exit(0);
        }

        _ => "error:unknown command".to_string(),
    };

    let _ = writer.write_all(response.as_bytes());
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
}
