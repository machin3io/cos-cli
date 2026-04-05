use cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1;
use cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use wayland_client::{
    Connection, EventQueue,
    protocol::{wl_output, wl_seat},
};
use wayland_protocols::ext::workspace::v1::client::{ext_workspace_handle_v1, ext_workspace_manager_v1};

mod daemon;
mod dispatch;

const HELP: &str = "\
Usage: cos-cli [COMMAND]

A CLI utility for COSMIC Wayland toplevel and workspace management.

Commands:
  info                          List active apps, workspaces, and outputs
  move                          Move an application to a specific workspace
  activate                      Activate an application on a specific seat
  state                         Set state of an application
  workspace                     Switch to a workspace
  move-to                       Move the focused app to a specific workspace
  daemon                        Start the background daemon for persistent window tracking
  query                         Query the daemon (default: info)
  gap                           Adjust window gaps on the current workspace
  minimize                      Minimize the focused app (with history tracking)
  unminimize                    Restore the last minimized app on the current workspace

Options for 'move':
  -a, --app-id <ID>             The Application ID (partial match, case-insensitive)
  -i, --index <INDEX>           The Application index from 'info' command
  -w, --workspace <NAME>        The name of the target workspace
  -g, --workspace-group <INDEX> The workspace group index from 'info' command (optional)
  -o, --output-index <INDEX>    The output index from 'info' command (optional)
  --wait <SECONDS>              Wait for the app to appear (optional, only for --app-id)

Options for 'activate':
  -i, --index <INDEX>           The Application index from 'info' command
  -s, --seat <INDEX>            The Seat index from 'info' command (optional)

Options for 'state':
  -a, --app-id <ID>             The Application ID (partial match, case-insensitive)
  -i, --index <INDEX>           The Application index from 'info' command
  --wait <SECONDS>              Wait for the app to appear (optional, only for --app-id)
  --maximize
  --unmaximize
  --minimize
  --unminimize
  --fullscreen
  --unfullscreen
  --sticky
  --unsticky

Options for 'workspace':
  -w, --workspace <NAME>        The name of the target workspace
  -g, --workspace-group <INDEX> The workspace group index from 'info' command (optional)
  --toggle                      Switch to the previous workspace
  --next                        Switch to the next workspace (wraps around)
  --prev                        Switch to the previous workspace (wraps around)
  --max <N>                     Highest workspace number for --next/--prev (overrides --no-dynamic)
  --no-dynamic                  Auto-detect max by ignoring the trailing dynamic workspace
                                COSMIC adds after pinned ones (use with --next/--prev)
  --skip-empty                  Skip workspaces with no visible windows (requires daemon)

Options for 'move-to':
  -w, --workspace <NAME>        The name of the target workspace

Options for 'gap':
  --increase                    Increase outer gap by 5
  --decrease                    Decrease outer gap by 5

Options for 'info':
  --json                        Output in JSON format

Examples:
  cos-cli info
  cos-cli info --json
  cos-cli move --app-id Firefox --workspace 2
  cos-cli move -i 0 -w 2
  cos-cli move -a terminal -w 2 --wait 5
  cos-cli move -a terminal -w 2 -o 1
  cos-cli move -a terminal -w 2 -g 1
  cos-cli activate -i 0 -s 0
  cos-cli activate -i 0
  cos-cli workspace -w 2
  cos-cli workspace -w 3 -g 0
  cos-cli workspace --toggle
  cos-cli workspace --next
  cos-cli workspace --prev
  cos-cli workspace --next --no-dynamic
  cos-cli workspace --prev --no-dynamic
  cos-cli workspace --next --skip-empty --no-dynamic
  cos-cli workspace --prev --skip-empty --no-dynamic
  cos-cli workspace --next --max 12
  cos-cli move-to -w 5
  cos-cli move-to -w 10
  cos-cli gap --increase
  cos-cli gap --decrease
  cos-cli minimize
  cos-cli unminimize
  cos-cli daemon
  cos-cli daemon --verbose
  cos-cli daemon --log
  cos-cli daemon --verbose --log
  cos-cli daemon --restart
  cos-cli query
  cos-cli query ping
  cos-cli query active-workspace
  cos-cli query focused
  cos-cli query visible-on 3
  cos-cli query shutdown
  cos-cli state -i 0 --maximize
  cos-cli state --app-id firefox --sticky --fullscreen
