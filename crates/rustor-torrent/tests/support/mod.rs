use std::{
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    time::Duration,
};

use librqbit::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerMode, ListenerOptions, Session,
    SessionOptions, create_torrent, spawn_utils::BlockingSpawner,
};
use rustor_torrent::{Engine, EngineConfig, TorrentState};
use tempfile::TempDir;
use tokio::time::{sleep, timeout};

pub struct Fixture {
    _dir: TempDir,
    pub session: std::sync::Arc<Session>,
    pub bytes: Vec<u8>,
    pub payload: Vec<u8>,
    pub peer: SocketAddr,
}

impl Fixture {
    pub async fn new() -> Self {
        Self::named("fixture.bin").await
    }

    pub async fn named(name: &str) -> Self {
        let dir = TempDir::new().unwrap();
        let payload: Vec<_> = (0..512 * 1024)
            .map(|i| ((i * 17 + i / 251) % 256) as u8)
            .collect();
        let file = dir.path().join(name);
        tokio::fs::write(&file, &payload).await.unwrap();
        let torrent = create_torrent(
            &file,
            CreateTorrentOptions {
                piece_length: Some(16 * 1024),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap();
        let bytes = torrent.as_bytes().unwrap().to_vec();
        let session = Session::new_with_opts(
            dir.path().to_owned(),
            SessionOptions {
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                listen: Some(ListenerOptions {
                    mode: ListenerMode::TcpOnly,
                    listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let handle = session
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
            .unwrap();
        timeout(Duration::from_secs(10), handle.wait_until_completed())
            .await
            .unwrap()
            .unwrap();
        let peer = session.listen_addr().unwrap();
        Self {
            _dir: dir,
            session,
            bytes,
            payload,
            peer,
        }
    }
}

pub fn config(root: &Path) -> EngineConfig {
    let mut config = EngineConfig::new(root.join("downloads"), root.join("state"));
    config.discovery = false;
    config
}

pub async fn wait_for(engine: &Engine, id: &rustor_torrent::TorrentId, state: TorrentState) {
    timeout(Duration::from_secs(20), async {
        loop {
            let snapshot = engine.snapshot(id).await.unwrap();
            assert_ne!(snapshot.state, TorrentState::Failed, "{:?}", snapshot.error);
            if snapshot.state == state {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("torrent did not reach expected state");
}
