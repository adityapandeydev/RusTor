use std::{
    collections::BTreeMap,
    net::SocketAddr,
    num::{NonZeroU32, NonZeroUsize},
    path::PathBuf,
    sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use librqbit::{
    AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListenerOptions, ManagedTorrent,
    Session, SessionOptions, SessionPersistenceConfig, TorrentStatsState, api::TorrentIdOrHash,
    limits::LimitsConfig, storage::StorageFactoryExt,
};
use serde::Serialize;
use tokio::{
    sync::{Mutex, OwnedMutexGuard, RwLock},
    time::timeout,
};

use crate::{Error, TorrentId, TorrentInput};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SpeedLimits {
    pub download_bps: Option<NonZeroU32>,
    pub upload_bps: Option<NonZeroU32>,
}
impl SpeedLimits {
    // The native governor quota uses whole nanoseconds per byte. Larger rates
    // round its interval to zero and cannot represent a finite limit.
    pub const MAX_BPS: u32 = 1_000_000_000;
    fn validate(self) -> Result<(), Error> {
        if [self.download_bps, self.upload_bps]
            .into_iter()
            .flatten()
            .any(|rate| !(16 * 1024..=Self::MAX_BPS).contains(&rate.get()))
        {
            return Err(Error::InvalidInput(
                "speed limits must be unlimited or between 16 KiB/s and 1,000,000,000 bytes/s",
            ));
        }
        Ok(())
    }
}

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

#[derive(Clone, Debug, Serialize)]
pub struct TorrentFile {
    pub index: usize,
    pub path: String,
    pub length: u64,
    pub selected: bool,
}

/// Resolved metadata only. Preparing never creates payload storage or a saved transfer.
#[derive(Serialize)]
pub struct PreparedTorrent {
    pub id: TorrentId,
    pub name: Option<String>,
    pub files: Vec<TorrentFile>,
    #[serde(skip)]
    input: TorrentInput,
    #[serde(skip)]
    peers: Vec<SocketAddr>,
    #[serde(skip)]
    relative_output: PathBuf,
}

impl PreparedTorrent {
    /// Preserve native subfolder placement when using a different default root.
    pub fn destination_under(&self, root: &std::path::Path) -> PathBuf {
        root.join(&self.relative_output)
    }
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

#[derive(Clone, Debug, Serialize)]
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
    download_dir: PathBuf,
    persistence: crate::persistence::Persistence,
    torrent_operations: Mutex<BTreeMap<TorrentId, Weak<Mutex<()>>>>,
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
        SpeedLimits {
            download_bps: config.download_limit_bps,
            upload_bps: config.upload_limit_bps,
        }
        .validate()
        .map_err(|_| {
            Error::InvalidConfig("initial speed limits are outside the supported range")
        })?;
        // Payload storage is opened when a transfer starts. An unavailable
        // default must not block previews or transfers at other destinations.
        tokio::fs::create_dir_all(&config.state_dir)
            .await
            .map_err(|e| backend("create engine directory", e))?;
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
        let persistence = crate::persistence::Persistence::new(config.state_dir.clone());
        let saved = persistence.preflight().await?;
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
            // Recheck payload hashes after restart instead of trusting a bitfield
            // that may have been interrupted alongside the last disk write.
            fastresume: false,
            concurrent_init_limit: Some(config.max_initializations.get()),
            peer_limit: Some(config.peer_limit.get()),
            ratelimits: LimitsConfig {
                download_bps: config.download_limit_bps,
                upload_bps: config.upload_limit_bps,
            },
            client_name_and_version: Some(concat!("RusTor/", env!("CARGO_PKG_VERSION")).to_owned()),
            ..Default::default()
        };
        let download_dir = config.download_dir.clone();
        let session = Session::new_with_opts(config.download_dir, options)
            .await
            .map_err(|e| backend("open torrent session", e))?;
        if let Err(error) = saved.verify_restored(&session) {
            session.stop().await;
            return Err(error);
        }
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
            download_dir,
            persistence,
            torrent_operations: Mutex::new(BTreeMap::new()),
        })
    }

    async fn lock_torrent(&self, id: &TorrentId) -> OwnedMutexGuard<()> {
        let gate = {
            let mut gates = self.torrent_operations.lock().await;
            gates.retain(|_, gate| gate.strong_count() > 0);
            if let Some(gate) = gates.get(id).and_then(Weak::upgrade) {
                gate
            } else {
                let gate = Arc::new(Mutex::new(()));
                gates.insert(id.clone(), Arc::downgrade(&gate));
                gate
            }
        };
        gate.lock_owned().await
    }

    pub async fn prepare(
        &self,
        input: TorrentInput,
        peers: Vec<SocketAddr>,
    ) -> Result<PreparedTorrent, Error> {
        let guard = self.session.read().await;
        let session = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let expected_id = input.id().clone();
        let response = timeout(
            self.metadata_timeout,
            session.add_torrent(
                input.into_native(),
                Some(AddTorrentOptions {
                    list_only: true,
                    initial_peers: Some(peers.clone()),
                    disable_trackers: !self.discovery,
                    ..Default::default()
                }),
            ),
        )
        .await
        .map_err(|_| Error::MetadataTimeout)?
        .map_err(|e| backend("prepare torrent", e))?;
        let (metadata, mut seen_peers, relative_output) = match response {
            AddTorrentResponse::ListOnly(list) => {
                let input = TorrentInput::from_bytes(list.torrent_bytes.to_vec())?;
                let files = list
                    .info
                    .iter_file_details()
                    .enumerate()
                    .map(|(index, f)| TorrentFile {
                        index,
                        path: f.filename.to_string(),
                        length: f.len,
                        selected: list
                            .only_files
                            .as_ref()
                            .is_none_or(|ids| ids.contains(&index)),
                    })
                    .collect::<Vec<_>>();
                if files.is_empty() || files.len() > 10_000 {
                    return Err(Error::InvalidInput(
                        "file list must contain between 1 and 10,000 files",
                    ));
                }
                let name = list.info.name().map(|n| n.into_owned());
                (
                    (input, name, files),
                    list.seen_peers,
                    list.output_folder
                        .strip_prefix(&self.download_dir)
                        .map_err(|e| backend("prepare destination", e))?
                        .to_owned(),
                )
            }
            AddTorrentResponse::AlreadyManaged(_, _) => {
                return Err(Error::InvalidInput(
                    "this torrent is already in your session",
                ));
            }
            AddTorrentResponse::Added(_, _) => {
                return Err(backend("prepare torrent", "unexpected managed transfer"));
            }
        };
        let (input, name, files) = metadata;
        if input.id() != &expected_id {
            return Err(backend("prepare torrent", "metadata identity mismatch"));
        }
        seen_peers.extend(peers);
        seen_peers.sort_unstable();
        seen_peers.dedup();
        Ok(PreparedTorrent {
            id: expected_id,
            name,
            files,
            input,
            peers: seen_peers,
            relative_output,
        })
    }

    pub async fn add_prepared(
        &self,
        prepared: &PreparedTorrent,
        files: Vec<usize>,
        mut options: AddOptions,
    ) -> Result<AddOutcome, Error> {
        if files.is_empty()
            || files.len() > prepared.files.len()
            || files.iter().any(|&i| i >= prepared.files.len())
        {
            return Err(Error::InvalidInput("choose at least one valid file"));
        }
        let unique = files
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        if unique.len() != files.len() {
            return Err(Error::InvalidInput("file selection contains duplicates"));
        }
        options.initial_peers.extend_from_slice(&prepared.peers);
        self.add_inner(prepared.input.clone(), options, Some(files))
            .await
    }

    pub async fn files(&self, id: &TorrentId) -> Result<Vec<TorrentFile>, Error> {
        let guard = self.session.read().await;
        let torrent = handle(&guard.as_ref().ok_or(Error::Stopped)?.native, id)?;
        let metadata = torrent
            .metadata
            .load_full()
            .ok_or_else(|| backend("list files", "metadata unavailable"))?;
        let selected = torrent.only_files();
        Ok(metadata
            .info
            .iter_file_details()
            .enumerate()
            .map(|(index, f)| TorrentFile {
                index,
                path: f.filename.to_string(),
                length: f.len,
                selected: selected.as_ref().is_none_or(|ids| ids.contains(&index)),
            })
            .collect())
    }

    pub async fn add(&self, input: TorrentInput, options: AddOptions) -> Result<AddOutcome, Error> {
        self.add_inner(input, options, None).await
    }

    async fn add_inner(
        &self,
        input: TorrentInput,
        options: AddOptions,
        only_files: Option<Vec<usize>>,
    ) -> Result<AddOutcome, Error> {
        let _operation = self.lock_torrent(input.id()).await;
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
                    only_files,
                    initial_peers: Some(options.initial_peers),
                    disable_trackers: !self.discovery,
                    // Existing files are protected. Engine-owned restore explicitly enables
                    // overwrite for its already-managed data, followed by verification.
                    overwrite: false,
                    storage_factory: Some(
                        crate::retry_storage::RetryStorageFactory::default().boxed(),
                    ),
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
        let persisted = async {
            self.persistence.verify_present(&handle).await?;
            self.persistence.verify_metainfo(&id).await
        }
        .await;
        if let Err(error) = persisted {
            if !already_managed {
                // A failed add must not silently keep downloading. Keep any
                // payload bytes while asking the native owner to forget it.
                let _ = session
                    .delete(TorrentIdOrHash::Id(handle.id()), false)
                    .await;
            }
            return Err(error);
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

    pub async fn set_speed_limits(&self, limits: SpeedLimits) -> Result<(), Error> {
        limits.validate()?;
        let _control = self.controls.lock().await;
        let guard = self.session.read().await;
        let native = &guard.as_ref().ok_or(Error::Stopped)?.native;
        native.ratelimits.set_download_bps(limits.download_bps);
        native.ratelimits.set_upload_bps(limits.upload_bps);
        Ok(())
    }

    pub async fn speed_limits(&self) -> Result<SpeedLimits, Error> {
        let _control = self.controls.lock().await;
        let guard = self.session.read().await;
        let native = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let limits = native.ratelimits.get_config();
        Ok(SpeedLimits {
            download_bps: limits.download_bps,
            upload_bps: limits.upload_bps,
        })
    }

    pub async fn pause(&self, id: &TorrentId) -> Result<(), Error> {
        let _operation = self.lock_torrent(id).await;
        let _control = self.controls.lock().await;
        let guard = self.session.read().await;
        let session = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let handle = handle(session, id)?;
        if handle.is_paused() {
            return self.persistence.verify_present(&handle).await;
        }
        session
            .pause(&handle)
            .await
            .map_err(|e| backend("pause torrent", e))?;
        self.persistence.verify_present(&handle).await
    }

    pub async fn resume(&self, id: &TorrentId) -> Result<(), Error> {
        let _operation = self.lock_torrent(id).await;
        let _control = self.controls.lock().await;
        let guard = self.session.read().await;
        let session = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let handle = handle(session, id)?;
        if !handle.is_paused() {
            return self.persistence.verify_present(&handle).await;
        }
        session
            .unpause(&handle)
            .await
            .map_err(|e| backend("resume torrent", e))?;
        self.persistence.verify_present(&handle).await
    }

    /// Retry only a failed transfer, rechecking its existing data in place.
    pub async fn retry(&self, id: &TorrentId) -> Result<(), Error> {
        let _operation = self.lock_torrent(id).await;
        let _control = self.controls.lock().await;
        let guard = self.session.read().await;
        let session = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let torrent = handle(session, id)?;
        if !matches!(torrent.stats().state, TorrentStatsState::Error) {
            return Err(Error::InvalidInput("only failed transfers can be retried"));
        }
        session
            .unpause(&torrent)
            .await
            .map_err(|e| backend("retry torrent", e))?;
        self.persistence.verify_present(&torrent).await
    }

    /// Stop downloading/seeding and forget the torrent. Payload files are always kept.
    pub async fn remove(&self, id: &TorrentId) -> Result<(), Error> {
        let _operation = self.lock_torrent(id).await;
        let _control = self.controls.lock().await;
        let guard = self.session.read().await;
        let session = &guard.as_ref().ok_or(Error::Stopped)?.native;
        let torrent = handle(session, id)?;
        session
            .delete(TorrentIdOrHash::Id(torrent.id()), false)
            .await
            .map_err(|e| backend("remove torrent", e))?;
        self.persistence.verify_removed(id).await
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

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use librqbit::{
        AddTorrent, CreateTorrentOptions, create_torrent, spawn_utils::BlockingSpawner,
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn retry_rechecks_owned_files_preserves_selection_and_does_not_duplicate() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("seed/collection");
        tokio::fs::create_dir_all(&source).await.unwrap();
        let data = vec![42u8; 128 * 1024];
        tokio::fs::write(source.join("wanted.bin"), &data)
            .await
            .unwrap();
        tokio::fs::write(source.join("skip.bin"), vec![9u8; 128 * 1024])
            .await
            .unwrap();
        let bytes = create_torrent(
            &source,
            CreateTorrentOptions {
                piece_length: Some(16 * 1024),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap();
        let seeder = Session::new_with_opts(
            root.path().join("seed"),
            SessionOptions {
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                listen: Some(ListenerOptions {
                    listen_addr: "127.0.0.1:0".parse().unwrap(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        seeder
            .add_torrent(
                AddTorrent::from_bytes(bytes.clone()),
                Some(AddTorrentOptions {
                    overwrite: true,
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap()
            .wait_until_completed()
            .await
            .unwrap();
        let mut config =
            EngineConfig::new(root.path().join("downloads"), root.path().join("state"));
        config.discovery = false;
        let engine = Engine::open(config).await.unwrap();
        let prepared = engine
            .prepare(
                TorrentInput::from_bytes(bytes.to_vec()).unwrap(),
                vec![seeder.listen_addr().unwrap()],
            )
            .await
            .unwrap();
        let wanted = prepared
            .files
            .iter()
            .find(|f| f.path.ends_with("wanted.bin"))
            .unwrap()
            .index;
        crate::retry_storage::FAIL_WRITES.store(true, Ordering::Release);
        let added = engine
            .add_prepared(&prepared, vec![wanted], AddOptions::default())
            .await
            .unwrap();
        timeout(Duration::from_secs(5), async {
            while engine.snapshot(&added.id).await.unwrap().state != TorrentState::Failed {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        // A persistent write error leaves the same transfer retryable.
        engine.retry(&added.id).await.unwrap();
        timeout(Duration::from_secs(5), async {
            while engine.snapshot(&added.id).await.unwrap().state != TorrentState::Failed {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(engine.list().await.unwrap().len(), 1);
        crate::retry_storage::FAIL_WRITES.store(false, Ordering::Release);
        let path = root.path().join("downloads/collection/wanted.bin");
        // A verified piece must survive recovery, while the remaining bytes are downloaded.
        let mut partial = vec![0u8; data.len()];
        partial[..16 * 1024].copy_from_slice(&data[..16 * 1024]);
        tokio::fs::write(&path, &partial).await.unwrap();
        assert_eq!(
            engine.snapshot(&added.id).await.unwrap().state,
            TorrentState::Failed
        );
        engine.retry(&added.id).await.unwrap();
        timeout(Duration::from_secs(10), async {
            loop {
                let state = engine.snapshot(&added.id).await.unwrap();
                assert_ne!(state.state, TorrentState::Failed, "{:?}", state.error);
                if state.state == TorrentState::Seeding {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), data);
        assert_eq!(
            engine
                .files(&added.id)
                .await
                .unwrap()
                .iter()
                .filter(|f| f.selected)
                .map(|f| f.index)
                .collect::<Vec<_>>(),
            vec![wanted]
        );
        assert_eq!(engine.list().await.unwrap().len(), 1);
        assert!(matches!(
            engine.retry(&added.id).await,
            Err(Error::InvalidInput(_))
        ));
        engine.shutdown().await.unwrap();
        let mut config =
            EngineConfig::new(root.path().join("downloads"), root.path().join("state"));
        config.discovery = false;
        let restored = Engine::open(config).await.unwrap();
        timeout(Duration::from_secs(5), async {
            while restored.snapshot(&added.id).await.unwrap().state != TorrentState::Seeding {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            restored
                .files(&added.id)
                .await
                .unwrap()
                .iter()
                .filter(|f| f.selected)
                .map(|f| f.index)
                .collect::<Vec<_>>(),
            vec![wanted]
        );
        restored.shutdown().await.unwrap();
        seeder.stop().await;
    }
}
