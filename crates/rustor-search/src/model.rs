use serde::Serialize;
use std::{future::Future, pin::Pin};

pub const MAX_MAGNET_BYTES: usize = 16 * 1024;
pub const MAX_TRACKERS: usize = 64;
pub const MAX_CANDIDATES: usize = 3200;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("invalid search query: {0}")]
    InvalidQuery(&'static str),
    #[error("invalid provider: {0}")]
    InvalidProvider(&'static str),
    #[error("invalid search result: {0}")]
    InvalidCandidate(&'static str),
    #[error("invalid search configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("search must start inside a Tokio runtime")]
    RuntimeUnavailable,
    #[error("search identity space exhausted")]
    IdentityExhausted,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SearchQuery(String);
impl SearchQuery {
    pub fn new(value: &str) -> Result<Self, SearchError> {
        if value.len() > 256 || value.chars().any(|c| c.is_control() && !c.is_whitespace()) {
            return Err(SearchError::InvalidQuery(
                "use at most 256 UTF-8 bytes without control characters",
            ));
        }
        Ok(Self(value.split_whitespace().collect::<Vec<_>>().join(" ")))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct InfoHash(String);
impl InfoHash {
    pub fn parse(value: &str) -> Result<Self, SearchError> {
        if value.len() > 64 {
            return Err(SearchError::InvalidCandidate("infohash is oversized"));
        }
        let value = value.trim();
        if value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(Self(value.to_ascii_lowercase()));
        }
        if value.len() == 32 {
            let bytes = data_encoding::BASE32_NOPAD
                .decode(value.to_ascii_uppercase().as_bytes())
                .map_err(|_| SearchError::InvalidCandidate("invalid v1 infohash"))?;
            if bytes.len() == 20 {
                return Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()));
            }
        }
        Err(SearchError::InvalidCandidate(
            "a hex or base32 v1 infohash is required",
        ))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Games,
    Movies,
    Tv,
    Anime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub label: String,
    pub categories: Vec<Category>,
    pub reports_health: bool,
}
impl ProviderDescriptor {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        mut categories: Vec<Category>,
        reports_health: bool,
    ) -> Result<Self, SearchError> {
        let id = id.into();
        let label = label.into();
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
        {
            return Err(SearchError::InvalidProvider(
                "id must use 1-64 lowercase ASCII letters, digits, underscores or hyphens",
            ));
        }
        if label.trim().is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
            return Err(SearchError::InvalidProvider(
                "label must contain 1-256 bytes without control characters",
            ));
        }
        categories.sort();
        categories.dedup();
        Ok(Self {
            id,
            label,
            categories,
            reports_health,
        })
    }
}

/// Provider data before validation; attribution is supplied by the registry, not this row.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub info_hash: String,
    pub name: String,
    pub magnet: String,
    pub size_bytes: Option<u64>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub file_count: Option<u32>,
    pub added_unix_ms: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SearchResult {
    pub info_hash: InfoHash,
    pub name: String,
    pub magnet: String,
    pub size_bytes: Option<u64>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub file_count: Option<u32>,
    pub added_unix_ms: Option<u64>,
    pub winning_provider: String,
    pub providers: Vec<String>,
    pub has_unknown_health: bool,
    pub trackers_truncated: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Default,
    Seeders,
    Size,
    Source,
    Added,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Ascending,
    #[default]
    Descending,
}
#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub query: SearchQuery,
    pub category: Option<Category>,
    pub hide_dead: bool,
    pub text_filter: SearchQuery,
    pub sort_field: SortField,
    pub sort_direction: SortDirection,
}
impl SearchRequest {
    pub fn new(query: SearchQuery) -> Self {
        Self {
            query,
            category: None,
            hide_dead: false,
            text_filter: SearchQuery::default(),
            sort_field: SortField::default(),
            sort_direction: SortDirection::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ProviderFailure {
    Unavailable,
    InvalidResponse,
    RateLimited { retry_after_seconds: Option<u64> },
    Panicked,
}
pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<Candidate>, ProviderFailure>> + Send + 'a>>;
/// Futures must use nonblocking I/O, bound response decoding, and release resources when dropped.
pub trait Provider: Send + Sync + 'static {
    fn descriptor(&self) -> &ProviderDescriptor;
    fn search<'a>(&'a self, query: &'a SearchQuery) -> ProviderFuture<'a>;
}
