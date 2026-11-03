use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use rustor_torrent::{AddOptions, Engine, EngineConfig, TorrentId, TorrentInput, TorrentSnapshot};
use serde::{Deserialize, Serialize};
use tokio::{
    io::AsyncReadExt,
    sync::{Mutex, OnceCell},
};

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
    pub error: Option<CommandError>,
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
pub struct AddRequest {
    pub source: AddSource,
    pub destination_id: Option<String>,
}

#[derive(Default)]
struct Selections {
    next: u64,
    files: VecDeque<(String, PathBuf)>,
    folders: VecDeque<(String, PathBuf)>,
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
    engine: OnceCell<Result<Engine, CommandError>>,
    config: Mutex<Option<EngineConfig>>,
    selections: Mutex<Selections>,
    default_destination: PathBuf,
    revision: AtomicU64,
}

impl DesktopState {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            default_destination: config.download_dir.clone(),
            stopping: AtomicBool::new(false),
            engine: OnceCell::new(),
            config: Mutex::new(Some(config)),
            selections: Mutex::new(Selections::default()),
            revision: AtomicU64::new(0),
        }
    }

    pub async fn initialize(&self) {
        self.engine
            .get_or_init(|| async {
                let config = self
                    .config
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| CommandError::new("stopped", "RusTor is shutting down."))?;
                Engine::open(config).await.map_err(Into::into)
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
        AppSnapshot {
            revision: self.revision.fetch_add(1, Ordering::Relaxed) + 1,
            phase,
            torrents,
            error,
            default_destination: self.default_destination.to_string_lossy().into_owned(),
        }
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

    pub async fn add(
        &self,
        request: AddRequest,
    ) -> Result<rustor_torrent::AddOutcome, CommandError> {
        self.add_with_options(request, AddOptions::default()).await
    }

    async fn add_with_options(
        &self,
        request: AddRequest,
        mut options: AddOptions,
    ) -> Result<rustor_torrent::AddOutcome, CommandError> {
        let engine = self.engine()?;
        let destination = request.destination_id.as_ref().map(|id| id.to_owned());
        let destination = match destination {
            Some(id) => Some(self.selections.lock().await.find(&id, true)?),
            None => None,
        };
        let input = match request.source {
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
        options.destination = destination;
        engine.add(input, options).await.map_err(Into::into)
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

    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        self.initialize().await;
        if let Some(Ok(engine)) = self.engine.get() {
            let _ = engine.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn file_command_cannot_name_an_arbitrary_local_path() {
        assert!(
            serde_json::from_str::<AddRequest>(
                r#"{"source":{"kind":"file","path":"C:/secret"},"destination_id":null}"#
            )
            .is_err()
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
        let added = state
            .add_with_options(
                AddRequest {
                    source: AddSource::File { id: file.id },
                    destination_id: Some(folder.id),
                },
                AddOptions {
                    initial_peers: vec![seeder.listen_addr().unwrap()],
                    ..Default::default()
                },
            )
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
        state.pause(added.id.as_str()).await.unwrap();
        assert_eq!(
            state.snapshot().await.torrents[0].state,
            rustor_torrent::TorrentState::Paused
        );
        state.resume(added.id.as_str()).await.unwrap();
        state.shutdown().await;
        seeder.stop().await;
    }
}
