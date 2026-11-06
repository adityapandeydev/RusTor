use crate::{
    lifecycle::Lifecycle,
    search::DesktopSearch,
    service::{CommandError, DesktopState},
};
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
#[cfg(test)]
#[path = "updater_test_fixture.rs"]
mod test_fixture;

const ENDPOINT: &str =
    "https://github.com/adityapandeydev/RusTor/releases/latest/download/latest.json";
const PUBLIC_KEY: &str = include_str!("../updater.pub");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Unavailable,
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Installing,
    Failed,
}

#[derive(Clone, Serialize)]
pub struct Snapshot {
    revision: u64,
    pub phase: Phase,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<CommandError>,
    pub retry: Option<&'static str>,
    pub transfers_stopped: bool,
}

struct Pending {
    update: Update,
    bytes: Option<Vec<u8>>,
}
pub struct Updates {
    status: Mutex<Snapshot>,
    pending: tokio::sync::Mutex<Option<Pending>>,
}

impl Updates {
    pub fn new<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> Self {
        let error = availability().err();
        Self {
            status: Mutex::new(Snapshot {
                revision: 0,
                phase: if error.is_some() {
                    Phase::Unavailable
                } else {
                    Phase::Idle
                },
                version: None,
                notes: None,
                downloaded: 0,
                total: None,
                error,
                retry: None,
                transfers_stopped: false,
            }),
            pending: tokio::sync::Mutex::new(None),
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        self.status.lock().unwrap().clone()
    }
    fn change<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        change: impl FnOnce(&mut Snapshot),
    ) {
        let snapshot = {
            let mut status = self.status.lock().unwrap();
            change(&mut status);
            status.revision += 1;
            status.clone()
        };
        let _ = app.emit("update-status", snapshot);
    }
    fn fail<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        error: CommandError,
        retry: &'static str,
    ) -> CommandError {
        self.change(app, |status| {
            status.phase = Phase::Failed;
            status.error = Some(error.clone());
            status.retry = Some(retry);
        });
        error
    }
    pub async fn check(&self, app: &tauri::AppHandle) -> Result<Snapshot, CommandError> {
        let mut pending = self.pending.try_lock().map_err(|_| busy())?;
        availability()?;
        if app
            .state::<Arc<DesktopState>>()
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(CommandError::new("stopped", "RusTor is shutting down."));
        }
        if self.snapshot().transfers_stopped {
            return Err(CommandError::new(
                "stopped",
                "Transfers are stopped. Retry installation or quit and reopen RusTor.",
            ));
        }
        *pending = None;
        self.change(app, |status| {
            status.phase = Phase::Checking;
            status.error = None;
            status.retry = None;
            status.version = None;
            status.notes = None;
            status.downloaded = 0;
            status.total = None;
        });
        let result = async {
            let updater = app
                .updater_builder()
                .pubkey(PUBLIC_KEY.trim())
                .endpoints(vec![ENDPOINT.parse().expect("constant HTTPS updater URL")])
                .map_err(check_error)?
                .timeout(Duration::from_secs(300))
                .build()
                .map_err(check_error)?;
            let update = tokio::time::timeout(Duration::from_secs(30), updater.check())
                .await
                .map_err(|_| {
                    CommandError::new(
                        "update_check_failed",
                        "The update check timed out. Check your connection and try again.",
                    )
                })?
                .map_err(check_error)?;
            if let Some(update) = update {
                validate_download_url(update.download_url.as_str(), &update.version)?;
                self.change(app, |status| {
                    status.phase = Phase::Available;
                    status.version = Some(update.version.clone());
                    status.notes = update.body.clone();
                });
                *pending = Some(Pending {
                    update,
                    bytes: None,
                });
            } else {
                self.change(app, |status| status.phase = Phase::Current);
            }
            Ok(self.snapshot())
        }
        .await;
        result.map_err(|error| self.fail(app, error, "check"))
    }
    pub async fn download(&self, app: &tauri::AppHandle) -> Result<Snapshot, CommandError> {
        let mut pending = self.pending.try_lock().map_err(|_| busy())?;
        let pending = pending
            .as_mut()
            .ok_or_else(|| CommandError::new("update_missing", "Check for an update first."))?;
        if pending.bytes.is_some() {
            return Ok(self.snapshot());
        }
        self.change(app, |status| {
            status.phase = Phase::Downloading;
            status.downloaded = 0;
            status.total = None;
            status.error = None;
            status.retry = None;
        });
        // The plugin returns bytes only after signature AND signed-version
        // verification. Download completion alone never enables installation.
        let mut last_emit = std::time::Instant::now();
        let result = pending
            .update
            .download(
                |length, total| {
                    let snapshot = {
                        let mut status = self.status.lock().unwrap();
                        status.downloaded = status.downloaded.saturating_add(length as u64);
                        status.total = total.filter(|value| *value > 0);
                        status.clone()
                    };
                    if last_emit.elapsed() >= Duration::from_millis(100) {
                        self.change(app, |status| {
                            status.downloaded = snapshot.downloaded;
                            status.total = snapshot.total;
                        });
                        last_emit = std::time::Instant::now();
                    }
                },
                || {},
            )
            .await;
        match result {
            Ok(bytes) => {
                pending.bytes = Some(bytes);
                self.change(app, |status| { status.phase = Phase::Ready; status.error = None; });
                Ok(self.snapshot())
            }
            Err(_) => Err(self.fail(app, CommandError::new("update_download_failed", "The update could not be downloaded or verified. Check your connection and try again. The installed app has not been replaced."), "download")),
        }
    }
    pub async fn install<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<(), CommandError> {
        let mut pending = self.pending.try_lock().map_err(|_| busy())?;
        let pending = pending
            .as_mut()
            .filter(|pending| pending.bytes.is_some())
            .ok_or_else(|| {
                CommandError::new(
                    "update_not_verified",
                    "Download and verify an update before installing it.",
                )
            })?;
        if pending
            .update
            .verify(pending.bytes.as_ref().expect("verified bytes"))
            .is_err()
        {
            pending.bytes = None;
            return Err(self.fail(app, CommandError::new("update_verification_failed", "The downloaded update could not be verified. Download it again; your current session has not been stopped."), "download"));
        }
        app.state::<Lifecycle>().begin_update()?;
        self.change(app, |status| {
            status.phase = Phase::Installing;
            status.error = None;
            status.retry = None;
        });
        let desktop = app.state::<Arc<DesktopState>>().inner().clone();
        if let Err(error) = desktop.shutdown_for_update().await {
            app.state::<Lifecycle>().update_failed();
            return Err(self.fail(app, CommandError::new("update_save_failed", format!("Update installation was cancelled because the session could not be saved. {}", error.message)), "install"));
        }
        app.state::<DesktopSearch>().shutdown().await;
        self.change(app, |status| status.transfers_stopped = true);
        // Windows exits only after successfully launching the NSIS installer;
        // macOS/Linux return after replacement and need an explicit relaunch.
        if pending
            .update
            .install(pending.bytes.as_ref().expect("verified bytes"))
            .is_err()
        {
            app.state::<Lifecycle>().update_failed();
            return Err(self.fail(app, CommandError::new("update_install_failed", "The installer could not finish. Transfers are stopped; retry installation, or quit and reopen RusTor. Your saved session and files are kept."), "install"));
        }
        app.state::<Lifecycle>().finish_update();
        app.restart();
    }
}

