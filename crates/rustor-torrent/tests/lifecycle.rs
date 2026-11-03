mod support;

use rustor_torrent::{AddOptions, Engine, Error, TorrentInput, TorrentState};
use std::{num::NonZeroU32, time::Duration};
use support::{Fixture, config, wait_for};
use tempfile::TempDir;
use tokio::time::{sleep, timeout};

#[tokio::test(flavor = "multi_thread")]
async fn downloads_to_selected_destination_and_deduplicates() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    let destination = root.path().join("selected");
    let options = AddOptions {
        destination: Some(destination.clone()),
        initial_peers: vec![fixture.peer],
        ..Default::default()
    };
    let added = engine
        .add(
            TorrentInput::from_bytes(fixture.bytes.clone()).unwrap(),
            options.clone(),
        )
        .await
        .unwrap();
    assert!(!added.already_managed);
    wait_for(&engine, &added.id, TorrentState::Seeding).await;
    assert_eq!(
        tokio::fs::read(destination.join("fixture.bin"))
            .await
            .unwrap(),
        fixture.payload
    );
    let duplicate = engine
        .add(
            TorrentInput::from_bytes(fixture.bytes.clone()).unwrap(),
            options,
        )
        .await
        .unwrap();
    assert!(duplicate.already_managed);
    assert_eq!(duplicate.id, added.id);
    assert_eq!(engine.list().await.unwrap().len(), 1);
    let snapshot = engine.snapshot(&added.id).await.unwrap();
    assert_eq!(snapshot.downloaded_bytes, fixture.payload.len() as u64);
    assert_eq!(snapshot.total_bytes, fixture.payload.len() as u64);
    engine.shutdown().await.unwrap();
    assert!(matches!(engine.list().await, Err(Error::Stopped)));
    fixture.session.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pauses_an_active_transfer_then_resumes_verified_bytes() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    let mut cfg = config(root.path());
    cfg.download_limit_bps = NonZeroU32::new(128 * 1024);
    let engine = Engine::open(cfg).await.unwrap();
    let added = engine
        .add(
            TorrentInput::from_bytes(fixture.bytes.clone()).unwrap(),
            AddOptions {
                initial_peers: vec![fixture.peer],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = engine.snapshot(&added.id).await.unwrap();
            if snapshot.downloaded_bytes > 0 && snapshot.state == TorrentState::Downloading {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    engine.pause(&added.id).await.unwrap();
    wait_for(&engine, &added.id, TorrentState::Paused).await;
    let paused_bytes = engine.snapshot(&added.id).await.unwrap().downloaded_bytes;
    sleep(Duration::from_millis(150)).await;
    assert_eq!(
        engine.snapshot(&added.id).await.unwrap().downloaded_bytes,
        paused_bytes
    );
    assert!(paused_bytes < fixture.payload.len() as u64);
    engine.resume(&added.id).await.unwrap();
    wait_for(&engine, &added.id, TorrentState::Seeding).await;
    assert_eq!(
        tokio::fs::read(root.path().join("downloads/fixture.bin"))
            .await
            .unwrap(),
        fixture.payload
    );
    engine.shutdown().await.unwrap();
    fixture.session.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn resolves_magnet_metadata_and_downloads_without_public_discovery() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    let input = TorrentInput::from_bytes(fixture.bytes.clone()).unwrap();
    let magnet = format!("magnet:?xt=urn:btih:{}", input.id().as_str());
    let added = engine
        .add(
            TorrentInput::parse(&magnet).unwrap(),
            AddOptions {
                initial_peers: vec![fixture.peer],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_for(&engine, &added.id, TorrentState::Seeding).await;
    assert_eq!(
        tokio::fs::read(root.path().join("downloads/fixture.bin"))
            .await
            .unwrap(),
        fixture.payload
    );
    engine.shutdown().await.unwrap();
    fixture.session.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_adds_of_the_same_torrent_create_one_transfer() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    let options = AddOptions {
        initial_peers: vec![fixture.peer],
        ..Default::default()
    };
    let (first, second) = tokio::join!(
        engine.add(
            TorrentInput::from_bytes(fixture.bytes.clone()).unwrap(),
            options.clone()
        ),
        engine.add(
            TorrentInput::from_bytes(fixture.bytes.clone()).unwrap(),
            options
        )
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first.id, second.id);
    assert_ne!(first.already_managed, second.already_managed);
    assert_eq!(engine.list().await.unwrap().len(), 1);
    wait_for(&engine, &first.id, TorrentState::Seeding).await;
    engine.shutdown().await.unwrap();
    fixture.session.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_torrents_download_concurrently_in_one_session() {
    let (first, second) = tokio::join!(Fixture::named("first.bin"), Fixture::named("second.bin"));
    let root = TempDir::new().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    let (a, b) = tokio::join!(
        engine.add(
            TorrentInput::from_bytes(first.bytes.clone()).unwrap(),
            AddOptions {
                initial_peers: vec![first.peer],
                ..Default::default()
            }
        ),
        engine.add(
            TorrentInput::from_bytes(second.bytes.clone()).unwrap(),
            AddOptions {
                initial_peers: vec![second.peer],
                ..Default::default()
            }
        )
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a.id, b.id);
    tokio::join!(
        wait_for(&engine, &a.id, TorrentState::Seeding),
        wait_for(&engine, &b.id, TorrentState::Seeding)
    );
    assert_eq!(engine.list().await.unwrap().len(), 2);
    assert_eq!(
        tokio::fs::read(root.path().join("downloads/first.bin"))
            .await
            .unwrap(),
        first.payload
    );
    assert_eq!(
        tokio::fs::read(root.path().join("downloads/second.bin"))
            .await
            .unwrap(),
        second.payload
    );
    engine.shutdown().await.unwrap();
    tokio::join!(first.session.stop(), second.session.stop());
}

#[tokio::test(flavor = "multi_thread")]
async fn existing_unrelated_file_is_not_overwritten() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    let path = root.path().join("downloads/fixture.bin");
    tokio::fs::write(&path, b"user-owned file").await.unwrap();
    let outcome = engine
        .add(
            TorrentInput::from_bytes(fixture.bytes.clone()).unwrap(),
            AddOptions {
                initial_peers: vec![fixture.peer],
                ..Default::default()
            },
        )
        .await;
    if let Ok(added) = outcome {
        wait_for_failure(&engine, &added.id).await;
    }
    assert_eq!(tokio::fs::read(&path).await.unwrap(), b"user-owned file");
    engine.shutdown().await.unwrap();
    fixture.session.stop().await;
}

async fn wait_for_failure(engine: &Engine, id: &rustor_torrent::TorrentId) {
    timeout(Duration::from_secs(5), async {
        while engine.snapshot(id).await.unwrap().state != TorrentState::Failed {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_seeder_keeps_files_and_does_not_restore_it() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    let cfg = config(root.path());
    let engine = Engine::open(config(root.path())).await.unwrap();
    let added = engine
        .add(
            TorrentInput::from_bytes(fixture.bytes.clone()).unwrap(),
            AddOptions {
                initial_peers: vec![fixture.peer],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_for(&engine, &added.id, TorrentState::Seeding).await;
    engine.remove(&added.id).await.unwrap();
    assert!(engine.list().await.unwrap().is_empty());
    assert!(matches!(
        engine.snapshot(&added.id).await,
        Err(Error::NotFound)
    ));
    assert_eq!(
        tokio::fs::read(root.path().join("downloads/fixture.bin"))
            .await
            .unwrap(),
        fixture.payload
    );
    engine.shutdown().await.unwrap();
    let restored = Engine::open(cfg).await.unwrap();
    assert!(restored.list().await.unwrap().is_empty());
    restored.shutdown().await.unwrap();
    fixture.session.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn removes_an_incomplete_download_without_deleting_its_partial_file() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    let cfg = config(root.path());
    let engine = Engine::open(config(root.path())).await.unwrap();
    let added = engine
        .add(
            TorrentInput::from_bytes(fixture.bytes.clone()).unwrap(),
            AddOptions {
                paused: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_for(&engine, &added.id, TorrentState::Paused).await;
    assert!(
        engine.snapshot(&added.id).await.unwrap().downloaded_bytes < fixture.payload.len() as u64
    );
    engine.resume(&added.id).await.unwrap();
    engine.remove(&added.id).await.unwrap();
    assert!(engine.list().await.unwrap().is_empty());
    assert!(root.path().join("downloads/fixture.bin").exists());
    assert!(matches!(
        engine.resume(&added.id).await,
        Err(Error::NotFound)
    ));
    engine.shutdown().await.unwrap();
    let restored = Engine::open(cfg).await.unwrap();
    assert!(restored.list().await.unwrap().is_empty());
    restored.shutdown().await.unwrap();
    fixture.session.stop().await;
}
