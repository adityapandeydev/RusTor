use crate::{
    Candidate, Category, InfoHash, Provider, ProviderDescriptor, ProviderFailure, ProviderFuture,
    SearchError, SearchQuery,
};
use reqwest::{Client, StatusCode, header};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::{sync::Mutex, time::Instant};

const ENDPOINT: &str = "https://nyaa.si/";
const NAMESPACE: &str = "https://nyaa.si/xmlns/nyaa";
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_NODES: u32 = 20_000;
// One overflow row lets SearchService report its default 100-row truncation.
const MAX_ROWS: usize = 101;
const REQUEST_INTERVAL: Duration = Duration::from_secs(2);
const TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://tracker.openbittorrent.com:6969/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://exodus.desync.com:6969/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.dler.org:6969/announce",
    "http://tracker.opentrackr.org:1337/announce",
    "http://tracker.openbittorrent.com:80/announce",
    "http://tracker.dler.org:6969/announce",
    "https://tracker.tamersunion.org:443/announce",
];

/// Fixed-origin metadata search. Does not fetch item links or start transfers.
pub struct NyaaProvider {
    descriptor: ProviderDescriptor,
    client: Client,
    endpoint: url::Url,
    gate: Mutex<Gate>,
}
struct Gate {
    next: Instant,
    cooldown: Option<Cooldown>,
}
struct Cooldown {
    until: Option<Instant>,
    seconds: u64,
    failure: ProviderFailure,
}
impl NyaaProvider {
    pub fn new() -> Result<Self, SearchError> {
        Ok(Self {
            descriptor: descriptor()?,
            client: client(true)?,
            endpoint: url::Url::parse(ENDPOINT)
                .map_err(|_| SearchError::InvalidConfig("invalid fixed provider endpoint"))?,
            gate: Mutex::new(Gate {
                next: Instant::now(),
                cooldown: None,
            }),
        })
    }
    async fn reserve(&self) -> Result<(), ProviderFailure> {
        loop {
            let mut gate = self.gate.lock().await;
            if let Some(cooldown) = &gate.cooldown {
                let remaining = match cooldown.until {
                    Some(until) if until <= Instant::now() => None,
                    Some(until) => {
                        let d = until.saturating_duration_since(Instant::now());
                        Some(d.as_secs().saturating_add(u64::from(d.subsec_nanos() != 0)))
                    }
                    None => Some(cooldown.seconds),
                };
                if let Some(seconds) = remaining {
                    return Err(match cooldown.failure {
                        ProviderFailure::RateLimited { .. } => ProviderFailure::RateLimited {
                            retry_after_seconds: Some(seconds),
                        },
                        _ => cooldown.failure.clone(),
                    });
                }
            }
            gate.cooldown = None;
            let next = gate.next;
            if next <= Instant::now() {
                gate.next = Instant::now() + REQUEST_INTERVAL;
                return Ok(());
            }
            // Release the lock so an in-flight response can publish a cooldown.
            drop(gate);
            tokio::time::sleep_until(next).await;
        }
    }

