use std::{
    collections::VecDeque,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use rustor_torrent::{
    AddOptions, Engine, EngineConfig, PreparedTorrent, TorrentFile, TorrentId, TorrentInput,
    TorrentSnapshot,
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::AsyncReadExt,
    sync::{Mutex, OnceCell},
};

use crate::settings::DownloadSettings;

const MAX_METAINFO: u64 = 16 * 1024 * 1024;
const MAX_SELECTIONS: usize = 32;

#[derive(Clone, Debug, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<rustor_torrent::Error> for CommandError {
    fn from(error: rustor_torrent::Error) -> Self {
        use rustor_torrent::Error;
        match error {
            Error::InvalidInput(message) => Self::new("invalid_input", message),
            Error::MetadataTimeout => Self::new(
                "metadata_timeout",
                "Could not find torrent metadata in time. Check the link and your connection, then try again.",
            ),
            Error::NotFound => Self::new("not_found", "This torrent is no longer in the session."),
            Error::Stopped => Self::new("stopped", "RusTor is shutting down."),
            Error::SessionNotSaved => Self::new(
                "session_not_saved",
                "The active torrent may have changed, but RusTor could not confirm that its session was saved. Check free space and permissions in RusTor's application-data folder. A removed torrent may return, or its pause state may change, after restarting.",
            ),
            Error::RestoreFailed => Self::new(
                "session_restore_failed",
                "The saved torrent session could not be restored completely. Close RusTor and check its application-data engine folder, free space, permissions and saved .torrent files. Keep the saved session and payload files for recovery.",
            ),
            Error::StateInUse => Self::new(
                "state_in_use",
                "Another RusTor process is using this session. Close it and restart RusTor.",
            ),
            _ => Self::new(
                "engine_error",
                "The torrent engine could not complete this operation. Check the download folder and try again.",
            ),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnginePhase {
    Starting,
    Ready,
    Failed,
}

#[derive(Clone, Serialize)]
pub struct AppSnapshot {
    pub revision: u64,
    pub phase: EnginePhase,
    pub torrents: Vec<TorrentSnapshot>,
    pub default_destination: String,
    pub settings: DownloadSettings,
    pub error: Option<CommandError>,
    pub tray_available: bool,
    pub background_error: Option<CommandError>,
    pub startup: crate::startup::Status,
}

#[derive(Serialize)]
pub struct Selection {
    pub id: String,
    pub label: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AddSource {
    Magnet { value: String },
    File { id: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub prepared_id: String,
    pub files: Vec<usize>,
    pub destination_id: Option<String>,
}

#[derive(Serialize)]
pub struct PreparedSelection {
    pub id: String,
    pub name: Option<String>,
    pub files: Vec<TorrentFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveSettingsRequest {
    pub destination_id: Option<String>,
    pub download_limit_kib: u32,
    pub upload_limit_kib: u32,
    #[serde(default)]
    pub continue_in_background: bool,
    #[serde(default)]
    pub start_at_login: bool,
    #[serde(default)]
    pub start_minimized: bool,
}

#[derive(Default)]
struct Selections {
    next: u64,
    files: VecDeque<(String, PathBuf)>,
    folders: VecDeque<(String, PathBuf)>,
    prepared: VecDeque<(String, Arc<PreparedTorrent>)>,
}

impl Selections {
    fn insert(&mut self, path: PathBuf, folder: bool) -> Selection {
        self.next += 1;
        let id = self.next.to_string();
        let label = if folder {
            path.to_string_lossy().into_owned()
        } else {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        let entries = if folder {
            &mut self.folders
        } else {
            &mut self.files
        };
        entries.push_back((id.clone(), path));
        while entries.len() > MAX_SELECTIONS {
            entries.pop_front();
        }
        Selection { id, label }
    }

    fn insert_prepared(&mut self, torrent: PreparedTorrent) -> PreparedSelection {
        self.next += 1;
        let id = self.next.to_string();
        let response = PreparedSelection {
            id: id.clone(),
            name: torrent.name.clone(),
            files: torrent.files.clone(),
        };
        self.prepared.push_back((id, Arc::new(torrent)));
        // At most four validated metainfo blobs (each <=16 MiB) are retained.
        while self.prepared.len() > 4 {
            self.prepared.pop_front();
        }
        response
    }

    fn find_prepared(&self, id: &str) -> Result<Arc<PreparedTorrent>, CommandError> {
        self.prepared
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, torrent)| torrent.clone())
            .ok_or_else(|| {
                CommandError::new(
                    "selection_expired",
                    "This file list has expired. Load the files again.",
                )
            })
    }

    fn find(&self, id: &str, folder: bool) -> Result<PathBuf, CommandError> {
        let entries = if folder { &self.folders } else { &self.files };
        entries
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, path)| path.clone())
            .ok_or_else(|| {
                CommandError::new(
                    "selection_expired",
                    "This selection is no longer available. Choose it again.",
                )
            })
    }
}

pub struct DesktopState {
    pub stopping: AtomicBool,
    pub tray_available: AtomicBool,
    pub continue_in_background: AtomicBool,
    background_error: Mutex<Option<CommandError>>,
    bulk_action: Mutex<()>,
    engine: OnceCell<Result<Engine, CommandError>>,
    config: Mutex<Option<EngineConfig>>,
    selections: Mutex<Selections>,
    settings: Mutex<DownloadSettings>,
    settings_path: PathBuf,
    revision: AtomicU64,
    startup_registration: Arc<dyn crate::startup::Registration>,
}

impl DesktopState {
    #[cfg(test)]
    pub fn new(config: EngineConfig) -> Self {
        Self::with_startup(config, Arc::new(crate::startup::UnavailableRegistration))
    }

    pub fn with_startup(
        config: EngineConfig,
        startup_registration: Arc<dyn crate::startup::Registration>,
    ) -> Self {
        Self {
            settings: Mutex::new(DownloadSettings::defaults(config.download_dir.clone())),
            settings_path: config.state_dir.join("settings.json"),
            stopping: AtomicBool::new(false),
            tray_available: AtomicBool::new(false),
            continue_in_background: AtomicBool::new(false),
            background_error: Mutex::new(None),
            bulk_action: Mutex::new(()),
            engine: OnceCell::new(),
            config: Mutex::new(Some(config)),
            selections: Mutex::new(Selections::default()),
            revision: AtomicU64::new(0),
            startup_registration,
        }
    }

    pub async fn initialize(&self) {
        self.engine
            .get_or_init(|| async {
                // shutdown also awaits this cell to drain an in-flight startup.
                // If startup has not begun, it must not open a new session.
                if self.stopping.load(Ordering::Acquire) {
                    return Err(CommandError::new("stopped", "RusTor is shutting down."));
                }
                let mut config = self
                    .config
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| CommandError::new("stopped", "RusTor is shutting down."))?;
                let saved =
                    DownloadSettings::load(&self.settings_path, self.settings.lock().await.clone())
                        .await?;
                let limits = saved.limits()?;
                config.download_limit_bps = limits.download_bps;
                config.upload_limit_bps = limits.upload_bps;
                let engine = Engine::open(config).await.map_err(CommandError::from)?;
                self.continue_in_background
                    .store(saved.continue_in_background, Ordering::Release);
                *self.settings.lock().await = saved;
                Ok(engine)
            })
            .await;
    }

    fn engine(&self) -> Result<&Engine, CommandError> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(CommandError::new("stopped", "RusTor is shutting down."));
        }
        self.engine
            .get()
            .ok_or_else(|| CommandError::new("starting", "The torrent engine is still starting."))?
            .as_ref()
            .map_err(Clone::clone)
    }

    pub async fn snapshot(&self) -> AppSnapshot {
        let (phase, torrents, error) = match self.engine.get() {
            None => (EnginePhase::Starting, vec![], None),
            Some(Err(error)) => (EnginePhase::Failed, vec![], Some(error.clone())),
            Some(Ok(engine)) => match engine.list().await {
                Ok(torrents) => (EnginePhase::Ready, torrents, None),
                Err(error) => (EnginePhase::Failed, vec![], Some(error.into())),
            },
        };
        let settings = self.settings.lock().await.clone();
        AppSnapshot {
            revision: self.revision.fetch_add(1, Ordering::Relaxed) + 1,
            phase,
            torrents,
            error,
            default_destination: settings.default_destination.to_string_lossy().into_owned(),
            settings,
            tray_available: self.tray_available.load(Ordering::Acquire),
            background_error: self.background_error.lock().await.clone(),
            startup: crate::startup::status(self.startup_registration.as_ref()),
        }
    }

    pub async fn save_settings(
        &self,
        request: SaveSettingsRequest,
    ) -> Result<AppSnapshot, CommandError> {
        let mut current = self.settings.lock().await;
        let engine = self.engine()?;
        if request.continue_in_background && !self.tray_available.load(Ordering::Acquire) {
            return Err(CommandError::new(
                "tray_unavailable",
                "The system tray is unavailable. Keep RusTor open to continue transfers.",
            ));
        }
        if request.start_at_login
            && request.start_minimized
            && !self.tray_available.load(Ordering::Acquire)
        {
            return Err(CommandError::new(
                "tray_unavailable",
                "Start minimized requires an available system tray. Start with the window visible instead.",
            ));
        }
        let destination = match request.destination_id {
            Some(id) => {
                let path = self.selections.lock().await.find(&id, true)?;
                if !tokio::fs::metadata(&path)
                    .await
                    .map(|m| m.is_dir())
                    .unwrap_or(false)
                {
                    return Err(CommandError::new(
                        "invalid_selection",
                        "The selected folder is no longer available. Choose it again.",
                    ));
                }
                path
            }
            None => current.default_destination.clone(),
        };
        let settings = DownloadSettings {
            default_destination: destination,
            download_limit_kib: request.download_limit_kib,
            upload_limit_kib: request.upload_limit_kib,
            continue_in_background: request.continue_in_background,
            start_at_login: request.start_at_login,
            start_minimized: request.start_minimized,
        };
        let limits = settings.limits()?;
        // A failed write leaves both committed preferences and active limits intact.
        let previous = crate::startup::apply(
            self.startup_registration.as_ref(),
            settings.start_at_login,
            current.start_at_login,
        )?;
        if let Err(error) = settings.save(self.settings_path.clone()).await {
            return Err(crate::startup::rollback(
                self.startup_registration.as_ref(),
                previous,
                error,
            ));
        }
        engine.set_speed_limits(limits).await?;
        *current = settings;
        self.continue_in_background
            .store(current.continue_in_background, Ordering::Release);
        drop(current);
        Ok(self.snapshot().await)
    }

    pub async fn select(&self, path: PathBuf, folder: bool) -> Result<Selection, CommandError> {
        let path = tokio::fs::canonicalize(path).await.map_err(|_| {
            CommandError::new(
                "unavailable",
                "The selected file or folder is no longer available.",
            )
        })?;
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|_| CommandError::new("unavailable", "The selection could not be read."))?;
        if folder != metadata.is_dir() || (!folder && !metadata.is_file()) {
            return Err(CommandError::new(
                "invalid_selection",
                "Choose a torrent file or a download folder.",
            ));
        }
        Ok(self.selections.lock().await.insert(path, folder))
    }

    pub async fn prepare(&self, source: AddSource) -> Result<PreparedSelection, CommandError> {
        self.prepare_with_peers(source, Vec::new()).await
    }

    async fn prepare_with_peers(
        &self,
        source: AddSource,
        peers: Vec<SocketAddr>,
    ) -> Result<PreparedSelection, CommandError> {
        let engine = self.engine()?;
        let input = self.read_input(source).await?;
        let torrent = engine.prepare(input, peers).await?;
        Ok(self.selections.lock().await.insert_prepared(torrent))
    }

    pub async fn discard_prepared(&self, id: &str) {
        self.selections
            .lock()
            .await
            .prepared
            .retain(|(key, _)| key != id);
    }

    pub async fn start(
        &self,
        request: StartRequest,
    ) -> Result<rustor_torrent::AddOutcome, CommandError> {
        let engine = self.engine()?;
        let (prepared, destination) = {
            let selections = self.selections.lock().await;
            let prepared = selections.find_prepared(&request.prepared_id)?;
            let destination = request
                .destination_id
                .as_ref()
                .map(|id| selections.find(id, true))
                .transpose()?;
            (prepared, destination)
        };
        let destination = match destination {
            Some(folder) => folder,
            None => prepared.destination_under(&self.settings.lock().await.default_destination),
        };
        let result = engine
            .add_prepared(
                &prepared,
                request.files,
                AddOptions {
                    destination: Some(destination),
                    ..Default::default()
                },
            )
            .await?;
        self.discard_prepared(&request.prepared_id).await;
        Ok(result)
    }

    async fn read_input(&self, source: AddSource) -> Result<TorrentInput, CommandError> {
        let input = match source {
            AddSource::Magnet { value } => TorrentInput::parse(&value)?,
            AddSource::File { id } => {
                let path = self.selections.lock().await.find(&id, false)?;
                let file = tokio::fs::File::open(path).await.map_err(|_| {
                    CommandError::new(
                        "file_unavailable",
                        "The torrent file could not be read. Choose it again.",
                    )
                })?;
                let mut bytes = Vec::new();
                file.take(MAX_METAINFO + 1)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|_| {
                        CommandError::new("file_unavailable", "The torrent file could not be read.")
                    })?;
                tokio::task::spawn_blocking(move || TorrentInput::from_bytes(bytes))
                    .await
                    .map_err(|_| {
                        CommandError::new(
                            "invalid_input",
                            "The torrent metadata could not be processed.",
                        )
                    })??
            }
        };
        Ok(input)
    }

    pub async fn pause(&self, id: &str) -> Result<(), CommandError> {
        self.engine()?
            .pause(&TorrentId::parse(id)?)
            .await
            .map_err(Into::into)
    }

    pub async fn resume(&self, id: &str) -> Result<(), CommandError> {
        self.engine()?
            .resume(&TorrentId::parse(id)?)
            .await
            .map_err(Into::into)
    }

    pub async fn retry(&self, id: &str) -> Result<(), CommandError> {
        self.engine()?
            .retry(&TorrentId::parse(id)?)
            .await
            .map_err(Into::into)
    }

    pub async fn remove(&self, id: &str) -> Result<(), CommandError> {
        self.engine()?
            .remove(&TorrentId::parse(id)?)
            .await
            .map_err(Into::into)
    }

    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        self.initialize().await;
        let _settings = self.settings.lock().await;
        if let Some(Ok(engine)) = self.engine.get() {
            let _ = engine.shutdown().await;
        }
    }

    pub async fn report_background_error(&self, error: CommandError) {
        *self.background_error.lock().await = Some(error);
    }

    pub async fn control_all(&self, pause: bool) -> Result<(), CommandError> {
        let _guard = self.bulk_action.try_lock().map_err(|_| {
            CommandError::new(
                "tray_busy",
                "Another tray action is still running. Wait a moment and try again.",
            )
        })?;
        let engine = self.engine()?;
        let torrents = engine.list().await.map_err(CommandError::from)?;
        let mut changed = 0;
        let mut failures = Vec::new();
        for torrent in torrents {
            if !bulk_target(torrent.state, pause) {
                continue;
            }
            let result = if pause {
                self.pause(torrent.id.as_str()).await
            } else {
                self.resume(torrent.id.as_str()).await
            };
            match result {
                Ok(()) => changed += 1,
                Err(error) => failures.push(error),
            }
        }
        if let Some(first) = failures.first() {
            return Err(CommandError::new(
                "tray_action_failed",
                format!(
                    "{} all: {changed} transfers updated; {} failed. {}",
                    if pause { "Pause" } else { "Resume" },
                    failures.len(),
                    first.message
                ),
            ));
        }
        *self.background_error.lock().await = None;
        Ok(())
    }
}

