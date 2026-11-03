mod eztv;
mod fitgirl;
mod html;
mod http;
mod json;
mod magnet;
mod metadata;
mod nyaa;
#[cfg(test)]
mod test_support;
mod tpb;
mod x1337;

pub use eztv::EztvProvider;
pub use fitgirl::FitGirlProvider;
pub use json::JsonProvider;
pub use nyaa::NyaaProvider;
pub use tpb::TpbProvider;
pub use x1337::X1337Provider;

/// Torlink provider entries implemented so far; each owns its own HTTP rate gate.
pub fn desktop_providers() -> Result<Vec<std::sync::Arc<dyn crate::Provider>>, crate::SearchError> {
    Ok(vec![
        std::sync::Arc::new(NyaaProvider::new()?),
        std::sync::Arc::new(JsonProvider::yts()?),
        std::sync::Arc::new(JsonProvider::bittorrented()?),
        std::sync::Arc::new(JsonProvider::subsplease()?),
        std::sync::Arc::new(TpbProvider::movies()?),
        std::sync::Arc::new(TpbProvider::tv()?),
        std::sync::Arc::new(FitGirlProvider::new()?),
        std::sync::Arc::new(EztvProvider::new()?),
        std::sync::Arc::new(X1337Provider::movies()?),
        std::sync::Arc::new(X1337Provider::tv()?),
    ])
}
