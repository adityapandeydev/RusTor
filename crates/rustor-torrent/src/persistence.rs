//! Read-only checks for librqbit 9.1.0's JSON store. The native session remains
//! its only writer; no independent manifest or competing recovery owner is added.
use crate::{Error, TorrentId, TorrentInput};
use librqbit::{ManagedTorrent, Session, api::TorrentIdOrHash};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncReadExt;

const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;
const MAX_METAINFO_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) struct Persistence {
    directory: PathBuf,
}

#[derive(Default, Deserialize)]
pub(crate) struct SavedSession {
    torrents: BTreeMap<usize, SavedTorrent>,
}

#[derive(Deserialize)]
struct SavedTorrent {
    info_hash: String,
    is_paused: bool,
    output_folder: PathBuf,
    only_files: Option<Vec<usize>>,
}

impl Persistence {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    async fn read(&self, allow_missing: bool) -> Result<SavedSession, Error> {
        let path = self.directory.join("session.json");
        let bytes = match read_bounded(&path, MAX_SESSION_BYTES).await {
            Ok(bytes) => bytes,
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SavedSession::default());
            }
            Err(_) => return Err(Error::SessionNotSaved),
        };
        tokio::task::spawn_blocking(move || serde_json::from_slice(&bytes))
            .await
            .map_err(|_| Error::SessionNotSaved)?
            .map_err(|_| Error::SessionNotSaved)
    }

    /// Fail before native restore can fall back to unbounded network metadata
    /// resolution or silently omit a record with missing/corrupt metainfo.
    pub async fn preflight(&self) -> Result<SavedSession, Error> {
        let saved = self.read(true).await.map_err(|_| Error::RestoreFailed)?;
        for torrent in saved.torrents.values() {
            let id = TorrentId::parse(&torrent.info_hash).map_err(|_| Error::RestoreFailed)?;
            if torrent.info_hash != id.as_str() || !torrent.output_folder.is_absolute() {
                return Err(Error::RestoreFailed);
            }
            self.verify_metainfo(&id)
                .await
                .map_err(|_| Error::RestoreFailed)?;
        }
        Ok(saved)
    }

    pub async fn verify_metainfo(&self, id: &TorrentId) -> Result<(), Error> {
        let bytes = read_bounded(
            &self.directory.join(format!("{}.torrent", id.as_str())),
            MAX_METAINFO_BYTES,
        )
        .await
        .map_err(|_| Error::SessionNotSaved)?;
        let parsed = tokio::task::spawn_blocking(move || TorrentInput::from_bytes(bytes))
            .await
            .map_err(|_| Error::SessionNotSaved)?
            .map_err(|_| Error::SessionNotSaved)?;
        if parsed.id() != id {
            return Err(Error::SessionNotSaved);
        }
        Ok(())
    }

    pub async fn verify_present(&self, handle: &ManagedTorrent) -> Result<(), Error> {
        confirm_saved(|| self.verify_present_once(handle)).await
    }

    async fn verify_present_once(&self, handle: &ManagedTorrent) -> Result<(), Error> {
        let saved = self.read(false).await?;
        if !saved
            .torrents
            .get(&handle.id())
            .is_some_and(|row| row.matches(handle))
        {
            return Err(Error::SessionNotSaved);
        }
        Ok(())
    }

    pub async fn verify_removed(&self, id: &TorrentId) -> Result<(), Error> {
        confirm_saved(|| self.verify_removed_once(id)).await
    }

    async fn verify_removed_once(&self, id: &TorrentId) -> Result<(), Error> {
        let saved = self.read(false).await?;
        if saved
            .torrents
            .values()
            .any(|row| row.info_hash == id.as_str())
        {
            return Err(Error::SessionNotSaved);
        }
        Ok(())
    }

    pub async fn verify_added(&self, handle: &ManagedTorrent, id: &TorrentId) -> Result<(), Error> {
        confirm_saved(|| async {
            self.verify_present_once(handle).await?;
            self.verify_metainfo(id).await
        })
        .await
    }
}

// The pinned native writer uses Tokio File::write_all without flush. Its last
// blocking write can still be pending when the operation returns. Confirm the
// postcondition within a fixed budget; a failed save still returns an error.
async fn confirm_saved<F, Fut>(check: F) -> Result<(), Error>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(), Error>>,
{
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if check().await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| Error::SessionNotSaved)
}

impl SavedSession {
    pub fn verify_restored(&self, session: &Session) -> Result<(), Error> {
        for (id, row) in &self.torrents {
            let handle = session
                .get(TorrentIdOrHash::Id(*id))
                .ok_or(Error::RestoreFailed)?;
            // is_paused reports the requested pause flag even during checking.
            if !row.matches(&handle) {
                return Err(Error::RestoreFailed);
            }
        }
        Ok(())
    }
}

impl SavedTorrent {
    fn matches(&self, handle: &ManagedTorrent) -> bool {
        self.info_hash == handle.info_hash().as_string()
            && self.is_paused == handle.is_paused()
            && self.output_folder == handle.output_folder()
            && self.only_files == handle.only_files()
    }
}

async fn read_bounded(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "saved state exceeds its read limit",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn waits_for_pending_writes_but_rejects_a_save_that_never_finishes() {
        let committed = std::sync::atomic::AtomicBool::new(false);
        let check = || async {
            if committed.load(std::sync::atomic::Ordering::Acquire) {
                Ok(())
            } else {
                Err(Error::SessionNotSaved)
            }
        };
        let write = async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            committed.store(true, std::sync::atomic::Ordering::Release);
        };
        let (result, ()) = tokio::join!(confirm_saved(check), write);
        result.unwrap();
        assert!(matches!(
            confirm_saved(|| async { Err(Error::SessionNotSaved) }).await,
            Err(Error::SessionNotSaved)
        ));
    }

    #[tokio::test]
    async fn bounds_saved_reads_and_rejects_malformed_schema_without_replacing_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session.json");
        let persistence = Persistence::new(root.path().to_owned());
        assert!(persistence.preflight().await.is_ok());
        for invalid in [
            br#"{}"#.as_slice(),
            br#"{"torrents":{}} trailing"#,
            b"broken json",
        ] {
            tokio::fs::write(&path, invalid).await.unwrap();
            assert!(matches!(
                persistence.preflight().await,
                Err(Error::RestoreFailed)
            ));
            assert_eq!(tokio::fs::read(&path).await.unwrap(), invalid);
        }
        tokio::fs::write(&path, br#"{"torrents":{}}"#)
            .await
            .unwrap();
        assert!(persistence.preflight().await.is_ok());
        assert!(read_bounded(&path, 15).await.is_ok());
        assert_eq!(
            read_bounded(&path, 14).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}
