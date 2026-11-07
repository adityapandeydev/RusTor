//! Optional provider artwork. Only fixed image origins are fetched; never item links.
use crate::InfoHash;
use image::{ImageFormat, ImageReader, Limits};
use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};

const MAX_DOWNLOAD: usize = 2 * 1024 * 1024;
const MAX_CACHED: usize = 128 * 1024;
const MAX_ENTRIES: usize = 256;

/// Validate before merging results and again at the network boundary.
pub fn artwork_url(provider: &str, value: &str) -> Option<String> {
    if value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
        return None;
    }
    let value = if provider == "subsplease" && value.starts_with("/wp-content/uploads/") {
        format!("https://subsplease.org{value}")
    } else {
        value.to_owned()
    };
    let url = url::Url::parse(&value).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    let allowed = match provider {
        "yts" => {
            matches!(host, "yts.mx" | "yts.am" | "yts.rs")
                && (url.path().starts_with("/assets/images/movies/")
                    || url.path().starts_with("/images/images/"))
        }
        "subsplease" => host == "subsplease.org" && url.path().starts_with("/wp-content/uploads/"),
        "fitgirl" => {
            matches!(
                host,
                "i1.imageban.ru"
                    | "i2.imageban.ru"
                    | "i3.imageban.ru"
                    | "i4.imageban.ru"
                    | "i5.imageban.ru"
                    | "i6.imageban.ru"
                    | "i7.imageban.ru"
                    | "i8.imageban.ru"
                    | "i9.imageban.ru"
            ) && url.path().starts_with("/out/")
        }
        "metadata" => {
            (host == "static.tvmaze.com"
                && url.path().starts_with("/uploads/images/medium_portrait/"))
                || (host == "m.media-amazon.com" && url.path().starts_with("/images/M/"))
        }
        _ => false,
    };
    if !allowed {
        return None;
    }
    // Request a small IMDb CDN rendition; full-resolution posters can exceed decode limits.
    if provider == "metadata" && host == "m.media-amazon.com" {
        let stem = url.path().split("._V1_").next()?;
        return Some(format!(
            "https://m.media-amazon.com{stem}._V1_QL75_UY192_.jpg"
        ));
    }
    Some(url.to_string())
}