";

struct CliError(String);

impl CliError {
    fn new(message: String) -> Box<Self> {
        Self(message).into()
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for CliError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "CliError({})", self.0)
    }
}

impl Error for CliError {
    fn description(&self) -> &str {
        &self.0
    }
}

impl From<String> for CliError {
    fn from(s: String) -> Self {
        CliError(s)
    }
}

impl From<&str> for CliError {
    fn from(s: &str) -> Self {
        CliError(s.to_string())
    }
}

struct Workspace {
    name: String,
    handle: ext_workspace_handle_v1::ExtWorkspaceHandleV1,
    active: bool,
}

struct AppState {
    workspace_group: Vec<Vec<Workspace>>,
    workspace_manager: Option<ext_workspace_manager_v1::ExtWorkspaceManagerV1>,
    cosmic_toplevel_manager: Option<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1>,
    outputs: Vec<(wl_output::WlOutput, String)>,
    seats: Vec<(wl_seat::WlSeat, String)>,
    apps: Vec<App>,
    daemon_state: Option<std::sync::Arc<std::sync::Mutex<daemon::DaemonState>>>,
}

#[derive(Debug, Clone)]
struct App {
    handle: zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
    title: Option<String>,
    app_id: Option<String>,
    outputs: Vec<wl_output::WlOutput>,
    state: Vec<State>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum State {
    Maximized = 0,
    Minimized = 1,
    Activated = 2,
    Fullscreen = 3,
}

impl TryFrom<u32> for State {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(State::Maximized),
            1 => Ok(State::Minimized),
            2 => Ok(State::Activated),
            3 => Ok(State::Fullscreen),
            _ => Err(()),
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            State::Maximized => "maximized",
            State::Minimized => "minimized",
            State::Fullscreen => "fullscreen",
            State::Activated => "activated",
        })
    }
}

#[derive(Debug)]
struct MoveArgs {
    app_id: Option<String>,
    app_index: Option<usize>,
    workspace_name: String,
    workspace_group_index: Option<usize>,
    output_index: Option<usize>,
    wait: Option<u64>,
}

#[derive(Debug)]
struct ActivateArgs {
    app_index: usize,
    seat_index: Option<usize>,
}

#[derive(Debug)]
struct StateArgs {
    app_id: Option<String>,
    app_index: Option<usize>,
    wait: Option<u64>,
    maximize: bool,
    unmaximize: bool,
    minimize: bool,
    unminimize: bool,
    fullscreen: bool,
    unfullscreen: bool,
    sticky: bool,
    unsticky: bool,
}

trait AppFinderArgs {
    fn app_index(&self) -> Option<usize>;
    fn app_id(&self) -> Option<&String>;
    fn wait(&self) -> Option<u64>;
}

impl AppFinderArgs for MoveArgs {
    fn app_index(&self) -> Option<usize> {
        self.app_index
    }

    fn app_id(&self) -> Option<&String> {
        self.app_id.as_ref()
    }

    fn wait(&self) -> Option<u64> {
        self.wait
    }
}

impl AppFinderArgs for StateArgs {
    fn app_index(&self) -> Option<usize> {
        self.app_index
    }

    fn app_id(&self) -> Option<&String> {
        self.app_id.as_ref()
    }

    fn wait(&self) -> Option<u64> {
        self.wait
    }
}

#[derive(Debug)]
struct WorkspaceArgs {
    workspace_name: Option<String>,
    workspace_group_index: Option<usize>,
    toggle: bool,
    next: bool,
    prev: bool,
    max: Option<usize>,
    no_dynamic: bool,
    skip_empty: bool,
}

#[derive(Debug)]
struct InfoArgs {
    json: bool,
}

enum Command {
    Info(InfoArgs),
    Move(MoveArgs),
    Activate(ActivateArgs),
    State(StateArgs),
    Workspace(WorkspaceArgs),
    MoveTo(String),
    Gap(i32),
    Minimize,
    Unminimize,
    Daemon,
}

