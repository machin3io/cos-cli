use cosmic_protocols::toplevel_info::v1::client::{
    zcosmic_toplevel_handle_v1, zcosmic_toplevel_info_v1,
};
use cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1;

use wayland_client::protocol::wl_seat;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, event_created_child,
    protocol::{wl_output, wl_registry},
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1, ext_foreign_toplevel_list_v1,
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1, ext_workspace_handle_v1, ext_workspace_manager_v1,
};

use crate::{App, AppState, State, Workspace};
use crate::daemon;

impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" => {
                    proxy.bind::<wl_seat::WlSeat, _, _>(name, 4, qh, ());
                }
                "wl_output" => {
                    proxy.bind::<wl_output::WlOutput, _, _>(name, 4, qh, ());
                }
                "zcosmic_toplevel_info_v1" => {
                    state.cosmic_toplevel_info = Some(
                        proxy.bind::<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, _, _>(
                            name,
                            3,
                            qh,
                            (),
                        ),
                    );
                }
                "ext_foreign_toplevel_list_v1" => {
                    proxy.bind::<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, _, _>(
                        name,
                        1,
                        qh,
                        (),
                    );
                }
                "ext_workspace_manager_v1" => {
                    state.workspace_manager = Some(
                        proxy.bind::<ext_workspace_manager_v1::ExtWorkspaceManagerV1, _, _>(
                            name,
                            version,
                            qh,
                            (),
                        ),
                    );
                }
                "zcosmic_toplevel_manager_v1" => {
                    state.cosmic_toplevel_manager = Some(
                        proxy.bind::<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, _, _>(
                            name,
                            version,
                            qh,
                            (),
                        ),
                    );
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for AppState {
    fn event(
        app_data: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<AppState>,
    ) {
        if let wl_output::Event::Name { name } = event {
            app_data.outputs.push((output.clone(), name));
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for AppState {
    fn event(
        app_data: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<AppState>,
    ) {
        if let wl_seat::Event::Name { name } = event {
            app_data.seats.push((seat.clone(), name));
        }
    }
}

impl Dispatch<ext_workspace_manager_v1::ExtWorkspaceManagerV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &ext_workspace_manager_v1::ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let ext_workspace_manager_v1::Event::WorkspaceGroup { workspace_group: _ } = event {
            state.workspace_group.push(Vec::new())
        }
    }

    event_created_child!(
        AppState,
        ext_workspace_manager_v1::ExtWorkspaceManagerV1,
        [
            ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ext_workspace_handle_v1::ExtWorkspaceHandleV1, ()),
            ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1, ()),
        ]
    );
}