/// Shared by desktop and future native clients. Cache contains resized PNGs only.
pub struct ArtworkCache {
    directory: PathBuf,
    client: Option<reqwest::Client>,
    slots: Semaphore,
    keys: [Mutex<()>; 32],
    disk: Arc<std::sync::Mutex<()>>,
    metadata: crate::artwork_lookup::Metadata,
}
impl ArtworkCache {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            client: reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(8))
                .connect_timeout(Duration::from_secs(4))
                .user_agent(concat!("RusTor/", env!("CARGO_PKG_VERSION")))
                .build()
                .ok(),
            slots: Semaphore::new(3),
            keys: std::array::from_fn(|_| Mutex::new(())),
            disk: Arc::new(std::sync::Mutex::new(())),
            metadata: crate::artwork_lookup::Metadata::new(),
        }
    }
    /// Returns a local data URL, never a renderer-accessible remote URL or path.
    /// Missing, invalid, blocked or unavailable artwork is an ordinary absence.
    pub async fn get(&self, hash: &str, source: Option<&str>) -> Option<String> {
        self.get_inner(hash, source, None).await
    }
    pub async fn get_for_result(&self, row: &crate::SearchResult) -> Option<String> {
        self.get_inner(
            row.info_hash.as_str(),
            row.artwork_url.as_deref(),
            Some(row),
        )
        .await
    }
    async fn get_inner(
        &self,
        hash: &str,
        source: Option<&str>,
        row: Option<&crate::SearchResult>,
    ) -> Option<String> {
        let hash = InfoHash::parse(hash).ok()?;
        let key = hash.as_str();
        let bucket = key
            .bytes()
            .fold(0usize, |sum, byte| sum + usize::from(byte))
            % self.keys.len();
        let _key = self.keys[bucket].lock().await;
        let _slot = self.slots.acquire().await.ok()?;
        let path = self.directory.join(format!("{key}.png"));
        let cached_path = path.clone();
        if let Some(bytes) = tokio::task::spawn_blocking(move || read_cached(&cached_path))
            .await
            .ok()
            .flatten()
        {
            return Some(data_url(&bytes));
        }
        let mut bytes = None;
        if let Some(source) = source
            && let Some(url) = ["yts", "subsplease", "fitgirl"]
                .into_iter()
                .find_map(|provider| artwork_url(provider, source))
        {
            bytes = self.download(url).await;
        }
        if bytes.is_none()
            && let Some(row) = row
        {
            let url = self.metadata.poster(self.client.as_ref()?, row).await?;
            bytes = self.download(url).await;
        }
        let bytes = bytes?;
        let disk = self.disk.clone();
        tokio::task::spawn_blocking(move || {
            let png = normalize(&bytes)?;
            // Cache failure must not prevent displaying a successfully decoded cover.
            if let Ok(_guard) = disk.lock() {
                let _ = store(&path, &png);
            }
            Some(data_url(&png))
        })
        .await
        .ok()
        .flatten()
    }
    async fn download(&self, url: String) -> Option<Vec<u8>> {
        let response = self
            .client
            .as_ref()?
            .get(url)
            .header("Accept", "image/jpeg,image/png,image/webp")
            .send()
            .await
            .ok()?;
        read_response(
            response,
            &["image/jpeg", "image/png", "image/webp"],
            MAX_DOWNLOAD,
        )
        .await
    }
}
pub(crate) async fn read_response(
    mut response: reqwest::Response,
    media: &[&str],
    limit: usize,
) -> Option<Vec<u8>> {
    if !response.status().is_success()
        || response.content_length().is_some_and(|n| n > limit as u64)
    {
        return None;
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)?
        .to_str()
        .ok()?
        .split(';')
        .next()?
        .trim();
    if !media.contains(&mime) {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    Some(bytes)
}
fn data_url(bytes: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        data_encoding::BASE64.encode(bytes)
    )
}
fn normalize(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if !matches!(
        reader.format()?,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return None;
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(128, 192);
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).ok()?;
    (png.get_ref().len() <= MAX_CACHED).then(|| png.into_inner())
}
fn read_cached(path: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_CACHED as u64 {
        return None;
    }
    normalize(&std::fs::read(path).ok()?)
}
fn store(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let directory = path.parent().expect("cache file has a parent");
    std::fs::create_dir_all(directory)?;
    let mut entries = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?;
            if path.extension()?.to_str()? != "png"
                || stem.len() != 40
                || InfoHash::parse(stem).is_err()
            {
                return None;
            }
            let meta = entry.metadata().ok()?;
            Some((meta.modified().ok()?, path))
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|(time, _)| *time);
    let remove = entries.len().saturating_sub(MAX_ENTRIES - 1);
    for (_, old) in entries.into_iter().take(remove) {
        std::fs::remove_file(old)?;
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(bytes)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn rejects_redirects_html_and_both_declared_and_streamed_size_overflows() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = ArtworkCache::new(dir.path().to_owned());
        cache.client = Some(
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        );
        for (status, headers, body) in [
            (
                "302 Found",
                "Location: http://127.0.0.1:1/never-follow\r\nContent-Type: image/png",
                vec![],
            ),
            (
                "200 OK",
                "Content-Type: text/html",
                b"<html>blocked</html>".to_vec(),
            ),
            (
                "200 OK",
                "Content-Type: image/png\r\nContent-Length: 2097153",
                vec![],
            ),
            (
                "200 OK",
                "Content-Type: image/png",
                vec![0; MAX_DOWNLOAD + 1],
            ),
        ] {
            let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/cover", server.local_addr().unwrap());
            let task = tokio::spawn(async move {
                let (mut socket, _) = server.accept().await.unwrap();
                let mut request = [0u8; 4096];
                let _ = socket.read(&mut request).await;
                let response =
                    format!("HTTP/1.1 {status}\r\n{headers}\r\nConnection: close\r\n\r\n");
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            });
            assert!(cache.download(url).await.is_none());
            task.await.unwrap();
        }
    }
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(width, height)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }
    #[test]
    fn normalizes_raster_dimensions_and_rejects_non_images_and_huge_dimensions() {
        let out = normalize(&png(400, 600)).unwrap();
        let image = image::load_from_memory(&out).unwrap();
        assert_eq!((image.width(), image.height()), (128, 192));
        assert!(normalize(b"<svg onload='evil'/>").is_none());
        assert!(normalize(&png(2049, 1)).is_none());
    }
    #[tokio::test]
    async fn cache_survives_restart_and_unknown_hashes_never_fetch_arbitrary_urls() {
        let dir = tempfile::tempdir().unwrap();
        let hash = "a".repeat(40);
        store(&dir.path().join(format!("{hash}.png")), &png(32, 48)).unwrap();
        let cache = ArtworkCache::new(dir.path().to_owned());
        assert!(
            cache
                .get(&hash, None)
                .await
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert!(
            cache
                .get("../outside", Some("https://127.0.0.1/a"))
                .await
                .is_none()
        );
        assert!(
            cache
                .get(&"b".repeat(40), Some("https://example.com/a.png"))
                .await
                .is_none()
        );
    }
    #[test]
    fn disk_budget_evicts_old_entries_and_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = png(1, 1);
        for i in 0..MAX_ENTRIES + 2 {
            store(&dir.path().join(format!("{i:040x}.png")), &bytes).unwrap();
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), MAX_ENTRIES);
    }
}
