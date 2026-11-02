use std::{
    net::SocketAddr,
    num::{NonZeroU32, NonZeroUsize},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use librqbit::{
    AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListenerOptions, ManagedTorrent,
    Session, SessionOptions, SessionPersistenceConfig, TorrentStatsState, api::TorrentIdOrHash,
    limits::LimitsConfig,
};
use serde::Serialize;
use tokio::{
    sync::{Mutex, RwLock},
    time::timeout,
};

use crate::{Error, TorrentId, TorrentInput};

pub struct EngineConfig {
    pub download_dir: PathBuf,
    pub state_dir: PathBuf,
    /// Disable public discovery for controlled tests or explicitly supplied peers.
    pub discovery: bool,
    pub max_initializations: NonZeroUsize,
    pub peer_limit: NonZeroUsize,
    pub download_limit_bps: Option<NonZeroU32>,
    pub upload_limit_bps: Option<NonZeroU32>,
    pub metadata_timeout: Duration,
}

impl EngineConfig {
    pub fn new(download_dir: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            download_dir,
            state_dir,
            discovery: true,
            max_initializations: NonZeroUsize::new(2).unwrap(),
            peer_limit: NonZeroUsize::new(128).unwrap(),
            download_limit_bps: None,
            upload_limit_bps: None,
            metadata_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Default)]
pub struct AddOptions {
    pub destination: Option<PathBuf>,
    pub paused: bool,
    /// Explicit peer hints; normally empty when using public discovery.
    pub initial_peers: Vec<SocketAddr>,
}

#[derive(Debug, Serialize)]
pub struct AddOutcome {
    pub id: TorrentId,
    pub already_managed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentState {
    Checking,
    Downloading,
    Paused,
    Seeding,
    Failed,
}

#[derive(Debug, Serialize)]
pub struct TorrentSnapshot {
    pub id: TorrentId,
    pub name: Option<String>,
    pub destination: PathBuf,
    pub state: TorrentState,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub uploaded_bytes: u64,
    pub download_bytes_per_second: u64,
    pub upload_bytes_per_second: u64,
    pub peers: u32,
    pub error: Option<String>,
}

/// One long-lived native session. A read/write gate drains accepted operations
/// before shutdown; control serialization does not block unrelated metadata adds.
pub struct Engine {
    session: RwLock<Option<ActiveSession>>,
    controls: Mutex<()>,
    metadata_timeout: Duration,
    discovery: bool,
    next_native_id: AtomicUsize,
}

struct ActiveSession {
    native: Arc<Session>,
    _directory_lock: std::fs::File,
}

impl Engine {
    pub async fn open(config: EngineConfig) -> Result<Self, Error> {
        if !config.download_dir.is_absolute() || !config.state_dir.is_absolute() {
            return Err(Error::InvalidConfig(
                "download and state paths must be absolute",
            ));
        }
        if config.metadata_timeout.is_zero() {
            return Err(Error::InvalidConfig("metadata timeout must be positive"));
        }
        for dir in [&config.download_dir, &config.state_dir] {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|e| backend("create engine directory", e))?;
        }
        let directory_lock = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(config.state_dir.join("session.lock"))
            .await
            .map_err(|e| backend("open session lock", e))?
            .into_std()
            .await;
        match directory_lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(Error::StateInUse),
            Err(error) => return Err(backend("lock session directory", error)),
        }
        let options = SessionOptions {
            listen: config.discovery.then(ListenerOptions::default),
            dht: config.discovery.then(|| DhtSessionConfig {
                persistence: None,
                ..Default::default()
            }),
            disable_trackers: !config.discovery,
            disable_local_service_discovery: !config.discovery,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(config.state_dir),
            }),
            fastresume: true,
            concurrent_init_limit: Some(config.max_initializations.get()),
            peer_limit: Some(config.peer_limit.get()),
            ratelimits: LimitsConfig {
                download_bps: config.download_limit_bps,
                upload_bps: config.upload_limit_bps,
            },
            client_name_and_version: Some(concat!("RusTor/", env!("CARGO_PKG_VERSION")).to_owned()),
            ..Default::default()
        };
        let session = Session::new_with_opts(config.download_dir, options)
            .await
            .map_err(|e| backend("open torrent session", e))?;
        let next_native_id = session
            .with_torrents(|torrents| {
                torrents
                    .map(|(id, _)| id)
                    .max()
                    .map_or(Some(0), |id| id.checked_add(1))
            })
            .ok_or_else(|| backend("open torrent session", "session ID space exhausted"))?;
        Ok(Self {
            session: RwLock::new(Some(ActiveSession {
                native: session,
                _directory_lock: directory_lock,
            })),
            controls: Mutex::new(()),
            metadata_timeout: config.metadata_timeout,
            discovery: config.discovery,
            next_native_id: AtomicUsize::new(next_native_id),
        })
    }

    pub async fn add(&self, input: TorrentInput, options: AddOptions) -> Result<AddOutcome, Error> {
        let destination = options
            .destination
            .map(|path| {
                if !path.is_absolute() {
                    return Err(Error::InvalidInput("destination must be absolute"));
                }
                path.into_os_string()
                    .into_string()
                    .map_err(|_| Error::InvalidInput("destination must be valid Unicode"))
            })
            .transpose()?;
        let guard = self.session.read().await;
        let session = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let expected_id = input.id().clone();
        // librqbit 9.1.0's JSON allocator reads max(persisted ID)+1 without
        // reserving it. Concurrent adds can collide before either is stored.
        // Reserve atomically here; restores retain their persisted numeric IDs.
        let native_id = self
            .next_native_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| backend("add torrent", "session ID space exhausted"))?;
        let response = timeout(
            self.metadata_timeout,
            session.add_torrent(
                input.into_native(),
                Some(AddTorrentOptions {
                    preferred_id: Some(native_id),
                    output_folder: destination,
                    paused: options.paused,
                    initial_peers: Some(options.initial_peers),
                    disable_trackers: !self.discovery,
                    // Existing files are protected. Engine-owned restore explicitly enables
                    // overwrite for its already-managed data, followed by verification.
                    overwrite: false,
                    ..Default::default()
                }),
            ),
        )
        .await
        .map_err(|_| Error::MetadataTimeout)?
        .map_err(|e| backend("add torrent", e))?;
        let (handle, already_managed) = match response {
            AddTorrentResponse::Added(_, handle) => (handle, false),
            AddTorrentResponse::AlreadyManaged(_, handle) => (handle, true),
            AddTorrentResponse::ListOnly(_) => {
                return Err(backend("add torrent", "unexpected metadata-only response"));
            }
        };
        let id = TorrentId::parse(&handle.info_hash().as_string())?;
        if id != expected_id {
            return Err(backend(
                "add torrent",
                "engine returned a different torrent identity",
            ));
        }
        Ok(AddOutcome {
            id,
            already_managed,
        })
    }

    pub async fn list(&self) -> Result<Vec<TorrentSnapshot>, Error> {
        let guard = self.session.read().await;
        let session = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let mut snapshots = session.with_torrents(|torrents| {
            torrents
                .map(|(_, handle)| snapshot(handle))
                .collect::<Result<Vec<_>, _>>()
        })?;
        snapshots.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(snapshots)
    }

    pub async fn snapshot(&self, id: &TorrentId) -> Result<TorrentSnapshot, Error> {
        let guard = self.session.read().await;
        let handle = handle(&guard.as_ref().ok_or(Error::Stopped)?.native, id)?;
        snapshot(&handle)
    }

    pub async fn pause(&self, id: &TorrentId) -> Result<(), Error> {
        let _control = self.controls.lock().await;
        let guard = self.session.read().await;
        let session = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let handle = handle(session, id)?;
        if handle.is_paused() {
            return Ok(());
        }
        session
            .pause(&handle)
            .await
            .map_err(|e| backend("pause torrent", e))
    }

    pub async fn resume(&self, id: &TorrentId) -> Result<(), Error> {
        let _control = self.controls.lock().await;
        let guard = self.session.read().await;
        let session = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let handle = handle(session, id)?;
        if !handle.is_paused() {
            return Ok(());
        }
        session
            .unpause(&handle)
            .await
            .map_err(|e| backend("resume torrent", e))
    }

    /// Idempotent. Applications must await this before exiting.
    pub async fn shutdown(&self) -> Result<(), Error> {
        let mut guard = self.session.write().await;
        if let Some(session) = guard.take() {
            session.native.stop().await;
        }
        Ok(())
    }
}