fn find_apps<T: AppFinderArgs>(
    state: &mut AppState,
    event_queue: &mut EventQueue<AppState>,
    args: &T,
) -> Result<Vec<App>, Box<dyn Error>> {
    if let Some(app_index) = args.app_index() {
        if let Some(app) = state.apps.get(app_index) {
            Ok(vec![app.clone()])
        } else {
            Err(CliError::new(format!("App index not found: {}", app_index)))
        }
    } else if let Some(app_id) = args.app_id() {
        let sleep = std::time::Duration::from_millis(500);
        let wait_dur = args.wait().map(std::time::Duration::from_secs);
        let now = std::time::Instant::now();
        let mut apps;
        loop {
            apps = state
                .apps
                .iter()
                .filter(|app| {
                    app.app_id
                        .as_ref()
                        .map(|v| v.to_lowercase().contains(&app_id.to_lowercase()))
                        .unwrap_or_default()
                })
                .cloned()
                .collect::<Vec<_>>();

            if !apps.is_empty() {
                break;
            }

            if let Some(wait) = wait_dur {
                if now.elapsed() > wait {
                    break;
                }
                std::thread::sleep(sleep);
                event_queue.roundtrip(state)?;
            } else {
                break;
            }
        }
        if apps.is_empty() {
            return Err(CliError::new(format!("App id not found: {}", app_id)));
        }
        Ok(apps)
    } else {
        unreachable!(); // Already handled by arg parsing
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let mut pargs = pico_args::Arguments::from_env();
    if pargs.contains(["-v", "--version"]) {
        println!("Version: {}", VERSION);
        return Ok(());
    }

    let subcommand = pargs.subcommand()?;

    let command = match subcommand.as_deref() {
        Some("info") => Command::Info(InfoArgs {
            json: pargs.contains("--json"),
        }),
        Some("move") => {
            let app_id: Option<String> = pargs.opt_value_from_str(["-a", "--app-id"])?;
            let app_index: Option<usize> = pargs.opt_value_from_str(["-i", "--index"])?;

            if app_id.is_none() && app_index.is_none() {
                return Err(CliError::new(
                    "Either --app-id or --index must be provided for 'move' command.".into(),
                ));
            }
            if app_id.is_some() && app_index.is_some() {
                return Err(CliError::new(
                    "Only one of --app-id or --index can be provided for 'move' command.".into(),
                ));
            }

            Command::Move(MoveArgs {
                app_id,
                app_index,
                workspace_name: pargs.value_from_str(["-w", "--workspace"])?,
                workspace_group_index: pargs.opt_value_from_str(["-g", "--workspace-group"])?,
                output_index: pargs.opt_value_from_str(["-o", "--output-index"])?,
                wait: pargs.opt_value_from_fn("--wait", |v| v.parse())?,
            })
        }
        Some("activate") => Command::Activate(ActivateArgs {
            app_index: pargs.value_from_str(["-i", "--index"])?,
            seat_index: pargs.opt_value_from_str(["-s", "--seat"])?,
        }),
        Some("state") => {
            let app_id: Option<String> = pargs.opt_value_from_str(["-a", "--app-id"])?;
            let app_index: Option<usize> = pargs.opt_value_from_str(["-i", "--index"])?;

            if app_id.is_none() && app_index.is_none() {
                return Err(CliError::new(
                    "Either --app-id or --index must be provided for 'state' command.".into(),
                ));
            }
            if app_id.is_some() && app_index.is_some() {
                return Err(CliError::new(
                    "Only one of --app-id or --index can be provided for 'state' command.".into(),
                ));
            }

            let args = StateArgs {
                app_id,
                app_index,
                wait: pargs.opt_value_from_fn("--wait", |v| v.parse())?,
                maximize: pargs.contains("--maximize"),
                unmaximize: pargs.contains("--unmaximize"),
                minimize: pargs.contains("--minimize"),
                unminimize: pargs.contains("--unminimize"),
                fullscreen: pargs.contains("--fullscreen"),
                unfullscreen: pargs.contains("--unfullscreen"),
                sticky: pargs.contains("--sticky"),
                unsticky: pargs.contains("--unsticky"),
            };
            let num_actions = [
                args.maximize,
                args.unmaximize,
                args.minimize,
                args.unminimize,
                args.fullscreen,
                args.unfullscreen,
                args.sticky,
                args.unsticky,
            ]
            .iter()
            .filter(|&&x| x)
            .count();

            if num_actions == 0 {
                return Err(CliError::new(
                    "No action specified for 'state' command.".into(),
                ));
            }

            Command::State(args)
        }
        Some("workspace") => {
            let toggle = pargs.contains("--toggle");
            let next = pargs.contains("--next");
            let prev = pargs.contains("--prev");
            let no_dynamic = pargs.contains("--no-dynamic");
            let skip_empty = pargs.contains("--skip-empty");
            let max: Option<usize> = pargs.opt_value_from_str("--max")?;
            let workspace_name: Option<String> = pargs.opt_value_from_str(["-w", "--workspace"])?;

            if !toggle && !next && !prev && workspace_name.is_none() {
                return Err(CliError::new(
                    "One of --workspace, --toggle, --next, or --prev must be provided.".into(),
                ));
            }

            Command::Workspace(WorkspaceArgs {
                workspace_name,
                workspace_group_index: pargs.opt_value_from_str(["-g", "--workspace-group"])?,
                toggle,
                next,
                prev,
                max,
                no_dynamic,
                skip_empty,
            })
        }
        Some("move-to") => {
            Command::MoveTo(pargs.value_from_str(["-w", "--workspace"])?)
        }
        Some("gap") => {
            let increase = pargs.contains("--increase");
            let decrease = pargs.contains("--decrease");

            if !increase && !decrease {
                return Err(CliError::new("Either --increase or --decrease must be provided for 'gap' command.".into()));
            }

            Command::Gap(if increase { 10 } else { -10 })
        }
        Some("minimize") => Command::Minimize,
        Some("unminimize") => Command::Unminimize,
        Some("daemon") => {
            if pargs.contains("--verbose") {
                daemon::VERBOSE.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if pargs.contains("--log") {
                let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
                daemon::set_log_file(std::path::Path::new(&runtime_dir).join("cos-cli.log"));
            }
            if pargs.contains("--restart") {
                // kill existing daemon if running
                let path = daemon::socket_path();
                if path.exists() {
                    let _ = daemon::send_command("shutdown");
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    let _ = fs::remove_file(&path);
                }
            }
            Command::Daemon
        }
        Some("query") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            let cmd = if args.is_empty() { "info".to_string() } else { args.join(" ") };

            match daemon::send_command(&cmd) {
                Ok(response) => println!("{}", response),
                Err(e) => eprintln!("{}", e),
            }
            return Ok(());
        }
        Some("help") | None => {
            println!("{HELP}");
            return Ok(());
        }
        Some(_) => {
            return Err(CliError::new(format!(
                "Unknown subcommand: {}",
                subcommand.unwrap_or_default()
            )));
        }
    };

    // daemon runs its own wayland connection and event loop
    if matches!(command, Command::Daemon) {
        return daemon::run_daemon();
    }

    let conn = Connection::connect_to_env()?;
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut state = AppState {
        cosmic_toplevel_manager: None,
        workspace_manager: None,
        workspace_group: Vec::new(),
        apps: Vec::new(),
        outputs: Vec::new(),
        seats: Vec::new(),
        daemon_state: None,
    };
    let _registry = conn.display().get_registry(&qh, ());

    event_queue.roundtrip(&mut state)?;
    event_queue.roundtrip(&mut state)?;

    match command {
        Command::Info(args) => {
            if args.json {
                let mut json = String::new();
                json.push('{');

                json.push_str("\"apps\":[");
                for (i, app) in state.apps.iter().enumerate() {
                    if i > 0 {
                        json.push(',');
                    }
                    let states = app
                        .state
                        .iter()
                        .map(|s| format!("\"{}\"", s))
                        .collect::<Vec<_>>()
                        .join(",");
                    json.push_str(&format!(
                        "{{\"index\":{},\"app_id\":\"{}\",\"title\":\"{}\",\"state\":[{}]}}",
                        i,
                        json_escape(app.app_id.as_deref().unwrap_or_default()),
                        json_escape(app.title.as_deref().unwrap_or_default()),
                        states
                    ));
                }
                json.push_str("],");

                json.push_str("\"workspaces\":[");
                for (i, group) in state.workspace_group.iter().enumerate() {
                    if i > 0 {
                        json.push(',');
                    }
                    json.push_str(&format!("{{\"index\":{},\"workspaces\":[", i));
                    for (j, ws) in group.iter().enumerate() {
                        if j > 0 {
                            json.push(',');
                        }
                        json.push_str(&format!(
                            "{{\"name\":\"{}\",\"active\":{}}}",
                            json_escape(&ws.name),
                            ws.active,
                        ));
                    }
                    json.push_str("]}");
                }
                json.push_str("],");

                json.push_str("\"outputs\":[");
                for (i, (_, name)) in state.outputs.iter().enumerate() {
                    if i > 0 {
                        json.push(',');
                    }
                    json.push_str(&format!(
                        "{{\"index\":{},\"name\":\"{}\"}}",
                        i,
                        json_escape(name)
                    ));
                }
                json.push_str("],");

                json.push_str("\"seats\":[");
                for (i, (_, name)) in state.seats.iter().enumerate() {
                    if i > 0 {
                        json.push(',');
                    }
                    json.push_str(&format!(
                        "{{\"index\":{},\"name\":\"{}\"}}",
                        i,
                        json_escape(name)
                    ));
                }
                json.push(']');

                json.push('}');
                println!("{}", json);
            } else {
                println!("Apps:");
                for (i, app) in state.apps.iter().enumerate() {
                    let states = app
                        .state
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!(
                        "\t[{}] {} (title: {}, state: [{}])",
                        i,
                        app.app_id.as_deref().unwrap_or_default(),
                        app.title.as_deref().unwrap_or_default(),
                        states
                    );
                }
                println!("Workspaces:");
                for (i, group) in state.workspace_group.iter().enumerate() {
                    println!("\t[{i}] Group");
                    for ws in group {
                        let active = if ws.active { " (active)" } else { "" };
                        println!("\t\tWorkspace: {}{}", ws.name, active);
                    }
                }
                println!("Outputs:");
                for (i, (_, name)) in state.outputs.iter().enumerate() {
                    println!("\t[{i}] Output: {name}");
                }

                println!("Seats:");
                for (i, (_, name)) in state.seats.iter().enumerate() {
                    println!("\t[{i}] Seat: {name}");
                }
            }
        }
        Command::Move(args) => {
            let apps_to_move = find_apps(&mut state, &mut event_queue, &args)?;

            let Some(manager) = &state.cosmic_toplevel_manager else {
                return Err(CliError::new(
                    "Compositor does not support workspace management protocol.".into(),
                ));
            };
            println!("Connected to cosmic toplevel manager!");

            let Some(ws) = (if let Some(group_index) = args.workspace_group_index {
                if let Some(group) = state.workspace_group.get(group_index) {
                    group.iter().find(|ws| ws.name == args.workspace_name)
                } else {
                    return Err(CliError::new(format!(
                        "Workspace group not found: {}",
                        group_index
                    )));
                }
            } else {
                state
                    .workspace_group
                    .iter()
                    .flat_map(|v| v.iter())
                    .find(|ws| ws.name == args.workspace_name)
            }) else {
                return Err(CliError::new(format!(
                    "Workspace not found: {}",
                    args.workspace_name
                )));
            };

            let output = if let Some(index) = args.output_index {
                if let Some(output) = state.outputs.get(index) {
                    output.0.clone()
                } else {
                    return Err(CliError::new(format!("Output index not found: {}", index)));
                }
            } else {
                if state.outputs.is_empty() {
                    return Err(CliError::new("No outputs found.".to_string()));
                }
                state.outputs[0].0.clone()
            };

            for app in apps_to_move {
                println!(
                    "Move {} to {}",
                    app.app_id.as_deref().unwrap_or_default(),
                    args.workspace_name,
                );
                manager.move_to_ext_workspace(&app.handle, &ws.handle, &output);
            }

            conn.flush()?;
        }
        Command::Activate(args) => {
            let Some(manager) = &state.cosmic_toplevel_manager else {
                return Err(CliError::new(
                    "Compositor does not support toplevel management protocol.".into(),
                ));
            };
            let Some(app) = state.apps.get(args.app_index) else {
                return Err(CliError::new(format!(
                    "App index not found: {}",
                    args.app_index
                )));
            };
            let seat = if let Some(seat_index) = args.seat_index {
                state
                    .seats
                    .get(seat_index)
                    .ok_or_else(|| CliError::new(format!("Seat index not found: {}", seat_index)))?
            } else {
                state
                    .seats
                    .first()
                    .ok_or_else(|| CliError::new("No seats found.".to_string()))?
            };
            manager.activate(&app.handle, &seat.0);
            conn.flush()?;
        }
        Command::State(args) => {
            let apps_to_modify = find_apps(&mut state, &mut event_queue, &args)?;

            let Some(manager) = &state.cosmic_toplevel_manager else {
                return Err(CliError::new(
                    "Compositor does not support toplevel management protocol.".into(),
                ));
            };

            for app in apps_to_modify {
                if args.maximize {
                    manager.set_maximized(&app.handle);
                }
                if args.unmaximize {
                    manager.unset_maximized(&app.handle);
                }
                if args.minimize {
                    manager.set_minimized(&app.handle);
                }
                if args.unminimize {
                    manager.unset_minimized(&app.handle);
                }
                if args.fullscreen {
                    manager.set_fullscreen(&app.handle, None);
                }
                if args.unfullscreen {
                    manager.unset_fullscreen(&app.handle);
                }
                if args.sticky {
                    manager.set_sticky(&app.handle);
                }
                if args.unsticky {
                    manager.unset_sticky(&app.handle);
                }
            }

            conn.flush()?;
        }
        Command::Workspace(args) => {
            let statefile = Path::new("/tmp/cos-cli-last-workspace");

            // find the current active workspace number
            let current_num: usize = state
                .workspace_group
                .iter()
                .flat_map(|v| v.iter())
                .find(|ws| ws.active)
                .and_then(|ws| ws.name.parse().ok())
                .unwrap_or(1);

            // resolve the max workspace number
            let ws_max = if let Some(m) = args.max {
                m
            } else if args.no_dynamic {
                // total workspace count minus the trailing dynamic one
                let total: usize = state.workspace_group.iter().map(|g| g.len()).sum();
                if total > 1 { total - 1 } else { total }
            } else {
                // default: use total workspace count as-is
                state.workspace_group.iter().map(|g| g.len()).sum::<usize>().max(1)
            };

            // resolve the target workspace name
            let target_name = if args.toggle {
                if let Ok(prev) = fs::read_to_string(statefile) {
                    let prev = prev.trim().to_string();
                    if prev.is_empty() {
                        return Err(CliError::new("no previous workspace recorded.".into()));
                    }
                    prev
                } else {
                    return Err(CliError::new("no previous workspace recorded.".into()));
                }
            } else if args.next || args.prev {
                let mut candidate = current_num;

                loop {
                    candidate = if args.next {
                        if candidate >= ws_max { 1 } else { candidate + 1 }
                    } else {
                        if candidate <= 1 { ws_max } else { candidate - 1 }
                    };

                    // wrapped all the way around, no non-empty workspace found
                    if candidate == current_num {
                        break;
                    }

                    if !args.skip_empty {
                        break;
                    }

                    // query daemon for visible window count
                    if let Ok(response) = daemon::send_command(&format!("visible-on {}", candidate)) {
                        if let Ok(count) = response.parse::<usize>() {
                            if count > 0 {
                                break;
                            }
                        }
                    } else {
                        // daemon not running, fall back to no skipping
                        break;
                    }
                }

                candidate.to_string()
            } else {
                args.workspace_name.unwrap()
            };

            let current_name = Some(current_num.to_string());

            // find the target workspace handle
            let Some(ws) = (if let Some(group_index) = args.workspace_group_index {
                if let Some(group) = state.workspace_group.get(group_index) {
                    group.iter().find(|ws| ws.name == target_name)
                } else {
                    return Err(CliError::new(format!(
                        "Workspace group not found: {}",
                        group_index
                    )));
                }
            } else {
                state
                    .workspace_group
                    .iter()
                    .flat_map(|v| v.iter())
                    .find(|ws| ws.name == target_name)
            }) else {
                return Err(CliError::new(format!(
                    "Workspace not found: {}",
                    target_name
                )));
            };

            let Some(manager) = &state.workspace_manager else {
                return Err(CliError::new(
                    "Compositor does not support workspace management protocol.".into(),
                ));
            };

            // save current workspace to statefile before switching
            if let Some(name) = current_name {
                let _ = fs::write(statefile, &name);
            }

            ws.handle.activate();
            manager.commit();
            conn.flush()?;

            // apply per-workspace gaps
            let (_, outer) = get_gaps_for_workspace(&target_name);
            apply_gaps(&target_name);
            let _ = daemon::send_command(&format!("log gap applied: workspace {} ({})", target_name, outer));

            // auto-maximize via daemon (daemon has correct handle IDs)
            let _ = daemon::send_command(&format!("auto-maximize {}", target_name));

            // notify daemon of workspace change
            let _ = daemon::send_command(&format!("set-workspace {}", target_name));
        }
        Command::Gap(delta) => {
            // find the current active workspace
            let current_ws = state
                .workspace_group
                .iter()
                .flat_map(|v| v.iter())
                .find(|ws| ws.active)
                .map(|ws| ws.name.clone())
                .unwrap_or_else(|| "1".to_string());

            let (_, old_outer) = get_gaps_for_workspace(&current_ws);
            adjust_gaps(&current_ws, delta);
            let (_, new_outer) = get_gaps_for_workspace(&current_ws);

            let _ = daemon::send_command(&format!("log gap changed: workspace {} ({} -> {})", current_ws, old_outer, new_outer));

            // auto-maximize when gap hits zero
            if new_outer == 0 && old_outer != 0 {
                let _ = daemon::send_command(&format!("auto-maximize {}", current_ws));
            }

            // unmaximize all when gap increases
            if delta > 0 {
                let _ = daemon::send_command(&format!("unmaximize-all {}", current_ws));
            }
        }
        Command::MoveTo(workspace_name) => {
            // daemon handles this with correct handle IDs
            if daemon::send_command(&format!("do-move-to {}", workspace_name)).is_ok() {
                // daemon handled it
            } else {
                // fallback: no daemon, use direct wayland
                let Some(manager) = &state.cosmic_toplevel_manager else {
                    return Err(CliError::new("Compositor does not support toplevel management protocol.".into()));
                };

                let Some(app) = state.apps.iter().find(|a| a.state.contains(&State::Activated)) else {
                    return Err(CliError::new("no focused app found.".into()));
                };

                let Some(ws) = state.workspace_group.iter().flat_map(|v| v.iter()).find(|ws| ws.name == workspace_name) else {
                    return Err(CliError::new(format!("Workspace not found: {}", workspace_name)));
                };

                let output = if state.outputs.is_empty() {
                    return Err(CliError::new("No outputs found.".to_string()));
                } else {
                    state.outputs[0].0.clone()
                };

                manager.move_to_ext_workspace(&app.handle, &ws.handle, &output);
                conn.flush()?;
            }
        }
        Command::Minimize => {
            // daemon handles this with correct handle IDs
            if daemon::send_command("do-minimize").is_ok() {
                // daemon handled it
            } else {
                // fallback: no daemon
                let Some(manager) = &state.cosmic_toplevel_manager else {
                    return Err(CliError::new("Compositor does not support toplevel management protocol.".into()));
                };

                let Some(app) = state.apps.iter().find(|a| a.state.contains(&State::Activated)) else {
                    return Err(CliError::new("no focused app found.".into()));
                };

                manager.set_minimized(&app.handle);
                conn.flush()?;
            }
        }
        Command::Unminimize => {
            // daemon handles this with correct handle IDs
            if daemon::send_command("do-unminimize").is_ok() {
                // daemon handled it
            } else {
                // fallback: no daemon, use statefile + direct wayland
                let Some(manager) = &state.cosmic_toplevel_manager else {
                    return Err(CliError::new("Compositor does not support toplevel management protocol.".into()));
                };

                let current_ws = state.workspace_group.iter().flat_map(|v| v.iter())
                    .find(|ws| ws.active).map(|ws| ws.name.clone()).unwrap_or_default();

                let minimize_stack = Path::new("/tmp/cos-cli-minimized");
                let stack_content = fs::read_to_string(minimize_stack).unwrap_or_default();
                let mut lines: Vec<&str> = stack_content.lines().collect();

                let Some(pos) = lines.iter().rposition(|line| line.starts_with(&format!("{}:", current_ws))) else {
                    return Err(CliError::new(format!("no minimized apps on workspace {}.", current_ws)));
                };

                let entry = lines[pos];
                let parts: Vec<&str> = entry.splitn(4, ':').collect();

                if parts.len() < 3 {
                    return Err(CliError::new("corrupt minimize stack entry.".into()));
                }

                let app_id = parts[2];
                let title = if parts.len() >= 4 { parts[3] } else { "" };

                let app = state.apps.iter().find(|a| {
                    a.app_id.as_deref() == Some(app_id) && a.state.contains(&State::Minimized)
                        && (title.is_empty() || a.title.as_deref() == Some(title))
                });

                let Some(app) = app else {
                    lines.remove(pos);
                    let _ = fs::write(minimize_stack, lines.join("\n") + if lines.is_empty() { "" } else { "\n" });
                    return Err(CliError::new(format!("app '{}' is no longer minimized.", app_id)));
                };

                lines.remove(pos);
                let _ = fs::write(minimize_stack, lines.join("\n") + if lines.is_empty() { "" } else { "\n" });

                manager.unset_minimized(&app.handle);
                conn.flush()?;
            }
        }
        Command::Daemon => unreachable!(),
    };

    Ok(())
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}


const GAPS_CONFIG: &str = ".config/cosmic/cos-cli/gaps";
const COSMIC_GAPS: &str = ".config/cosmic/com.system76.CosmicTheme.Dark/v1/gaps";
const DEFAULT_GAP: (u32, u32) = (0, 30);
const AUTO_MAXIMIZE_CONFIG: &str = ".config/cosmic/cos-cli/auto_maximize";


fn gaps_config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/x".to_string());
    Path::new(&home).join(GAPS_CONFIG)
}


