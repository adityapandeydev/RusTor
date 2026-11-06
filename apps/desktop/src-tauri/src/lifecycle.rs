use crate::{
    search::DesktopSearch,
    service::{CommandError, DesktopState},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

#[derive(Default)]
pub struct Lifecycle {
    quitting: AtomicBool,
    complete: AtomicBool,
    foreground_requested: AtomicBool,
}

impl Lifecycle {
    pub fn foreground_requested(&self) -> bool {
        self.foreground_requested.load(Ordering::Acquire)
    }
    pub fn complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }
    fn begin_quit(&self) -> bool {
        !self.quitting.swap(true, Ordering::AcqRel)
    }
    pub fn should_background(&self, state: &DesktopState) -> bool {
        !self.quitting.load(Ordering::Acquire)
            && !state.stopping.load(Ordering::Acquire)
            && state.tray_available.load(Ordering::Acquire)
            && state.continue_in_background.load(Ordering::Acquire)
    }
}

pub fn quit(app: &tauri::AppHandle) {
    if !app.state::<Lifecycle>().begin_quit() {
        return;
    }
    let state = app.state::<Arc<DesktopState>>().inner().clone();
    state.stopping.store(true, Ordering::Release);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        app.state::<DesktopSearch>().shutdown().await;
        state.shutdown().await;
        app.state::<Lifecycle>()
            .complete
            .store(true, Ordering::Release);
        app.exit(0);
    });
}

pub fn show(app: &tauri::AppHandle) -> tauri::Result<()> {
    app.state::<Lifecycle>()
        .foreground_requested
        .store(true, Ordering::Release);
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
    }
    Ok(())
}

pub fn report(app: &tauri::AppHandle, error: CommandError) {
    let state = app.state::<Arc<DesktopState>>().inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        state.report_background_error(error).await;
        // Errors from a hidden-window action must be visible, not silently discarded.
        if let Err(error) = show(&app) {
            eprintln!("Could not reopen RusTor: {error}");
        }
    });
}

pub fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open RusTor", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause_all", "Pause all", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume_all", "Resume all", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit RusTor", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pause, &resume, &quit_item])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("application icon".into()))?;
    TrayIconBuilder::with_id("rustor-tray")
        .icon(icon)
        .tooltip("RusTor")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => {
                if let Err(error) = show(app) {
                    report(
                        app,
                        CommandError::new(
                            "window_unavailable",
                            format!("Could not open the window: {error}"),
                        ),
                    );
                }
            }
            "quit" => quit(app),
            "pause_all" | "resume_all" => {
                let pause = event.id.as_ref() == "pause_all";
                let app = app.clone();
                let state = app.state::<Arc<DesktopState>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = state.control_all(pause).await {
                        report(&app, error);
                    }
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let app = tray.app_handle();
                if let Err(error) = show(app) {
                    report(
                        app,
                        CommandError::new(
                            "window_unavailable",
                            format!("Could not open the window: {error}"),
                        ),
                    );
                }
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_only_backgrounds_with_an_available_tray_and_explicit_preference() {
        let state = DesktopState::new(rustor_torrent::EngineConfig::new(
            std::env::temp_dir(),
            std::env::temp_dir(),
        ));
        let lifecycle = Lifecycle::default();
        assert!(!lifecycle.should_background(&state));
        state.continue_in_background.store(true, Ordering::Release);
        assert!(!lifecycle.should_background(&state));
        state.tray_available.store(true, Ordering::Release);
        assert!(lifecycle.should_background(&state));
        assert!(lifecycle.begin_quit());
        assert!(!lifecycle.begin_quit());
        assert!(!lifecycle.complete());
        assert!(!lifecycle.should_background(&state));
        let lifecycle = Lifecycle::default();
        state.stopping.store(true, Ordering::Release);
        assert!(!lifecycle.should_background(&state));
    }
}