fn availability() -> Result<(), CommandError> {
    if PUBLIC_KEY.trim().is_empty() {
        return Err(CommandError::new(
            "updater_not_configured",
            "In-app updates will be enabled after release signing is configured. You can install a newer release over this version without uninstalling it.",
        ));
    }
    use tauri::utils::config::BundleType;
    let supported = !cfg!(debug_assertions)
        && matches!(
            tauri::utils::platform::bundle_type(),
            Some(BundleType::Nsis | BundleType::App | BundleType::AppImage)
        );
    if !supported {
        return Err(CommandError::new(
            "updater_unsupported",
            "Use the installer to update this development, standalone or package-manager build. In-app updates support installed Windows, macOS apps and Linux AppImages.",
        ));
    }
    Ok(())
}
fn validate_download_url(value: &str, version: &str) -> Result<(), CommandError> {
    let prefix = format!("https://github.com/adityapandeydev/RusTor/releases/download/v{version}/");
    if value.strip_prefix(&prefix).is_some_and(|file| {
        !file.is_empty()
            && file
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    }) {
        Ok(())
    } else {
        Err(CommandError::new(
            "update_manifest_invalid",
            "The update manifest contains an unsupported download address.",
        ))
    }
}
fn busy() -> CommandError {
    CommandError::new("update_busy", "An update operation is already running.")
}
fn check_error(_: tauri_plugin_updater::Error) -> CommandError {
    CommandError::new(
        "update_check_failed",
        "Could not check for updates. Check your connection and try again; the release feed may not be available yet.",
    )
}

#[tauri::command]
pub fn get_update_status(state: tauri::State<'_, Updates>) -> Snapshot {
    state.snapshot()
}
#[tauri::command]
pub async fn check_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, Updates>,
) -> Result<Snapshot, CommandError> {
    state.check(&app).await
}
#[tauri::command]
pub async fn download_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, Updates>,
) -> Result<Snapshot, CommandError> {
    state.download(&app).await
}
#[tauri::command]
pub async fn install_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, Updates>,
) -> Result<(), CommandError> {
    state.install(&app).await
}

#[cfg(test)]
mod tests {
    use super::test_fixture as fixture;
    use super::*;

