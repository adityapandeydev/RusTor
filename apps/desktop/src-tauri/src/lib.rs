mod lifecycle;
mod search;
mod service;
mod settings;
mod startup;

use service::{
    AddSource, AppSnapshot, CommandError, DesktopState, PreparedSelection, SaveSettingsRequest,
    Selection, StartRequest,
};
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
async fn prepare_torrent(
    source: AddSource,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<PreparedSelection, CommandError> {
    state.prepare(source).await
}

#[tauri::command]
async fn discard_prepared(
    id: String,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<(), CommandError> {
    state.discard_prepared(&id).await;
    Ok(())
}

#[tauri::command]
async fn start_prepared(
    request: StartRequest,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<rustor_torrent::AddOutcome, CommandError> {
    state.start(request).await
}

#[tauri::command]
async fn save_settings(
    request: SaveSettingsRequest,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<AppSnapshot, CommandError> {
    state.save_settings(request).await
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
async fn retry_torrent(
    id: String,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<(), CommandError> {
    state.retry(&id).await
}

#[tauri::command]
async fn remove_torrent(
    id: String,
    state: tauri::State<'_, Arc<DesktopState>>,
) -> Result<(), CommandError> {
    state.remove(&id).await
}

#[tauri::command]
async fn start_search(
    query: String,
    category: Option<rustor_search::Category>,
    state: tauri::State<'_, search::DesktopSearch>,
) -> Result<rustor_search::SearchSnapshot, CommandError> {
    Ok((*state.start(query, category).await?).clone())
}
#[tauri::command]
async fn get_search_snapshot(
    id: u64,
    state: tauri::State<'_, search::DesktopSearch>,
) -> Result<rustor_search::SearchSnapshot, CommandError> {
    Ok((*state.snapshot(id).await?).clone())
}
#[tauri::command]
async fn cancel_search(
    id: u64,
    state: tauri::State<'_, search::DesktopSearch>,
) -> Result<bool, CommandError> {
    Ok(state.cancel(id).await)
}

pub fn run() {
    let login_launch = startup::is_login_launch(&std::env::args().collect::<Vec<_>>());
    let autostart = tauri_plugin_autostart::Builder::new().arg(startup::LOGIN_ARGUMENT);
    let autostart = autostart.app_name(startup::ENTRY_NAME);
    let app = tauri::Builder::default()
        // Must run before other plugins: the second process never opens an engine.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !startup::is_login_launch(&args)
                && let Err(error) = lifecycle::show(app)
            {
                lifecycle::report(app, CommandError::new("window_unavailable", error.to_string()));
            }
        }))
        .plugin(autostart.build())
        .manage(lifecycle::Lifecycle::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            pick_torrent,
            pick_destination,
            prepare_torrent,
            discard_prepared,
            start_prepared,
            pause_torrent,
            resume_torrent,
            remove_torrent,
            retry_torrent,
            save_settings,
            start_search,
            get_search_snapshot,
            cancel_search,
            quit_app
        ])
        .setup(move |app| {
            let download_dir = app.path().download_dir()?.join("RusTor");
            let state_dir = app.path().app_local_data_dir()?.join("engine");
            // Development smoke tests can isolate state from the normal app profile.
            // Release binaries always use the OS application-data directory.
            #[cfg(debug_assertions)]
            let state_dir = std::env::var_os("RUSTOR_DEV_STATE_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(state_dir);
            let state = Arc::new(DesktopState::with_startup(rustor_torrent::EngineConfig::new(
                download_dir,
                state_dir,
            ), Arc::new(startup::NativeRegistration::new(app.handle().clone()))));
            app.manage(state.clone());
            app.manage(search::DesktopSearch::new());
            match lifecycle::setup_tray(app.handle()) {
                Ok(()) => state.tray_available.store(true, Ordering::Release),
                Err(error) => lifecycle::report(app.handle(), CommandError::new("tray_unavailable", format!(
                    "The system tray could not be started ({error}). Closing RusTor will quit; keep the window open to continue transfers."))),
            }
            if !login_launch {
                lifecycle::show(app.handle())?;
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                state.initialize().await;
                if login_launch {
                    let snapshot = state.snapshot().await;
                    let minimize = startup::should_minimize(
                        true, snapshot.startup.enabled == Some(true), snapshot.settings.start_minimized,
                        snapshot.tray_available, handle.state::<lifecycle::Lifecycle>().foreground_requested(),
                        matches!(snapshot.phase, service::EnginePhase::Ready),
                    );
                    if minimize {
                        #[cfg(target_os = "linux")]
                        if let Some(window) = handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.minimize();
                        }
                    } else if let Err(error) = lifecycle::show(&handle) {
                        lifecycle::report(&handle, CommandError::new("window_unavailable", error.to_string()));
                    }
                }
                while !state.stopping.load(Ordering::Acquire) {
                    let _ = handle.emit("torrent-snapshot", state.snapshot().await);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" && let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle();
                let state = app.state::<Arc<DesktopState>>();
                if app.state::<lifecycle::Lifecycle>().should_background(&state) {
                    // Linux desktops can lack a visible status notifier even when creation succeeds.
                    // Keep a taskbar entry there as a recovery path.
                    #[cfg(target_os = "linux")]
                    let result = window.minimize();
                    #[cfg(not(target_os = "linux"))]
                    let result = window.hide();
                    if let Err(error) = result {
                        lifecycle::report(app, CommandError::new("window_unavailable", format!(
                            "Could not move RusTor to the background ({error}). The app is still running; use Quit RusTor to exit.")));
                    }
                } else {
                    lifecycle::quit(app);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Could not start the RusTor desktop application");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = &event
            && !app.state::<lifecycle::Lifecycle>().complete()
        {
            api.prevent_exit();
            lifecycle::quit(app);
        }
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            if let Err(error) = lifecycle::show(app) {
                lifecycle::report(
                    app,
                    CommandError::new("window_unavailable", error.to_string()),
                );
            }
        }
    });
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    lifecycle::quit(&app);
}
