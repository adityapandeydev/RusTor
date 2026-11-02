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

async fn roundtrip(crash: bool) {
    let fixture = Fixture::new().await;
    let root = TempDir::new().unwrap();
    tokio::fs::write(root.path().join("input.torrent"), &fixture.bytes)
        .await
        .unwrap();
    let peer = fixture.peer.to_string();
    let mut prepare = spawn_worker(root.path(), if crash { "crash" } else { "prepare" }, &peer);
    if crash {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !root.path().join("ready").exists() {
            assert!(
                prepare.0.try_wait().unwrap().is_none(),
                "worker exited before checkpoint"
            );
            assert!(Instant::now() < deadline, "worker did not reach checkpoint");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        prepare.0.kill().unwrap();
        prepare.0.wait().unwrap();
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
        let engine = Engine::open(config(&root)).await.unwrap();
        if phase == "prepare" || phase == "crash" {
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
            let expected = if phase == "verify_paused" {
                TorrentState::Paused
            } else {
                TorrentState::Seeding
            };
            wait_for(&engine, &id, expected).await;
            let snapshot = engine.snapshot(&id).await.unwrap();
            assert_eq!(snapshot.downloaded_bytes, snapshot.total_bytes);
            assert!(snapshot.total_bytes > 0);
        }
        engine.shutdown().await.unwrap();
    });
}