    async fn cooldown(&self, seconds: u64, failure: ProviderFailure) {
        self.gate.lock().await.cooldown = Some(Cooldown {
            until: Instant::now().checked_add(Duration::from_secs(seconds)),
            seconds,
            failure,
        });
    }
    async fn fetch(&self, query: &SearchQuery) -> Result<Vec<Candidate>, ProviderFailure> {
        self.reserve().await?;
        let mut url = self.endpoint.clone();
        url.query_pairs_mut()
            .clear()
            .append_pair("page", "rss")
            .append_pair("q", query.as_str())
            .append_pair("c", "0_0")
            .append_pair("f", "0");
        let mut response = self
            .client
            .get(url)
            .header(
                header::ACCEPT,
                "application/rss+xml, application/xml;q=0.9, text/xml;q=0.8",
            )
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| ProviderFailure::Unavailable)?;
        let status = response.status();
        let after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| retry_after(value, SystemTime::now()));
        if status == StatusCode::TOO_MANY_REQUESTS {
            let failure = ProviderFailure::RateLimited {
                retry_after_seconds: after,
            };
            self.cooldown(after.unwrap_or(60), failure.clone()).await;
            return Err(failure);
        }
        if status == StatusCode::SERVICE_UNAVAILABLE
            && let Some(seconds) = after
        {
            self.cooldown(seconds, ProviderFailure::Unavailable).await;
        }
        if status != StatusCode::OK {
            return Err(ProviderFailure::Unavailable);
        }
        let media = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(';').next())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !matches!(
            media.as_str(),
            "application/rss+xml" | "application/xml" | "text/xml"
        ) {
            return Err(ProviderFailure::InvalidResponse);
        }
        if let Some(encoding) = response.headers().get(header::CONTENT_ENCODING)
            && !encoding
                .to_str()
                .map(|s| s.eq_ignore_ascii_case("identity"))
                .unwrap_or(false)
        {
            return Err(ProviderFailure::InvalidResponse);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BODY_BYTES as u64)
        {
            return Err(ProviderFailure::InvalidResponse);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ProviderFailure::Unavailable)?
        {
            if chunk.len() > MAX_BODY_BYTES.saturating_sub(body.len()) {
                return Err(ProviderFailure::InvalidResponse);
            }
            body.extend_from_slice(&chunk);
        }
        tokio::task::spawn_blocking(move || parse_feed(&body))
            .await
            .map_err(|_| ProviderFailure::InvalidResponse)?
    }
}
impl Provider for NyaaProvider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
    fn search<'a>(&'a self, query: &'a SearchQuery) -> ProviderFuture<'a> {
        Box::pin(self.fetch(query))
    }
}
fn descriptor() -> Result<ProviderDescriptor, SearchError> {
    ProviderDescriptor::new("nyaa", "Nyaa", vec![Category::Anime], true)
}
fn client(require_https: bool) -> Result<Client, SearchError> {
    let builder = Client::builder()
        .https_only(require_https)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(4))
        .read_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(12))
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .user_agent(concat!("RusTor/", env!("CARGO_PKG_VERSION")));
    #[cfg(test)]
    let builder = if !require_https {
        builder.no_proxy()
    } else {
        builder
    };
    builder
        .build()
        .map_err(|_| SearchError::InvalidConfig("could not build provider HTTP client"))
}
fn parse_feed(bytes: &[u8]) -> Result<Vec<Candidate>, ProviderFailure> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(ProviderFailure::InvalidResponse);
    }
    let xml = std::str::from_utf8(bytes).map_err(|_| ProviderFailure::InvalidResponse)?;
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
            ..Default::default()
        },
    )
    .map_err(|_| ProviderFailure::InvalidResponse)?;
    let rss = doc.root_element();
    if !rss.has_tag_name("rss") || rss.attribute("version") != Some("2.0") {
        return Err(ProviderFailure::InvalidResponse);
    }
    let channel = rss
        .children()
        .find(|n| n.has_tag_name("channel"))
        .ok_or(ProviderFailure::InvalidResponse)?;
    Ok(channel
        .children()
        .filter(|n| n.has_tag_name("item"))
        .take(MAX_ROWS)
        .map(|item| {
            let raw_hash = field(item, Some(NAMESPACE), "infoHash");
            let info_hash = InfoHash::parse(&raw_hash)
                .map(|hash| hash.as_str().to_owned())
                .unwrap_or(raw_hash);
            let name = field(item, None, "title");
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query
                .append_pair("xt", &format!("urn:btih:{info_hash}"))
                .append_pair("dn", &name);
            for tracker in TRACKERS {
                query.append_pair("tr", tracker);
            }
            let magnet = format!("magnet:?{}", query.finish());
            Candidate {
                info_hash,
                name,
                magnet,
                size_bytes: parse_size(&field(item, Some(NAMESPACE), "size")),
                seeders: count(&field(item, Some(NAMESPACE), "seeders")),
                leechers: count(&field(item, Some(NAMESPACE), "leechers")),
                file_count: None,
                added_unix_ms: chrono::DateTime::parse_from_rfc2822(&field(item, None, "pubDate"))
                    .ok()
                    .and_then(|date| u64::try_from(date.timestamp_millis()).ok()),
            }
        })
        .collect())
}
fn field(node: roxmltree::Node<'_, '_>, namespace: Option<&str>, name: &str) -> String {
    let Some(element) = node.children().find(|n| {
        n.is_element() && n.tag_name().namespace() == namespace && n.tag_name().name() == name
    }) else {
        return String::new();
    };
    let mut value = String::new();
    for text in element
        .children()
        .filter_map(|n| if n.is_text() { n.text() } else { None })
    {
        if text.len() > 2048 - value.len() {
            return String::new();
        }
        value.push_str(text);
    }
    value.trim().to_owned()
}
fn count(value: &str) -> Option<u32> {
    value.trim().parse().ok()
}
fn parse_size(value: &str) -> Option<u64> {
    if value.len() > 64 {
        return None;
    }
    let boundary = value.find(|c: char| c.is_ascii_alphabetic())?;
    let number = value[..boundary].trim();
    let unit = value[boundary..].trim().to_ascii_uppercase();
    let multiplier: u128 = match unit.as_str() {
        "B" => 1,
        "KB" => 1000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "TB" => 1_000_000_000_000,
        "KIB" => 1024,
        "MIB" => 1024_u128.pow(2),
        "GIB" => 1024_u128.pow(3),
        "TIB" => 1024_u128.pow(4),
        _ => return None,
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let scale = 10_u128.pow(fraction.len() as u32);
    let numerator = whole
        .parse::<u128>()
        .ok()?
        .checked_mul(scale)?
        .checked_add(if fraction.is_empty() {
            0
        } else {
            fraction.parse::<u128>().ok()?
        })?;
    u64::try_from(numerator.checked_mul(multiplier)?.checked_add(scale / 2)? / scale).ok()
}
fn retry_after(value: &str, now: SystemTime) -> Option<u64> {
    if value.len() > 128 {
        return None;
    }
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return value.parse().ok();
    }
    let target = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .timestamp();
    Some(
        u64::try_from(target)
            .unwrap_or(0)
            .saturating_sub(now.duration_since(UNIX_EPOCH).ok()?.as_secs()),
    )
}

#[cfg(test)]
#[path = "nyaa_tests.rs"]
mod tests;