fn cosmic_gaps_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/x".to_string());
    Path::new(&home).join(COSMIC_GAPS)
}


pub fn is_auto_maximize_enabled() -> bool {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/x".to_string());
    let path = Path::new(&home).join(AUTO_MAXIMIZE_CONFIG);

    // create with default true if missing
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&path, "true\n");
        return true;
    }

    match fs::read_to_string(&path) {
        Ok(content) => content.trim() != "false",
        Err(_) => true,
    }
}


fn read_gaps_config() -> Vec<(String, u32, u32)> {
    // read the gaps config file, returns a list of (workspace_name, inner, outer) tuples
    // format: workspace:inner,outer (one per line, # comments, "default" key for fallback)

    let path = gaps_config_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() != 2 {
            return None;
        }
        let vals: Vec<&str> = parts[1].split(',').collect();
        if vals.len() != 2 {
            return None;
        }
        let inner = vals[0].trim().parse::<u32>().ok()?;
        let outer = vals[1].trim().parse::<u32>().ok()?;
        Some((parts[0].trim().to_string(), inner, outer))
    }).collect()
}


pub fn get_gaps_for_workspace(workspace: &str) -> (u32, u32) {
    let config = read_gaps_config();

    // look for exact workspace match first
    if let Some((_, inner, outer)) = config.iter().find(|(ws, _, _)| ws == workspace) {
        return (*inner, *outer);
    }

    // fall back to default entry
    if let Some((_, inner, outer)) = config.iter().find(|(ws, _, _)| ws == "default") {
        return (*inner, *outer);
    }

    DEFAULT_GAP
}


