use super::{http::MAX_BODY_BYTES, metadata::magnet_hash};
use crate::{MAX_MAGNET_BYTES, ProviderFailure};
use scraper::{Html, Selector};

pub(super) fn document(text: &str) -> Result<Html, ProviderFailure> {
    if text.len() > MAX_BODY_BYTES
        || text.bytes().filter(|b| *b == b'<').take(20_001).count() > 20_000
    {
        return Err(ProviderFailure::InvalidResponse);
    }
    Ok(Html::parse_document(text))
}
pub(super) fn selector(value: &str) -> Result<Selector, ProviderFailure> {
    Selector::parse(value).map_err(|_| ProviderFailure::InvalidResponse)
}
pub(super) fn first_magnet(html: &str) -> Result<Option<String>, ProviderFailure> {
    let doc = document(html)?;
    let anchors = selector("a[href]")?;
    Ok(doc
        .select(&anchors)
        .filter_map(|a| a.value().attr("href"))
        .find(|href| href.len() <= MAX_MAGNET_BYTES && magnet_hash(href).is_some())
        .map(str::to_owned))
}
pub(super) fn bounded_text<'a>(parts: impl Iterator<Item = &'a str>, max: usize) -> String {
    let mut text = String::new();
    for part in parts {
        if part.len() > max.saturating_sub(text.len()) {
            return String::new();
        }
        text.push_str(part);
    }
    text.trim().to_owned()
}

/// Ignore decorative/third-party images; retain only a provider-approved cover.
pub(super) fn first_artwork(html: &str, provider: &str) -> Result<Option<String>, ProviderFailure> {
    let doc = document(html)?;
    let images = selector("img[src]")?;
    Ok(doc
        .select(&images)
        .find_map(|image| crate::artwork::artwork_url(provider, image.value().attr("src")?)))
}
