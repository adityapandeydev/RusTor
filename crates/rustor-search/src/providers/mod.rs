mod http;
mod magnet;
mod metadata;
mod nyaa;
pub use nyaa::NyaaProvider;
mod json;
pub use json::JsonProvider;
/// Torlink provider entries implemented so far; each owns its own HTTP rate gate.
pub fn desktop_providers() -> Result<Vec<std::sync::Arc<dyn crate::Provider>>, crate::SearchError> {
    Ok(vec![
        std::sync::Arc::new(NyaaProvider::new()?),
        std::sync::Arc::new(JsonProvider::yts()?),
        std::sync::Arc::new(JsonProvider::bittorrented()?),
        std::sync::Arc::new(JsonProvider::subsplease()?),
    ])
}
