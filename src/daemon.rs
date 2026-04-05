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

fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}


// persistent state tracked by the daemon, keyed on wayland handle object ID
pub struct DaemonState {
    pub active_workspace: String,
    pub windows: HashMap<String, WindowInfo>,
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
    pub workspace: String,
    pub state: Vec<String>,
    pub minimized_at: Option<u64>,
    pub activated_at: Option<u64>,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            active_workspace: "1".to_string(),
            windows: HashMap::new(),
        }
    }


    // called from dispatch when a new toplevel appears
    pub fn on_window_created(&mut self, handle_id: &str) {
        let ws = self.active_workspace.clone();
        if verbose() { println!("new window: {} on workspace {}", handle_id, ws); }

        self.windows.insert(handle_id.to_string(), WindowInfo {
            app_id: String::new(),
            title: String::new(),
            workspace: ws,
            state: Vec::new(),
            minimized_at: None,
            activated_at: None,
        });
    }


    // called from dispatch when a toplevel's title changes
    pub fn on_title_changed(&mut self, handle_id: &str, title: &str) {
        if let Some(w) = self.windows.get_mut(handle_id) {
            w.title = title.to_string();
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
                if verbose() { println!("  focus: {} '{}' on workspace {}", w.app_id, w.title, w.workspace); }
            } else if !is_activated && was_activated {
                w.activated_at = None;
            }

            // track minimize timestamp
            if is_minimized && !was_minimized {
                w.minimized_at = Some(now_millis());
                self.write_minimize_statefile();
            } else if !is_minimized && was_minimized {
                w.minimized_at = None;
                self.write_minimize_statefile();
            }
        }
    }


    // called from dispatch when a toplevel is closed
    pub fn on_window_closed(&mut self, handle_id: &str) {
        if let Some(w) = self.windows.remove(handle_id) {
            if verbose() { println!("window closed: {} '{}'", w.app_id, w.title); }
        }
    }


    // called from dispatch when workspace active state changes
    pub fn on_workspace_changed(&mut self, name: &str) {
        if name != self.active_workspace {
            if verbose() { println!("workspace changed: {} -> {}", self.active_workspace, name); }
            self.active_workspace = name.to_string();
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


    pub fn last_minimized_on_workspace(&self, workspace: &str) -> Option<(&String, &WindowInfo)> {
        self.windows.iter()
            .filter(|(_, w)| w.workspace == workspace && w.state.contains(&"minimized".to_string()) && w.minimized_at.is_some())
            .max_by_key(|(_, w)| w.minimized_at.unwrap())
    }


    pub fn write_minimize_statefile(&self) {
        let path = Path::new("/tmp/cos-cli-minimized");
        let mut content = String::new();

        // collect all minimized windows, sorted by timestamp
        let mut minimized: Vec<&WindowInfo> = self.windows.values()
            .filter(|w| w.state.contains(&"minimized".to_string()) && w.minimized_at.is_some())
            .collect();

        minimized.sort_by_key(|w| w.minimized_at.unwrap());

        for w in minimized {
            content.push_str(&format!("{}:{}:{}:{}\n", w.workspace, w.minimized_at.unwrap(), w.app_id, w.title));
        }

        let _ = fs::write(path, content);
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


pub fn daemon_running() -> bool {
    let path = socket_path();

    if !path.exists() {
        return false;
    }

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
            let _ = fs::remove_file(&path);
            false
        }
    }
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
        workspace_manager: None,
        workspace_group: Vec::new(),
        apps: Vec::new(),
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

    // NOTE: the event loop is now minimal — all window tracking happens in the dispatch
    // ####: handlers (dispatch.rs) which write directly to DaemonState via the Arc<Mutex<>>
    // ####: we only need to keep the event loop alive
    println!("entering event loop...");

    loop {
        event_queue.blocking_dispatch(&mut wl_state)?;
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