fn bulk_target(state: rustor_torrent::TorrentState, pause: bool) -> bool {
    use rustor_torrent::TorrentState::*;
    if pause {
        matches!(state, Checking | Downloading | Seeding)
    } else {
        state == Paused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemoryRegistration(AtomicBool);
    impl crate::startup::Registration for MemoryRegistration {
        fn enabled(&self) -> Result<bool, CommandError> {
            Ok(self.0.load(Ordering::Acquire))
        }
        fn set_enabled(&self, enabled: bool) -> Result<(), CommandError> {
            self.0.store(enabled, Ordering::Release);
            Ok(())
        }
    }

    #[tokio::test]
    async fn startup_registration_and_settings_rollback_keep_the_actual_os_state_visible() {
        let root = tempfile::tempdir().unwrap();
        let mut config =
            EngineConfig::new(root.path().join("downloads"), root.path().join("state"));
        config.discovery = false;
        let registration = Arc::new(MemoryRegistration(AtomicBool::new(false)));
        let mut state = DesktopState::with_startup(config, registration.clone());
        state.tray_available.store(true, Ordering::Release);
        state.initialize().await;
        let request = || SaveSettingsRequest {
            destination_id: None,
            download_limit_kib: 0,
            upload_limit_kib: 0,
            continue_in_background: false,
            start_at_login: true,
            start_minimized: true,
        };
        let saved = state.save_settings(request()).await.unwrap();
        assert_eq!(saved.startup.enabled, Some(true));
        assert!(saved.settings.start_minimized);
        let committed_path = state.settings_path.clone();
        let committed = tokio::fs::read(&committed_path).await.unwrap();
        registration.0.store(false, Ordering::Release);
        assert_eq!(state.snapshot().await.startup.enabled, Some(false));
        state.settings_path = root.path().join("missing/settings.json");
        assert_eq!(
            state.save_settings(request()).await.err().unwrap().code,
            "settings_not_saved"
        );
        assert!(!registration.0.load(Ordering::Acquire));
        assert_eq!(state.snapshot().await.settings, saved.settings);
        assert_eq!(tokio::fs::read(committed_path).await.unwrap(), committed);
        state.shutdown().await;
        assert!(!registration.0.load(Ordering::Acquire));
    }

    #[test]
    fn recovery_errors_explain_active_state_and_preserve_recovery_guidance() {
        let error = CommandError::from(rustor_torrent::Error::SessionNotSaved);
        assert_eq!(error.code, "session_not_saved");
        assert!(error.message.contains("may have changed"));
        assert!(error.message.contains("removed torrent may return"));
        let error = CommandError::from(rustor_torrent::Error::RestoreFailed);
        assert_eq!(error.code, "session_restore_failed");
        assert!(
            error
                .message
                .contains("Keep the saved session and payload files")
        );
    }

    #[test]
    fn tray_bulk_controls_do_not_retry_failures_or_resume_already_running_transfers() {
        use rustor_torrent::TorrentState::*;
        for state in [Checking, Downloading, Seeding] {
            assert!(bulk_target(state, true));
            assert!(!bulk_target(state, false));
        }
        assert!(!bulk_target(Paused, true));
        assert!(bulk_target(Paused, false));
        assert!(!bulk_target(Failed, true));
        assert!(!bulk_target(Failed, false));
    }

    #[tokio::test]
    async fn unavailable_tray_rejects_background_mode_and_bulk_actions_are_serialized() {
        let root = tempfile::tempdir().unwrap();
        let mut config =
            EngineConfig::new(root.path().join("downloads"), root.path().join("state"));
        config.discovery = false;
        let state = DesktopState::new(config);
        assert_eq!(state.control_all(true).await.unwrap_err().code, "starting");
        state.initialize().await;
        assert_eq!(
            state
                .save_settings(SaveSettingsRequest {
                    destination_id: None,
                    download_limit_kib: 0,
                    upload_limit_kib: 0,
                    continue_in_background: true,
                    start_at_login: false,
                    start_minimized: false,
                })
                .await
                .err()
                .unwrap()
                .code,
            "tray_unavailable"
        );
        assert!(!state.continue_in_background.load(Ordering::Acquire));
        assert!(!state.settings_path.exists());
        let guard = state.bulk_action.lock().await;
        assert_eq!(state.control_all(true).await.unwrap_err().code, "tray_busy");
        drop(guard);
        state
            .report_background_error(CommandError::new("test", "visible failure"))
            .await;
        assert_eq!(
            state.snapshot().await.background_error.unwrap().message,
            "visible failure"
        );
        state.control_all(true).await.unwrap();
        assert!(state.snapshot().await.background_error.is_none());
        state.shutdown().await;
        assert_eq!(state.control_all(false).await.unwrap_err().code, "stopped");
    }

    #[test]
    fn selections_are_scoped_bounded_and_do_not_accept_paths_as_ids() {
        let mut selections = Selections::default();
        let first = selections.insert(PathBuf::from("/safe/a.torrent"), false);
        assert!(selections.find("/safe/a.torrent", false).is_err());
        assert!(selections.find(&first.id, true).is_err());
        for i in 0..MAX_SELECTIONS {
            selections.insert(PathBuf::from(format!("/safe/{i}.torrent")), false);
        }
        assert!(selections.find(&first.id, false).is_err());
        assert_eq!(selections.files.len(), MAX_SELECTIONS);
    }

    #[tokio::test]
    async fn starting_and_failed_engine_states_are_visible() {
        let state = DesktopState::new(EngineConfig::new("relative".into(), "state".into()));
        assert!(matches!(
            state.snapshot().await.phase,
            EnginePhase::Starting
        ));
        state.initialize().await;
        let snapshot = state.snapshot().await;
        assert!(matches!(snapshot.phase, EnginePhase::Failed));
        assert!(snapshot.error.is_some());
    }

    #[tokio::test]
    async fn closing_before_startup_does_not_open_a_session_or_create_directories() {
        let root = tempfile::tempdir().unwrap();
        let mut config =
            EngineConfig::new(root.path().join("downloads"), root.path().join("state"));
        config.discovery = false;
        let state = DesktopState::new(config);
        state.shutdown().await;
        state.initialize().await;
        assert!(!root.path().join("state").exists());
        assert!(!root.path().join("downloads").exists());
        assert_eq!(state.engine().err().unwrap().code, "stopped");
    }

    #[tokio::test]
    async fn settings_apply_restore_and_failed_writes_do_not_change_live_limits() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("chosen");
        tokio::fs::create_dir_all(&destination).await.unwrap();
        let make_config = || {
            let mut config =
                EngineConfig::new(root.path().join("downloads"), root.path().join("state"));
            config.discovery = false;
            config
        };
        let mut state = DesktopState::new(make_config());
        state.tray_available.store(true, Ordering::Release);
        state.initialize().await;
        let folder = state.select(destination.clone(), true).await.unwrap();
        let saved = state
            .save_settings(SaveSettingsRequest {
                destination_id: Some(folder.id),
                download_limit_kib: 64,
                upload_limit_kib: 32,
                continue_in_background: true,
                start_at_login: false,
                start_minimized: false,
            })
            .await
            .unwrap();
        assert_eq!(
            saved.settings.default_destination,
            tokio::fs::canonicalize(&destination).await.unwrap()
        );
        assert_eq!(
            state.engine().unwrap().speed_limits().await.unwrap(),
            saved.settings.limits().unwrap()
        );
        // Invalid input never changes the saved preferences or active limiter.
        let committed = tokio::fs::read(&state.settings_path).await.unwrap();
        assert!(
            state
                .save_settings(SaveSettingsRequest {
                    destination_id: None,
                    download_limit_kib: 1,
                    upload_limit_kib: 0,
                    continue_in_background: false,
                    start_at_login: false,
                    start_minimized: false,
                })
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read(&state.settings_path).await.unwrap(),
            committed
        );
        assert_eq!(
            state.engine().unwrap().speed_limits().await.unwrap(),
            saved.settings.limits().unwrap()
        );
        let committed_path = state.settings_path.clone();
        state.settings_path = root.path().join("missing/settings.json");
        assert!(
            state
                .save_settings(SaveSettingsRequest {
                    destination_id: None,
                    download_limit_kib: 256,
                    upload_limit_kib: 128,
                    continue_in_background: false,
                    start_at_login: false,
                    start_minimized: false,
                })
                .await
                .is_err()
        );
        assert_eq!(state.snapshot().await.settings, saved.settings);
        assert!(state.continue_in_background.load(Ordering::Acquire));
        assert_eq!(
            state.engine().unwrap().speed_limits().await.unwrap(),
            saved.settings.limits().unwrap()
        );
        assert_eq!(tokio::fs::read(&committed_path).await.unwrap(), committed);
        state.settings_path = committed_path;
        state.shutdown().await;
        let restored = DesktopState::new(make_config());
        restored.initialize().await;
        assert!(restored.continue_in_background.load(Ordering::Acquire));
        assert_eq!(restored.snapshot().await.settings, saved.settings);
        assert_eq!(
            restored.engine().unwrap().speed_limits().await.unwrap(),
            saved.settings.limits().unwrap()
        );
        restored.shutdown().await;
    }

    #[test]
    fn file_command_cannot_name_an_arbitrary_local_path() {
        assert!(
            serde_json::from_str::<AddSource>(r#"{"kind":"file","path":"C:/secret"}"#).is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn selected_file_reaches_selected_folder_through_the_desktop_service() {
        use librqbit::{
            AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session,
            SessionOptions, create_torrent, spawn_utils::BlockingSpawner,
        };
        use std::{net::Ipv4Addr, time::Duration};
        let root = tempfile::tempdir().unwrap();
        let seed = root.path().join("seed");
        let destination = root.path().join("selected");
        tokio::fs::create_dir_all(&seed).await.unwrap();
        tokio::fs::create_dir_all(&destination).await.unwrap();
        let data = vec![42u8; 32 * 1024];
        let payload = seed.join("authorized.bin");
        tokio::fs::write(&payload, &data).await.unwrap();
        let metadata = create_torrent(
            &payload,
            CreateTorrentOptions::default(),
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap();
        let path = root.path().join("authorized.torrent");
        tokio::fs::write(&path, &metadata).await.unwrap();
        let seeder = Session::new_with_opts(
            seed,
            SessionOptions {
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                listen: Some(ListenerOptions {
                    listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let handle = seeder
            .add_torrent(
                AddTorrent::from_bytes(metadata),
                Some(AddTorrentOptions {
                    overwrite: true,
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_completed().await.unwrap();
        let mut config = EngineConfig::new(root.path().join("default"), root.path().join("state"));
        config.discovery = false;
        let state = DesktopState::new(config);
        state.initialize().await;
        let file = state.select(path, false).await.unwrap();
        let folder = state.select(destination.clone(), true).await.unwrap();
        assert!(
            state
                .start(StartRequest {
                    prepared_id: "unknown".into(),
                    files: vec![0],
                    destination_id: None
                })
                .await
                .is_err()
        );
        let prepared = state
            .prepare_with_peers(
                AddSource::File {
                    id: file.id.clone(),
                },
                vec![seeder.listen_addr().unwrap()],
            )
            .await
            .unwrap();
        // Bound retained previews and reject stale handles without starting anything.
        let first_id = prepared.id.clone();
        let mut prepared = prepared;
        for _ in 0..4 {
            prepared = state
                .prepare_with_peers(
                    AddSource::File {
                        id: file.id.clone(),
                    },
                    vec![seeder.listen_addr().unwrap()],
                )
                .await
                .unwrap();
        }
        assert!(
            state
                .start(StartRequest {
                    prepared_id: first_id,
                    files: vec![0],
                    destination_id: None
                })
                .await
                .is_err()
        );
        assert_eq!(state.selections.lock().await.prepared.len(), 4);
        assert!(state.snapshot().await.torrents.is_empty());
        assert!(!destination.join("authorized.bin").exists());
        assert!(
            state
                .start(StartRequest {
                    prepared_id: prepared.id.clone(),
                    files: vec![],
                    destination_id: Some(folder.id.clone())
                })
                .await
                .is_err()
        );
        let added = state
            .start(StartRequest {
                prepared_id: prepared.id,
                files: vec![0],
                destination_id: Some(folder.id),
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let snapshot = state.snapshot().await;
                if snapshot
                    .torrents
                    .iter()
                    .any(|t| t.id == added.id && t.state == rustor_torrent::TorrentState::Seeding)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::read(destination.join("authorized.bin"))
                .await
                .unwrap(),
            data
        );
        state.control_all(true).await.unwrap();
        assert_eq!(
            state.snapshot().await.torrents[0].state,
            rustor_torrent::TorrentState::Paused
        );
        state.control_all(false).await.unwrap();
        let future_folder = root.path().join("future-downloads");
        tokio::fs::create_dir_all(&future_folder).await.unwrap();
        let future_selection = state.select(future_folder.clone(), true).await.unwrap();
        state
            .save_settings(SaveSettingsRequest {
                destination_id: Some(future_selection.id),
                download_limit_kib: 0,
                upload_limit_kib: 0,
                continue_in_background: false,
                start_at_login: false,
                start_minimized: false,
            })
            .await
            .unwrap();
        assert_eq!(
            state.snapshot().await.torrents[0].destination,
            tokio::fs::canonicalize(&destination).await.unwrap()
        );
        assert_eq!(
            state.retry(added.id.as_str()).await.unwrap_err().code,
            "invalid_input"
        );
        assert_eq!(state.snapshot().await.torrents.len(), 1);
        state.remove(added.id.as_str()).await.unwrap();
        assert!(state.snapshot().await.torrents.is_empty());
        assert_eq!(
            tokio::fs::read(destination.join("authorized.bin"))
                .await
                .unwrap(),
            data
        );
        let prepared = state
            .prepare_with_peers(
                AddSource::File { id: file.id },
                vec![seeder.listen_addr().unwrap()],
            )
            .await
            .unwrap();
        state
            .start(StartRequest {
                prepared_id: prepared.id,
                files: vec![0],
                destination_id: None,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while state.snapshot().await.torrents[0].state != rustor_torrent::TorrentState::Seeding
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::read(future_folder.join("authorized.bin"))
                .await
                .unwrap(),
            data
        );
        assert_eq!(
            tokio::fs::read(destination.join("authorized.bin"))
                .await
                .unwrap(),
            data
        );
        state.shutdown().await;
        seeder.stop().await;
    }
}