fn handle(session: &Session, id: &TorrentId) -> Result<Arc<ManagedTorrent>, Error> {
    let native_id = TorrentIdOrHash::parse(id.as_str())
        .map_err(|_| Error::InvalidInput("invalid torrent identity"))?;
    session.get(native_id).ok_or(Error::NotFound)
}

fn snapshot(handle: &ManagedTorrent) -> Result<TorrentSnapshot, Error> {
    let stats = handle.stats();
    let state = match stats.state {
        TorrentStatsState::Initializing { .. } => TorrentState::Checking,
        TorrentStatsState::Live if stats.finished => TorrentState::Seeding,
        TorrentStatsState::Live => TorrentState::Downloading,
        TorrentStatsState::Paused => TorrentState::Paused,
        TorrentStatsState::Error => TorrentState::Failed,
    };
    Ok(TorrentSnapshot {
        id: TorrentId::parse(&handle.info_hash().as_string())?,
        name: handle.name(),
        destination: handle.output_folder().to_owned(),
        state,
        downloaded_bytes: stats.progress_bytes,
        total_bytes: stats.total_bytes,
        uploaded_bytes: stats.uploaded_bytes,
        download_bytes_per_second: stats
            .live
            .as_ref()
            .map_or(0, |s| s.download_speed.as_bytes()),
        upload_bytes_per_second: stats.live.as_ref().map_or(0, |s| s.upload_speed.as_bytes()),
        peers: stats
            .live
            .as_ref()
            .map_or(0, |s| s.snapshot.peer_stats.live),
        error: stats.error,
    })
}

fn backend(operation: &'static str, error: impl std::fmt::Display) -> Error {
    Error::Backend {
        operation,
        message: error.to_string(),
    }
}
