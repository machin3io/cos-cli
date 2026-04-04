use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::fs;
use std::path::{Path, PathBuf};

use wayland_client::Connection;

use crate::{AppState, State};


// persistent state tracked by the daemon
pub struct DaemonState {
    pub active_workspace: String,
    pub windows: HashMap<u32, WindowInfo>,
    next_window_id: u32,
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
    pub workspace: String,
    pub state: Vec<String>,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            active_workspace: "1".to_string(),
            windows: HashMap::new(),
            next_window_id: 0,
        }
    }

    pub fn add_window(&mut self, app_id: &str, title: &str, workspace: &str) -> u32 {
        let id = self.next_window_id;
        self.next_window_id += 1;

        self.windows.insert(id, WindowInfo {
            app_id: app_id.to_string(),
            title: title.to_string(),
            workspace: workspace.to_string(),
            state: Vec::new(),
        });

        id
    }

    pub fn remove_window(&mut self, id: u32) {
        self.windows.remove(&id);
    }

    pub fn windows_on_workspace(&self, workspace: &str) -> Vec<&WindowInfo> {
        self.windows.values().filter(|w| w.workspace == workspace).collect()
    }

    pub fn visible_windows_on_workspace(&self, workspace: &str) -> Vec<&WindowInfo> {
        self.windows.values().filter(|w| {
            w.workspace == workspace && !w.state.contains(&"minimized".to_string())
        }).collect()
    }
}


pub fn socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    Path::new(&runtime_dir).join("cos-cli.sock")
}


pub fn daemon_running() -> bool {
    let path = socket_path();

    if !path.exists() {
        return false;
    }

    // try to connect to check if daemon is alive
    match UnixStream::connect(&path) {
        Ok(mut stream) => {
            let _ = stream.write_all(b"ping\n");
            let _ = stream.flush();

            let mut reader = BufReader::new(&stream);
            let mut response = String::new();

            if reader.read_line(&mut response).is_ok() {
                response.trim() == "pong"
            } else {
                false
            }
        }
        Err(_) => {
            // stale socket, clean up
            let _ = fs::remove_file(&path);
            false
        }
    }
}


