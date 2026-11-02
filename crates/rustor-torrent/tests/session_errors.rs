use rustor_torrent::{AddOptions, Engine, EngineConfig, Error, TorrentId, TorrentInput};
use std::time::Duration;
use tempfile::TempDir;

fn config(root: &std::path::Path) -> EngineConfig {
    let mut config = EngineConfig::new(root.join("downloads"), root.join("state"));
    config.discovery = false;
    config
}

#[tokio::test]
async fn rejects_a_second_session_using_the_same_state_directory() {
    let root = TempDir::new().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    assert!(matches!(
        Engine::open(config(root.path())).await,
        Err(Error::StateInUse)
    ));
    engine.shutdown().await.unwrap();
    let next = Engine::open(config(root.path())).await.unwrap();
    next.shutdown().await.unwrap();
}

#[tokio::test]
async fn rejects_relative_paths_and_preserves_corrupt_state_for_diagnosis() {
    let cfg = EngineConfig::new("relative".into(), "state".into());
    assert!(matches!(
        Engine::open(cfg).await,
        Err(Error::InvalidConfig(_))
    ));
    let root = TempDir::new().unwrap();
    let cfg = config(root.path());
    tokio::fs::create_dir_all(&cfg.state_dir).await.unwrap();
    let path = cfg.state_dir.join("session.json");
    tokio::fs::write(&path, b"not JSON").await.unwrap();
    assert!(matches!(
        Engine::open(cfg).await,
        Err(Error::Backend { .. })
    ));
    assert_eq!(tokio::fs::read(path).await.unwrap(), b"not JSON");
}

#[tokio::test(flavor = "multi_thread")]
async fn unresolved_magnet_times_out_without_blocking_snapshots() {
    let root = TempDir::new().unwrap();
    let mut cfg = config(root.path());
    cfg.metadata_timeout = Duration::from_millis(100);
    let engine = Engine::open(cfg).await.unwrap();
    // A real TCP connection whose peer never sends a handshake keeps metadata
    // pending. An empty peer stream can fail immediately, without a timeout.
    let silent_peer = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let input = TorrentInput::parse("0123456789abcdef0123456789abcdef01234567").unwrap();
    let add = engine.add(
        input,
        AddOptions {
            initial_peers: vec![silent_peer.local_addr().unwrap()],
            ..Default::default()
        },
    );
    let snapshot = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        tokio::time::timeout(Duration::from_millis(50), engine.list())
            .await
            .unwrap()
            .unwrap()
    };
    let (result, snapshots) = tokio::join!(add, snapshot);
    assert!(matches!(result, Err(Error::MetadataTimeout)), "{result:?}");
    assert!(snapshots.is_empty());
    engine.shutdown().await.unwrap();
    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_torrent_and_stopped_engine_return_typed_errors() {
    let root = TempDir::new().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    let id = TorrentId::parse("0123456789abcdef0123456789abcdef01234567").unwrap();
    assert!(matches!(engine.snapshot(&id).await, Err(Error::NotFound)));
    assert!(matches!(engine.pause(&id).await, Err(Error::NotFound)));
    engine.shutdown().await.unwrap();
    assert!(matches!(engine.resume(&id).await, Err(Error::Stopped)));
}
