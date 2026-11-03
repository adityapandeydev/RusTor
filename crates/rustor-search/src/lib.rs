//! Provider-independent search; no HTTP client, desktop UI, or torrent session.
mod model;
mod results;
pub use model::*;
pub use results::ResultIndex;
mod service;
pub use service::*;