impl Dispatch<ext_workspace_handle_v1::ExtWorkspaceHandleV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &ext_workspace_handle_v1::ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_workspace_handle_v1::Event::Name { name } => {
                if let Some(current_group) = state.workspace_group.last_mut() {
                    current_group.push(Workspace {
                        name,
                        handle: proxy.clone(),
                        active: false,
                    });
                }
            }
            ext_workspace_handle_v1::Event::State { state: ws_state } => {
                let is_active = matches!(
                    ws_state,
                    wayland_client::WEnum::Value(ext_workspace_handle_v1::State::Active)
                );

                for group in state.workspace_group.iter_mut() {
                    if let Some(ws) = group.iter_mut().find(|w| w.handle == *proxy) {
                        ws.active = is_active;

                        // notify daemon of workspace change
                        if is_active {
                            if let Some(ref ds) = state.daemon_state {
                                if let Ok(mut ds) = ds.lock() {
                                    ds.on_workspace_changed(&ws.name);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1,
        _event: ext_workspace_group_handle_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, ()> for AppState {
    fn event(
        _app_data: &mut AppState,
        _workspace: &zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1,
        _event: zcosmic_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, ()> for AppState {
    fn event(
        _app_data: &mut Self,
        _info: &zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1,
        _event: zcosmic_toplevel_info_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<AppState>,
    ) {
        // v3 binding: toplevel discovery is handled via ext_foreign_toplevel_list_v1,
        // cosmic handles are created via get_cosmic_toplevel in that handler
    }

    event_created_child!(
        AppState,
        zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1,
        [
            zcosmic_toplevel_info_v1::EVT_TOPLEVEL_OPCODE => (zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()),
        ]
    );
}


impl Dispatch<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, ()> for AppState {
    fn event(
        _app_data: &mut Self,
        _list: &ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        _event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<AppState>,
    ) {
        // toplevel events are handled via ext_foreign_toplevel_handle_v1
    }

    event_created_child!(
        AppState,
        ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        [
            ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()),
        ]
    );
}


impl Dispatch<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()> for AppState {
    fn event(
        app_data: &mut Self,
        foreign_handle: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<AppState>,
    ) {
        let fid = format!("{:?}", foreign_handle.id());

        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                app_data.foreign_toplevel_props.entry(fid).or_insert_with(|| (String::new(), String::new())).1 = title;
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                app_data.foreign_toplevel_props.entry(fid).or_insert_with(|| (String::new(), String::new())).0 = app_id;
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {
                // only create a cosmic handle on the first Done event per foreign toplevel
                if app_data.foreign_toplevel_done.contains(&fid) {
                    return;
                }
                app_data.foreign_toplevel_done.insert(fid.clone());

                let (app_id, title) = app_data.foreign_toplevel_props.remove(&fid).unwrap_or_default();

                if let Some(ref info) = app_data.cosmic_toplevel_info {
                    let cosmic_handle = info.get_cosmic_toplevel(foreign_handle, qh, ());
                    let hid = daemon::handle_id(&cosmic_handle);

                    // store foreign → cosmic handle mapping for close event forwarding
                    app_data.foreign_to_cosmic.insert(fid.clone(), hid.clone());

                    // notify daemon of new window with title/app_id
                    if let Some(ref ds) = app_data.daemon_state {
                        if let Ok(mut ds) = ds.lock() {
                            ds.on_window_created(&hid);
                            ds.on_app_id_changed(&hid, &app_id);
                            ds.on_title_changed(&hid, &title);
                        }
                    }

                    app_data.apps.push(App {
                        handle: cosmic_handle,
                        title: Some(title),
                        app_id: Some(app_id),
                        outputs: Vec::new(),
                        state: Vec::new(),
                    });
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                app_data.foreign_toplevel_done.remove(&fid);
                app_data.foreign_toplevel_props.remove(&fid);

                // forward close to daemon and remove the cosmic handle
                if let Some(cosmic_hid) = app_data.foreign_to_cosmic.remove(&fid) {
                    if let Some(ref ds) = app_data.daemon_state {
                        if let Ok(mut ds) = ds.lock() {
                            ds.on_window_closed(&cosmic_hid);
                        }
                    }
                    app_data.apps.retain(|a| daemon::handle_id(&a.handle) != cosmic_hid);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()> for AppState {
    fn event(
        app_data: &mut Self,
        toplevel: &zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<AppState>,
    ) {
        match event {
            zcosmic_toplevel_handle_v1::Event::Title { title } => {
                if let Some(info) = app_data.apps.iter_mut().find(|t| &t.handle == toplevel) {
                    info.title = Some(title.clone());
                }

                // notify daemon
                if let Some(ref ds) = app_data.daemon_state {
                    if let Ok(mut ds) = ds.lock() {
                        ds.on_title_changed(&daemon::handle_id(toplevel), &title);
                    }
                }
            }
            zcosmic_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(info) = app_data.apps.iter_mut().find(|t| &t.handle == toplevel) {
                    info.app_id = Some(app_id.clone());
                }

                // notify daemon
                if let Some(ref ds) = app_data.daemon_state {
                    if let Ok(mut ds) = ds.lock() {
                        ds.on_app_id_changed(&daemon::handle_id(toplevel), &app_id);
                    }
                }
            }
            zcosmic_toplevel_handle_v1::Event::OutputEnter { output } => {
                if let Some(info) = app_data.apps.iter_mut().find(|t| &t.handle == toplevel) {
                    info.outputs.push(output);
                }
            }
            zcosmic_toplevel_handle_v1::Event::State { state } => {
                let parsed_states: Vec<State> = state
                    .chunks_exact(4)
                    .map(|chunk| u32::from_ne_bytes(chunk.try_into().unwrap()))
                    .flat_map(|val| State::try_from(val).ok())
                    .collect();

                if let Some(info) = app_data.apps.iter_mut().find(|t| &t.handle == toplevel) {
                    info.state = parsed_states.clone();
                }

                // notify daemon
                if let Some(ref ds) = app_data.daemon_state {
                    if let Ok(mut ds) = ds.lock() {
                        ds.on_state_changed(&daemon::handle_id(toplevel), &parsed_states);
                    }
                }
            }
            zcosmic_toplevel_handle_v1::Event::Closed => {
                // notify daemon before removing
                if let Some(ref ds) = app_data.daemon_state {
                    if let Ok(mut ds) = ds.lock() {
                        ds.on_window_closed(&daemon::handle_id(toplevel));
                    }
                }

                app_data.apps.retain(|a| &a.handle != toplevel);
            }
            zcosmic_toplevel_handle_v1::Event::ExtWorkspaceEnter { workspace } => {
                // resolve workspace handle to name
                let ws_name = app_data.workspace_group.iter()
                    .flat_map(|g| g.iter())
                    .find(|ws| ws.handle == workspace)
                    .map(|ws| ws.name.clone());

                if let Some(name) = ws_name {
                    if let Some(ref ds) = app_data.daemon_state {
                        if let Ok(mut ds) = ds.lock() {
                            ds.on_workspace_enter(&daemon::handle_id(toplevel), &name);
                        }
                    }
                }
            }
            zcosmic_toplevel_handle_v1::Event::ExtWorkspaceLeave { workspace } => {
                let ws_name = app_data.workspace_group.iter()
                    .flat_map(|g| g.iter())
                    .find(|ws| ws.handle == workspace)
                    .map(|ws| ws.name.clone());

                if let Some(name) = ws_name {
                    if let Some(ref ds) = app_data.daemon_state {
                        if let Ok(mut ds) = ds.lock() {
                            ds.on_workspace_leave(&daemon::handle_id(toplevel), &name);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
