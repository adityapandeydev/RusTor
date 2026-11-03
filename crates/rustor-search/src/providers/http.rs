use crate::{ProviderFailure, SearchError};
use reqwest::{Client, StatusCode, header};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::{sync::Mutex, time::Instant};

pub(super) const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const REQUEST_INTERVAL: Duration = Duration::from_secs(2);
#[derive(Clone, Copy)]
pub(super) enum Media {
    Xml,
    Html,
    Json,
    SubsPleaseJson,
}
impl Media {
    fn accept(self) -> &'static str {
        match self {
            Self::Xml => "application/rss+xml, application/xml;q=0.9, text/xml;q=0.8",
            Self::Html => "text/html",
            _ => "application/json",
        }
    }
    fn accepts(self, value: &str) -> bool {
        match self {
            Self::Xml => matches!(
                value,
                "application/rss+xml" | "application/xml" | "text/xml"
            ),
            Self::Html => matches!(value, "text/html" | "application/xhtml+xml"),
            Self::Json => value == "application/json",
            // This fixed-origin API currently labels valid JSON text/html; decoding must still succeed.
            Self::SubsPleaseJson => matches!(value, "application/json" | "text/html"),
        }
    }
}
pub(super) struct Http {
    pub(super) client: Client,
    pub(super) gate: Mutex<Gate>,
    interval: Duration,
}
pub(super) struct Gate {
    next: Instant,
    pub(super) cooldown: Option<Cooldown>,
}
pub(super) struct Cooldown {
    until: Option<Instant>,
    seconds: u64,
    failure: ProviderFailure,
}
impl Http {
    pub(super) fn new() -> Result<Self, SearchError> {
        Ok(Self {
            client: client(true)?,
            interval: REQUEST_INTERVAL,
            gate: Mutex::new(Gate {
                next: Instant::now(),
                cooldown: None,
            }),
        })
    }
    #[cfg(test)]
    pub(super) fn local() -> Self {
        let mut http = Self::new().unwrap();
        http.client = client(false).unwrap();
        http.interval = Duration::ZERO;
        http
    }
    pub(super) async fn reserve(&self) -> Result<(), ProviderFailure> {
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
                gate.next = Instant::now() + self.interval;
                return Ok(());
            }
            // Release the lock so an in-flight response can publish a cooldown.
            drop(gate);
            tokio::time::sleep_until(next).await;
        }
    }

    pub(super) async fn cooldown(&self, seconds: u64, failure: ProviderFailure) {
        self.gate.lock().await.cooldown = Some(Cooldown {
            until: Instant::now().checked_add(Duration::from_secs(seconds)),
            seconds,
            failure,
        });
    }
    pub(super) async fn get(
        &self,
        url: url::Url,
        kind: Media,
        timeout: Duration,
    ) -> Result<Vec<u8>, ProviderFailure> {
        self.reserve().await?;
        let mut response = self
            .client
            .get(url)
            .timeout(timeout)
            .header(header::ACCEPT, kind.accept())
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
        if !kind.accepts(&media) {
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
        Ok(body)
    }
}
pub(super) fn client(require_https: bool) -> Result<Client, SearchError> {
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
pub(super) fn retry_after(value: &str, now: SystemTime) -> Option<u64> {
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