    // Local HTTP is allowed only in these tests, never in the production builder.
    async fn server(mode: &'static str) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let base = endpoint.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0; 4096];
                let count = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                let (status, body): (&str, Vec<u8>) = if request.starts_with("GET /payload ") {
                    (
                        "200 OK",
                        if mode == "corrupt" {
                            b"corrupt".to_vec()
                        } else {
                            fixture::BYTES.to_vec()
                        },
                    )
                } else if mode == "current" {
                    ("204 No Content", vec![])
                } else if mode == "malformed" {
                    ("200 OK", b"not json".to_vec())
                } else {
                    let version = if mode == "wrong_version" {
                        "99.0.1"
                    } else {
                        "99.0.0"
                    };
                    let signature = if mode == "wrong_signature" {
                        "invalid"
                    } else {
                        fixture::SIGNATURE
                    };
                    ("200 OK", serde_json::to_vec(&serde_json::json!({ "version": version, "url": format!("{base}payload"), "signature": signature })).unwrap())
                };
                let length = body.len()
                    + if mode == "interrupted" && request.starts_with("GET /payload ") {
                        100
                    } else {
                        0
                    };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n"
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            }
        });
        (endpoint, task)
    }
    fn native_test_updater(
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
        endpoint: &str,
    ) -> tauri_plugin_updater::Updater {
        app.updater_builder()
            .endpoints(vec![endpoint.parse().unwrap()])
            .unwrap()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
    }

    fn test_app() -> tauri::App<tauri::test::MockRuntime> {
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".into(),
            serde_json::json!({
                "pubkey": fixture::PUBLIC_KEY, "requireSignedVersion": true,
                "dangerousInsecureTransportProtocol": true
            }),
        );
        tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap()
    }

    #[tokio::test]
    async fn native_plugin_downloads_verify_signatures_versions_and_interrupted_responses() {
        let app = test_app();
        for mode in [
            "valid",
            "corrupt",
            "wrong_version",
            "wrong_signature",
            "interrupted",
            "current",
            "malformed",
        ] {
            let (endpoint, task) = server(mode).await;
            let result = native_test_updater(app.handle(), &endpoint).check().await;
            if mode == "current" {
                assert!(result.unwrap().is_none());
            } else if mode == "malformed" {
                assert!(result.is_err());
            } else {
                let update = result.unwrap().unwrap();
                let mut downloaded = 0;
                let result = update.download(|count, _| downloaded += count, || {}).await;
                if mode == "valid" {
                    assert_eq!(result.unwrap(), fixture::BYTES);
                    assert_eq!(downloaded, fixture::BYTES.len());
                } else {
                    assert!(
                        result.is_err(),
                        "{mode} must never produce installable bytes"
                    );
                }
            }
            task.abort();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let offline = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        assert!(
            native_test_updater(app.handle(), &offline)
                .check()
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn installation_requires_verified_bytes_and_cancels_when_settings_cannot_be_saved() {
        let app = test_app();
        let updates = Updates::new(app.handle());
        let guard = updates.pending.lock().await;
        assert_eq!(
            updates.install(app.handle()).await.unwrap_err().code,
            "update_busy"
        );
        drop(guard);
        assert_eq!(
            updates.install(app.handle()).await.unwrap_err().code,
            "update_not_verified"
        );
        app.manage(Lifecycle::default());
        app.manage(DesktopSearch::new());
        let root = tempfile::tempdir().unwrap();
        let mut config = rustor_torrent::EngineConfig::new(
            root.path().join("downloads"),
            root.path().join("state"),
        );
        config.discovery = false;
        let desktop = Arc::new(DesktopState::new(config));
        desktop.initialize().await;
        app.manage(desktop.clone());
        let (endpoint, task) = server("valid").await;
        let update = native_test_updater(app.handle(), &endpoint)
            .check()
            .await
            .unwrap()
            .unwrap();
        let bytes = update.download(|_, _| {}, || {}).await.unwrap();
        *updates.pending.lock().await = Some(Pending {
            update,
            bytes: Some(bytes),
        });
        tokio::fs::create_dir(root.path().join("state/settings.json"))
            .await
            .unwrap();
        assert_eq!(
            updates.install(app.handle()).await.unwrap_err().code,
            "update_save_failed"
        );
        let snapshot = updates.snapshot();
        assert_eq!(snapshot.phase, Phase::Failed);
        assert_eq!(snapshot.retry, Some("install"));
        assert!(!snapshot.transfers_stopped);
        assert!(!desktop.stopping.load(std::sync::atomic::Ordering::Acquire));
        assert!(matches!(
            desktop.snapshot().await.phase,
            crate::service::EnginePhase::Ready
        ));
        assert!(app.state::<Lifecycle>().begin_update().is_ok());
        app.state::<Lifecycle>().update_failed();
        desktop.shutdown().await;
        task.abort();
    }
    #[test]
    fn manifest_downloads_stay_on_the_versioned_project_release_route() {
        assert!(
            validate_download_url(
                "https://github.com/adityapandeydev/RusTor/releases/download/v0.2.0/RusTor.exe",
                "0.2.0"
            )
            .is_ok()
        );
        for url in [
            "http://github.com/adityapandeydev/RusTor/releases/download/v0.2.0/a.exe",
            "https://evil.test/a.exe",
            "https://github.com/adityapandeydev/RusTor/releases/download/v0.2.0/a.exe?redirect=evil",
        ] {
            assert!(validate_download_url(url, "0.2.0").is_err());
        }
    }
}
