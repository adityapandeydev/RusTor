use super::service::CommandError;
use serde::{Deserialize, Serialize};
use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
};
use tokio::io::AsyncReadExt;

pub const MAX_RATE_KIB: u32 = rustor_torrent::SpeedLimits::MAX_BPS / 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DownloadSettings {
    pub default_destination: PathBuf,
    pub download_limit_kib: u32,
    pub upload_limit_kib: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSettings {
    version: u8,
    settings: DownloadSettings,
}

impl DownloadSettings {
    pub fn defaults(destination: PathBuf) -> Self {
        Self {
            default_destination: destination,
            download_limit_kib: 0,
            upload_limit_kib: 0,
        }
    }
    pub fn limits(&self) -> Result<rustor_torrent::SpeedLimits, CommandError> {
        fn rate(kib: u32) -> Result<Option<NonZeroU32>, CommandError> {
            if kib != 0 && !(16..=MAX_RATE_KIB).contains(&kib) {
                return Err(CommandError::new(
                    "invalid_settings",
                    "Use 0 for unlimited, or a whole number from 16 to 976,562 KiB/s.",
                ));
            }
            Ok(NonZeroU32::new(kib * 1024))
        }
        Ok(rustor_torrent::SpeedLimits {
            download_bps: rate(self.download_limit_kib)?,
            upload_bps: rate(self.upload_limit_kib)?,
        })
    }
    pub async fn load(path: &Path, defaults: Self) -> Result<Self, CommandError> {
        let file = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(defaults),
            Err(_) => return Err(load_error()),
        };
        let mut bytes = Vec::new();
        file.take(65537)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| load_error())?;
        if bytes.len() > 65536 {
            return Err(load_error());
        }
        let stored: StoredSettings = serde_json::from_slice(&bytes).map_err(|_| load_error())?;
        if stored.version != 1 || !stored.settings.default_destination.is_absolute() {
            return Err(load_error());
        }
        stored.settings.limits().map_err(|_| load_error())?;
        Ok(stored.settings)
    }
    pub async fn save(&self, path: PathBuf) -> Result<(), CommandError> {
        self.limits()?;
        if !self.default_destination.is_absolute() {
            return Err(CommandError::new(
                "invalid_settings",
                "Choose an absolute download folder using the folder picker.",
            ));
        }
        let bytes = serde_json::to_vec_pretty(&StoredSettings {
            version: 1,
            settings: self.clone(),
        })
        .map_err(|_| save_error())?;
        if bytes.len() > 65536 {
            return Err(save_error());
        }
        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let directory = path.parent().ok_or_else(save_error)?;
            let mut temporary =
                tempfile::NamedTempFile::new_in(directory).map_err(|_| save_error())?;
            temporary.write_all(&bytes).map_err(|_| save_error())?;
            temporary.as_file().sync_all().map_err(|_| save_error())?;
            temporary.persist(path).map_err(|_| save_error())?;
            Ok(())
        })
        .await
        .map_err(|_| save_error())?
    }
}
fn load_error() -> CommandError {
    CommandError::new(
        "settings_unavailable",
        "Saved download settings could not be loaded. Close RusTor and repair or remove settings.json in its application-data engine folder. Your torrent files have not been removed.",
    )
}
fn save_error() -> CommandError {
    CommandError::new(
        "settings_not_saved",
        "Settings could not be saved. Check available space and permissions in RusTor's application-data folder, then try again.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn settings_roundtrip_replace_and_failed_save_preserve_committed_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let defaults = DownloadSettings::defaults(root.path().join("downloads"));
        assert_eq!(
            DownloadSettings::load(&path, defaults.clone())
                .await
                .unwrap(),
            defaults
        );
        let mut saved = defaults.clone();
        saved.download_limit_kib = 128;
        saved.save(path.clone()).await.unwrap();
        assert_eq!(
            DownloadSettings::load(&path, defaults.clone())
                .await
                .unwrap(),
            saved
        );
        saved.upload_limit_kib = 64;
        saved.save(path.clone()).await.unwrap();
        let committed = tokio::fs::read(&path).await.unwrap();
        saved.upload_limit_kib = 1;
        assert!(saved.save(path.clone()).await.is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), committed);
        saved.upload_limit_kib = 64;
        assert!(
            saved
                .save(root.path().join("missing/settings.json"))
                .await
                .is_err()
        );
        assert_eq!(tokio::fs::read(&path).await.unwrap(), committed);
    }
    #[tokio::test]
    async fn corrupt_settings_are_reported_and_preserved() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        tokio::fs::write(&path, b"broken json").await.unwrap();
        assert!(
            DownloadSettings::load(
                &path,
                DownloadSettings::defaults(root.path().join("downloads"))
            )
            .await
            .is_err()
        );
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"broken json");
    }
    #[test]
    fn rate_boundaries_are_checked_before_conversion() {
        let mut settings = DownloadSettings::defaults(std::env::temp_dir());
        for value in [0, 16, MAX_RATE_KIB] {
            settings.download_limit_kib = value;
            assert!(settings.limits().is_ok());
        }
        for value in [1, 15, MAX_RATE_KIB + 1, u32::MAX] {
            settings.download_limit_kib = value;
            assert!(settings.limits().is_err());
        }
    }
}