pub fn send_command(cmd: &str) -> Result<String, String> {
    use std::io::Read;

    let path = socket_path();

    let mut stream = UnixStream::connect(&path).map_err(|e| format!("failed to connect to daemon: {}", e))?;

    stream.write_all(cmd.as_bytes()).map_err(|e| format!("failed to send command: {}", e))?;
    stream.write_all(b"\n").map_err(|e| format!("failed to send newline: {}", e))?;
    stream.flush().map_err(|e| format!("failed to flush: {}", e))?;

    // shut down the write end so the server knows we're done
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
        workspace_manager: None,
        workspace_group: Vec::new(),
        apps: Vec::new(),
        outputs: Vec::new(),
        seats: Vec::new(),
    };

    let _registry = conn.display().get_registry(&qh, ());

    // initial roundtrips to populate state
    event_queue.roundtrip(&mut wl_state)?;
    event_queue.roundtrip(&mut wl_state)?;

    // populate daemon state from initial wayland state
    {
        let mut ds = state.lock().unwrap();

        // find active workspace
        for group in &wl_state.workspace_group {
            for ws in group {
                if ws.active {
                    ds.active_workspace = ws.name.clone();
                }
            }
        }

        // register all existing windows on the active workspace (best guess)
        for app in &wl_state.apps {
            let app_id = app.app_id.as_deref().unwrap_or("unknown");
            let title = app.title.as_deref().unwrap_or("");
            let ws = &ds.active_workspace.clone();

            let id = ds.add_window(app_id, title, ws);

            let states: Vec<String> = app.state.iter().map(|s| s.to_string()).collect();
            if let Some(w) = ds.windows.get_mut(&id) {
                w.state = states;
            }
        }

        println!("initial state: {} windows on workspace {}", ds.windows.len(), ds.active_workspace);
    }

    // main event loop — track changes, don't rebuild
    println!("entering event loop...");

    // keep a snapshot of known app_id+title combos to detect new/removed windows
    let mut known_keys: Vec<(String, String)> = wl_state.apps.iter().map(|a| {
        (a.app_id.as_deref().unwrap_or("unknown").to_string(), a.title.as_deref().unwrap_or("").to_string())
    }).collect();

    loop {
        event_queue.blocking_dispatch(&mut wl_state)?;

        let mut ds = state.lock().unwrap();

        // sync workspace state from wayland
        for group in &wl_state.workspace_group {
            for ws in group {
                if ws.active && ws.name != ds.active_workspace {
                    println!("workspace changed: {} -> {}", ds.active_workspace, ws.name);
                    ds.active_workspace = ws.name.clone();
                }
            }
        }

        // build current app keys
        let current_keys: Vec<(String, String)> = wl_state.apps.iter().map(|a| {
            (a.app_id.as_deref().unwrap_or("unknown").to_string(), a.title.as_deref().unwrap_or("").to_string())
        }).collect();

        // detect new windows — assign to active workspace
        for (app_id, title) in &current_keys {
            if !known_keys.contains(&(app_id.clone(), title.clone())) {
                let ws = ds.active_workspace.clone();
                println!("new window: {} '{}' on workspace {}", app_id, title, ws);
                ds.add_window(app_id, title, &ws);
            }
        }

        // detect removed windows
        let removed: Vec<u32> = ds.windows.iter()
            .filter(|(_, w)| !current_keys.contains(&(w.app_id.clone(), w.title.clone())))
            .map(|(id, _)| *id)
            .collect();

        for id in &removed {
            if let Some(w) = ds.windows.get(id) {
                println!("window removed: {} '{}'", w.app_id, w.title);
            }
            ds.windows.remove(id);
        }

        // update state (minimized, activated, etc.) on existing windows
        let active_ws = ds.active_workspace.clone();

        for app in &wl_state.apps {
            let app_id = app.app_id.as_deref().unwrap_or("unknown");
            let title = app.title.as_deref().unwrap_or("");
            let states: Vec<String> = app.state.iter().map(|s| s.to_string()).collect();

            if let Some(w) = ds.windows.values_mut().find(|w| w.app_id == app_id && w.title == title) {
                let old_state = w.state.clone();
                w.state = states.clone();

                // if the app just became activated, it's on the active workspace
                if states.contains(&"activated".to_string()) && !old_state.contains(&"activated".to_string()) {
                    println!("  activated: {} '{}' moved {} -> {}", app_id, title, w.workspace, active_ws);
                    w.workspace = active_ws.clone();
                }
            }
        }

        // update title changes on existing windows
        for app in &wl_state.apps {
            let app_id = app.app_id.as_deref().unwrap_or("unknown");
            let title = app.title.as_deref().unwrap_or("");

            // find a window with this app_id whose title changed
            let has_existing = ds.windows.values().any(|w| w.app_id == app_id && w.title == title);

            if !has_existing {
                if let Some(w) = ds.windows.values_mut().find(|w| w.app_id == app_id && w.title != title) {
                    w.title = title.to_string();
                }
            }
        }

        known_keys = current_keys;
    }
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

            let entries: Vec<String> = windows.iter().map(|w| {
                format!("{}|{}|{}", w.app_id, w.title, w.state.join(","))
            }).collect();

            if entries.is_empty() {
                "none".to_string()
            } else {
                entries.join(";")
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

                // find the first window with this app_id and update its workspace
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
            // return full state as simple text
            let mut out = format!("workspace:{}\n", ds.active_workspace);

            for (id, w) in &ds.windows {
                out.push_str(&format!("window:{}:{}:{}:{}:{}\n", id, w.app_id, w.workspace, w.state.join(","), w.title));
            }

            out
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
