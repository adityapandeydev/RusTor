mod service;

use service::{AddRequest, AppSnapshot, CommandError, DesktopState, Selection};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
async fn get_snapshot(
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<AppSnapshot, CommandError> {
    Ok(state.snapshot().await)
}

#[tauri::command]
async fn pick_torrent(
    window: tauri::Window,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<Option<Selection>, CommandError> {
    let file = window
        .dialog()
        .file()
        .set_title("Choose a torrent file")
        .add_filter("Torrent metadata", &["torrent"])
        .set_parent(&window)
        .blocking_pick_file();
    match file {
        Some(file) => Ok(Some(
            state
                .select(
                    file.into_path().map_err(|_| {
                        CommandError::new("invalid_selection", "Choose a local torrent file.")
                    })?,
                    false,
                )
                .await?,
        )),
        None => Ok(None),
    }
}

#[tauri::command]
async fn pick_destination(
    window: tauri::Window,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<Option<Selection>, CommandError> {
    let folder = window
        .dialog()
        .file()
        .set_title("Choose a download folder")
        .set_parent(&window)
        .blocking_pick_folder();
    match folder {
        Some(folder) => Ok(Some(
            state
                .select(
                    folder.into_path().map_err(|_| {
                        CommandError::new("invalid_selection", "Choose a local download folder.")
                    })?,
                    true,
                )
                .await?,
        )),
        None => Ok(None),
    }
}

#[tauri::command]
async fn add_torrent(
    request: AddRequest,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<rustor_torrent::AddOutcome, CommandError> {
    state.add(request).await
}

#[tauri::command]
async fn pause_torrent(
    id: String,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<(), CommandError> {
    state.pause(&id).await
}

#[tauri::command]
async fn resume_torrent(
    id: String,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<(), CommandError> {
    state.resume(&id).await
}

#[tauri::command]
async fn remove_torrent(
    id: String,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<(), CommandError> {
    state.remove(&id).await
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            pick_torrent,
            pick_destination,
            add_torrent,
            pause_torrent,
            resume_torrent,
            remove_torrent
        ])
        .setup(|app| {
            let download_dir = app.path().download_dir()?.join("RusTor");
            let state_dir = app.path().app_local_data_dir()?.join("engine");
            // Development smoke tests can isolate state from the normal app profile.
            // Release binaries always use the OS application-data directory.
            #[cfg(debug_assertions)]
            let state_dir = std::env::var_os("RUSTOR_DEV_STATE_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(state_dir);
            let state = Arc::new(DesktopState::new(rustor_torrent::EngineConfig::new(
                download_dir,
                state_dir,
            )));
            app.manage(state.clone());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                state.initialize().await;
                while !state.stopping.load(Ordering::Acquire) {
                    let _ = handle.emit("torrent-snapshot", state.snapshot().await);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Could not start the RusTor desktop application");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event
            && code.is_none()
        {
            api.prevent_exit();
            let state = app.state::<Arc<DesktopState>>().inner().clone();
            if !state.stopping.swap(true, Ordering::AcqRel) {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    state.shutdown().await;
                    app.exit(0);
                });
            }
        }
    });
}