fn apply_gaps(workspace: &str) {
    let config_path = gaps_config_path();

    // create the config dir and a default template if missing
    if !config_path.exists() {
        if let Some(parent) = config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let template = "# per-workspace gaps: workspace:inner,outer\n# \"default\" is used as fallback for unlisted workspaces\ndefault:0,30\n";
        let _ = fs::write(&config_path, template);
    }

    let (inner, outer) = get_gaps_for_workspace(workspace);
    let content = format!("({}, {})\n", inner, outer);
    let _ = fs::write(cosmic_gaps_path(), content);
}


fn adjust_gaps(workspace: &str, delta: i32) {
    let (inner, outer) = get_gaps_for_workspace(workspace);

    let new_outer = (outer as i32 + delta).max(0) as u32;

    // update or add the entry for this workspace
    let path = gaps_config_path();
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;

    if let Ok(content) = fs::read_to_string(&path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                if let Some(ws) = trimmed.split(':').next() {
                    if ws.trim() == workspace {
                        lines.push(format!("{}:{},{}", workspace, inner, new_outer));
                        found = true;
                        continue;
                    }
                }
            }
            lines.push(line.to_string());
        }
    }

    if !found {
        lines.push(format!("{}:{},{}", workspace, inner, new_outer));
    }

    lines.push(String::new());
    let _ = fs::write(&path, lines.join("\n"));

    // apply immediately
    let content = format!("({}, {})\n", inner, new_outer);
    let _ = fs::write(cosmic_gaps_path(), content);
}
