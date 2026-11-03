mod support;

use librqbit::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session, SessionOptions,
    create_torrent, spawn_utils::BlockingSpawner,
};
use rustor_torrent::{AddOptions, Engine, Error, TorrentInput, TorrentState};
use support::{config, wait_for};

#[tokio::test(flavor = "multi_thread")]
async fn previews_without_payload_then_downloads_only_selected_files_and_restores_selection() {
    let root = tempfile::tempdir().unwrap();
    let seed = root.path().join("seed");
    let source = seed.join("collection");
    tokio::fs::create_dir_all(&source).await.unwrap();
    let wanted = vec![41u8; 128 * 1024];
    let skipped = vec![73u8; 128 * 1024];
    tokio::fs::write(source.join("wanted.bin"), &wanted)
        .await
        .unwrap();
    tokio::fs::write(source.join("skipped.bin"), &skipped)
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
        seed,
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
    let peer = seeder.listen_addr().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    let input = TorrentInput::from_bytes(bytes.to_vec()).unwrap();
    let magnet =
        TorrentInput::parse(&format!("magnet:?xt=urn:btih:{}", input.id().as_str())).unwrap();
    let prepared = engine.prepare(magnet, vec![peer]).await.unwrap();
    assert_eq!(prepared.files.len(), 2);
    assert_eq!(
        prepared.destination_under(&root.path().join("new-root")),
        root.path().join("new-root/collection")
    );
    assert!(engine.list().await.unwrap().is_empty());
    assert!(!root.path().join("downloads/collection").exists());
    let wanted_id = prepared
        .files
        .iter()
        .find(|f| f.path.ends_with("wanted.bin"))
        .unwrap()
        .index;
    assert!(matches!(
        engine
            .add_prepared(&prepared, vec![], AddOptions::default())
            .await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        engine
            .add_prepared(&prepared, vec![2], AddOptions::default())
            .await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        engine
            .add_prepared(&prepared, vec![wanted_id, wanted_id], AddOptions::default())
            .await,
        Err(Error::InvalidInput(_))
    ));
    let added = engine
        .add_prepared(&prepared, vec![wanted_id], AddOptions::default())
        .await
        .unwrap();
    wait_for(&engine, &added.id, TorrentState::Seeding).await;
    assert_eq!(
        tokio::fs::read(root.path().join("downloads/collection/wanted.bin"))
            .await
            .unwrap(),
        wanted
    );
    let skipped_path = root.path().join("downloads/collection/skipped.bin");
    if skipped_path.exists() {
        assert_ne!(tokio::fs::read(&skipped_path).await.unwrap(), skipped);
    }
    assert_eq!(
        engine
            .files(&added.id)
            .await
            .unwrap()
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.index)
            .collect::<Vec<_>>(),
        vec![wanted_id]
    );
    engine.shutdown().await.unwrap();
    let restored = Engine::open(config(root.path())).await.unwrap();
    wait_for(&restored, &added.id, TorrentState::Seeding).await;
    assert_eq!(
        restored
            .files(&added.id)
            .await
            .unwrap()
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.index)
            .collect::<Vec<_>>(),
        vec![wanted_id]
    );
    restored.shutdown().await.unwrap();
    seeder.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn local_metainfo_preview_can_be_abandoned_without_creating_a_transfer() {
    let fixture = support::Fixture::new().await;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::open(config(root.path())).await.unwrap();
    let preview = engine
        .prepare(
            TorrentInput::from_bytes(fixture.bytes.clone()).unwrap(),
            vec![fixture.peer],
        )
        .await
        .unwrap();
    assert_eq!(preview.files[0].path, "fixture.bin");
    assert!(preview.files[0].selected);
    assert_eq!(preview.files[0].length, fixture.payload.len() as u64);
    drop(preview);
    assert!(engine.list().await.unwrap().is_empty());
    engine.shutdown().await.unwrap();
    fixture.session.stop().await;
}
