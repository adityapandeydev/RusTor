mod support;

use rustor_torrent::{AddOptions, Engine, TorrentInput, TorrentState};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use support::{Fixture, config, wait_for};
use tempfile::TempDir;

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_worker(root: &Path, phase: &str, peer: &str) -> Worker {
    Worker(
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "recovery_worker", "--nocapture"])
            .env("RUSTOR_TEST_ROOT", root)
            .env("RUSTOR_TEST_PHASE", phase)
            .env("RUSTOR_TEST_PEER", peer)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}

async fn wait_for_exit(worker: &mut Worker) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = worker.0.try_wait().unwrap() {
            assert!(status.success(), "worker exited with {status}");
            return;
        }
        assert!(Instant::now() < deadline, "worker did not exit");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn kill_at_checkpoint(worker: &mut Worker, root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !root.join("ready").exists() {
        assert!(
            worker.0.try_wait().unwrap().is_none(),
            "worker exited before checkpoint"
        );
        assert!(Instant::now() < deadline, "worker did not reach checkpoint");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    worker.0.kill().unwrap();
    worker.0.wait().unwrap();
}

async fn roundtrip(crash: bool) {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    tokio::fs::write(root.path().join("input.torrent"), &fixture.bytes)
        .await
        .unwrap();
    let peer = fixture.peer.to_string();
    let mut prepare = spawn_worker(root.path(), if crash { "crash" } else { "prepare" }, &peer);
    if crash {
        kill_at_checkpoint(&mut prepare, root.path()).await;
    } else {
        wait_for_exit(&mut prepare).await;
    }
    let mut verify = spawn_worker(
        root.path(),
        if crash {
            "verify_active"
        } else {
            "verify_paused"
        },
        "",
    );
    wait_for_exit(&mut verify).await;
    assert_eq!(
        tokio::fs::read(root.path().join("downloads/fixture.bin"))
            .await
            .unwrap(),
        fixture.payload
    );
    fixture.session.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn paused_state_and_bytes_survive_a_clean_process_restart() {
    roundtrip(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn completed_active_session_survives_forced_process_termination() {
    roundtrip(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn partial_progress_and_bytes_survive_forced_process_termination() {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    tokio::fs::write(root.path().join("input.torrent"), &fixture.bytes)
        .await
        .unwrap();
    let mut worker = spawn_worker(root.path(), "partial", &fixture.peer.to_string());
    kill_at_checkpoint(&mut worker, root.path()).await;
    let payload = root.path().join("downloads/fixture.bin");
    let before = tokio::fs::read(&payload).await.unwrap();
    let mut verify = spawn_worker(root.path(), "verify_partial", "");
    wait_for_exit(&mut verify).await;
    assert_eq!(tokio::fs::read(&payload).await.unwrap(), before);
    assert!(
        before
            .chunks(16 * 1024)
            .zip(fixture.payload.chunks(16 * 1024))
            .any(|(a, b)| a == b)
    );
    assert_ne!(
        before, fixture.payload,
        "worker was killed only after completing"
    );
    fixture.session.stop().await;
}

// Runs in an isolated OS process, not as a normal test in the parent harness.
#[test]
#[ignore = "subprocess helper invoked by the recovery tests"]
fn recovery_worker() {
    let root = PathBuf::from(std::env::var_os("RUSTOR_TEST_ROOT").unwrap());
    let phase = std::env::var("RUSTOR_TEST_PHASE").unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let input =
            TorrentInput::from_bytes(tokio::fs::read(root.join("input.torrent")).await.unwrap())
                .unwrap();
        let id = input.id().clone();
        let mut cfg = config(&root);
        if phase == "partial" {
            cfg.download_limit_bps = std::num::NonZeroU32::new(16 * 1024);
        }
        let engine = Engine::open(cfg).await.unwrap();
        if phase == "prepare" || phase == "crash" || phase == "partial" {
            let peer = std::env::var("RUSTOR_TEST_PEER").unwrap().parse().unwrap();
            engine
                .add(
                    input,
                    AddOptions {
                        initial_peers: vec![peer],
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            if phase == "partial" {
                let downloaded = tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let snapshot = engine.snapshot(&id).await.unwrap();
                        assert_ne!(snapshot.state, TorrentState::Failed, "{:?}", snapshot.error);
                        if snapshot.state == TorrentState::Downloading
                            && snapshot.downloaded_bytes > 0
                        {
                            assert!(snapshot.downloaded_bytes < snapshot.total_bytes);
                            return snapshot.downloaded_bytes;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                tokio::fs::write(root.join("ready.tmp"), downloaded.to_be_bytes())
                    .await
                    .unwrap();
                tokio::fs::rename(root.join("ready.tmp"), root.join("ready"))
                    .await
                    .unwrap();
                std::future::pending::<()>().await;
            }
            wait_for(&engine, &id, TorrentState::Seeding).await;
            if phase == "prepare" {
                engine.pause(&id).await.unwrap();
                wait_for(&engine, &id, TorrentState::Paused).await;
            } else {
                tokio::fs::write(root.join("ready"), b"ready")
                    .await
                    .unwrap();
                std::future::pending::<()>().await;
            }
        } else {
            assert_eq!(engine.list().await.unwrap().len(), 1);
            let expected = if phase == "verify_partial" {
                TorrentState::Downloading
            } else if phase == "verify_paused" {
                TorrentState::Paused
            } else {
                TorrentState::Seeding
            };
            wait_for(&engine, &id, expected).await;
            let snapshot = engine.snapshot(&id).await.unwrap();
            if phase == "verify_partial" {
                let checkpoint = u64::from_be_bytes(
                    tokio::fs::read(root.join("ready"))
                        .await
                        .unwrap()
                        .try_into()
                        .unwrap(),
                );
                assert!(checkpoint > 0);
                assert!(snapshot.downloaded_bytes >= checkpoint);
                assert!(snapshot.downloaded_bytes < snapshot.total_bytes);
            } else {
                assert_eq!(snapshot.downloaded_bytes, snapshot.total_bytes);
            }
            assert!(snapshot.total_bytes > 0);
        }
        engine.shutdown().await.unwrap();
    });
}
