mod support;

use rustor_torrent::{AddOptions, Engine, Error, TorrentInput, TorrentState};
use support::{Fixture, config, wait_for};
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread")]
async fn control_save_failures_are_reported_and_payloads_are_preserved() {
    for action in ["pause", "resume", "remove"] {
        let fixture = Fixture::new().await;
        let root = TempDir::new().unwrap();
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
        if action == "resume" {
            engine.pause(&added.id).await.unwrap();
        }
        let database = root.path().join("state/session.json");
        let committed = tokio::fs::read(&database).await.unwrap();
        // Block the native temp-file open without platform-dependent permissions.
        let obstruction = root.path().join("state/session.json.tmp");
        tokio::fs::create_dir(&obstruction).await.unwrap();
        let result = match action {
            "pause" => engine.pause(&added.id).await,
            "resume" => engine.resume(&added.id).await,
            _ => engine.remove(&added.id).await,
        };
        assert!(
            matches!(result, Err(Error::SessionNotSaved)),
            "{action} did not report a failed save: {result:?}"
        );
        assert_eq!(tokio::fs::read(&database).await.unwrap(), committed);
        assert_eq!(
            tokio::fs::read(root.path().join("downloads/fixture.bin"))
                .await
                .unwrap(),
            fixture.payload
        );
        tokio::fs::remove_dir(&obstruction).await.unwrap();
        // The active change may already have happened. A later successful native
        // operation flushes its own database; our adapter must never rewrite it.
        if action != "remove" {
            if action == "resume" {
                // This is already live; repeat resume must still report the
                // uncommitted state rather than claiming an idempotent success.
                assert!(matches!(
                    engine.resume(&added.id).await,
                    Err(Error::SessionNotSaved)
                ));
                engine.pause(&added.id).await.unwrap();
            }
            engine.resume(&added.id).await.unwrap();
            wait_for(&engine, &added.id, TorrentState::Seeding).await;
            engine.pause(&added.id).await.unwrap();
        }
        engine.shutdown().await.unwrap();
        let restored = Engine::open(config(root.path())).await.unwrap();
        assert_eq!(restored.list().await.unwrap().len(), 1);
        wait_for(
            &restored,
            &added.id,
            if action == "remove" {
                TorrentState::Seeding
            } else {
                TorrentState::Paused
            },
        )
        .await;
        restored.remove(&added.id).await.unwrap();
        restored.shutdown().await.unwrap();
        let empty = Engine::open(config(root.path())).await.unwrap();
        assert!(empty.list().await.unwrap().is_empty());
        empty.shutdown().await.unwrap();
        fixture.session.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_saved_metadata_fails_startup_without_replacing_the_database() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
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
    engine.shutdown().await.unwrap();
    let database = root.path().join("state/session.json");
    let committed = tokio::fs::read(&database).await.unwrap();
    let metadata = root
        .path()
        .join(format!("state/{}.torrent", added.id.as_str()));
    let backup = root.path().join("metadata.backup");
    tokio::fs::rename(&metadata, &backup).await.unwrap();
    let opened = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        Engine::open(config(root.path())),
    )
    .await;
    assert!(
        matches!(opened, Ok(Err(Error::RestoreFailed))),
        "startup did not report unavailable saved metadata"
    );
    assert_eq!(tokio::fs::read(&database).await.unwrap(), committed);
    tokio::fs::rename(&backup, &metadata).await.unwrap();
    tokio::fs::write(&metadata, b"corrupt metainfo")
        .await
        .unwrap();
    assert!(matches!(
        Engine::open(config(root.path())).await,
        Err(Error::RestoreFailed)
    ));
    assert_eq!(tokio::fs::read(&database).await.unwrap(), committed);
    tokio::fs::write(&metadata, &fixture.bytes).await.unwrap();
    let restored = Engine::open(config(root.path())).await.unwrap();
    wait_for(&restored, &added.id, TorrentState::Seeding).await;
    restored.shutdown().await.unwrap();
    fixture.session.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_cannot_silently_skip_a_record_when_its_native_save_fails() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
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
    engine.shutdown().await.unwrap();
    let database = root.path().join("state/session.json");
    let committed = tokio::fs::read(&database).await.unwrap();
    let obstruction = root.path().join("state/session.json.tmp");
    tokio::fs::create_dir(&obstruction).await.unwrap();
    assert!(matches!(
        Engine::open(config(root.path())).await,
        Err(Error::RestoreFailed)
    ));
    assert_eq!(tokio::fs::read(&database).await.unwrap(), committed);
    tokio::fs::remove_dir(&obstruction).await.unwrap();
    let restored = Engine::open(config(root.path())).await.unwrap();
    wait_for(&restored, &added.id, TorrentState::Seeding).await;
    restored.shutdown().await.unwrap();
    fixture.session.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_metainfo_save_does_not_leave_a_new_transfer_running() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    let input = TorrentInput::from_bytes(fixture.bytes.clone()).unwrap();
    let obstruction = root
        .path()
        .join(format!("state/{}.torrent", input.id().as_str()));
    tokio::fs::create_dir(&obstruction).await.unwrap();
    let result = engine
        .add(
            input,
            AddOptions {
                initial_peers: vec![fixture.peer],
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(result, Err(Error::SessionNotSaved)));
    assert!(engine.list().await.unwrap().is_empty());
    tokio::fs::remove_dir(&obstruction).await.unwrap();
    engine.shutdown().await.unwrap();
    let restored = Engine::open(config(root.path())).await.unwrap();
    assert!(restored.list().await.unwrap().is_empty());
    restored.shutdown().await.unwrap();
    fixture.session.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_rechecks_every_piece_instead_of_trusting_stale_resume_bits() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
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
    engine.shutdown().await.unwrap();
    let mut changed = fixture.payload.clone();
    changed[16 * 1024..32 * 1024].fill(0);
    tokio::fs::write(root.path().join("downloads/fixture.bin"), &changed)
        .await
        .unwrap();
    let restored = Engine::open(config(root.path())).await.unwrap();
    wait_for(&restored, &added.id, TorrentState::Downloading).await;
    let snapshot = restored.snapshot(&added.id).await.unwrap();
    assert_eq!(
        snapshot.downloaded_bytes,
        fixture.payload.len() as u64 - 16 * 1024
    );
    assert_eq!(
        tokio::fs::read(root.path().join("downloads/fixture.bin"))
            .await
            .unwrap(),
        changed
    );
    restored.shutdown().await.unwrap();
    fixture.session.stop().await;
}
