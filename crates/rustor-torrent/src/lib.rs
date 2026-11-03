//! Native torrent lifecycle for RusTor. No desktop or terminal UI dependencies.

mod input;
mod retry_storage;
mod session;

pub use input::{TorrentId, TorrentInput};
pub use session::{
    AddOptions, AddOutcome, Engine, EngineConfig, PreparedTorrent, TorrentFile, TorrentSnapshot,
    TorrentState,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid torrent input: {0}")]
    InvalidInput(&'static str),
    #[error("Invalid engine configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("Torrent is not managed by this session")]
    NotFound,
    #[error("Torrent engine has stopped")]
    Stopped,
    #[error("Torrent state directory is already in use")]
    StateInUse,
    #[error("Torrent metadata resolution timed out")]
    MetadataTimeout,
    #[error("{operation} failed: {message}")]
    Backend {
        operation: &'static str,
        message: String,
    },
}
