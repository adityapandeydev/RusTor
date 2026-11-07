//! Torrent search orchestration and bounded provider adapters; no desktop UI or torrent session.
pub mod artwork;
mod artwork_lookup;
mod cache;
mod model;
mod results;
pub use model::*;
pub use results::ResultIndex;
mod service;
pub use service::*;
pub mod providers;
