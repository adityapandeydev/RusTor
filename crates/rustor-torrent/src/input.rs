use librqbit::{AddTorrent, Magnet, torrent_from_bytes};
use serde::Serialize;

use crate::Error;

const MAX_METAINFO_BYTES: usize = 16 * 1024 * 1024;
const MAX_MAGNET_BYTES: usize = 64 * 1024;
const MAX_SELECTED_FILES: usize = 10_000;

/// Stable lowercase v1 infohash; independent of the native session's numeric IDs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct TorrentId(String);

impl TorrentId {
    pub fn parse(value: &str) -> Result<Self, Error> {
        let value = value.trim();
        let valid = match value.len() {
            40 => value.bytes().all(|b| b.is_ascii_hexdigit()),
            32 => value
                .bytes()
                .all(|b| matches!(b.to_ascii_uppercase(), b'A'..=b'Z' | b'2'..=b'7')),
            _ => false,
        };
        if !valid {
            return Err(Error::InvalidInput("expected a hex or base32 v1 infohash"));
        }
        let parsed = Magnet::parse(&format!(
            "magnet:?xt=urn:btih:{}",
            value.to_ascii_uppercase()
        ))
        .map_err(|_| Error::InvalidInput("invalid v1 infohash"))?;
        let hash = parsed
            .as_id20()
            .ok_or(Error::InvalidInput("a v1 infohash is required"))?;
        Ok(Self(hash.as_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validated input. Original metainfo bytes retain private flags and announce lists.
pub struct TorrentInput {
    id: TorrentId,
    source: Source,
}

enum Source {
    Magnet(String),
    Metainfo(Vec<u8>),
}

impl TorrentInput {
    /// Accept a magnet or bare hex/base32 hash. Never fetch an arbitrary URL.
    pub fn parse(value: &str) -> Result<Self, Error> {
        let value = value.trim();
        if value.len() > MAX_MAGNET_BYTES {
            return Err(Error::InvalidInput("magnet exceeds 64 KiB"));
        }
        if let Ok(id) = TorrentId::parse(value) {
            let magnet = format!("magnet:?xt=urn:btih:{}", id.as_str());
            return Ok(Self {
                id,
                source: Source::Magnet(magnet),
            });
        }
        let mut url = url::Url::parse(value)
            .map_err(|_| Error::InvalidInput("expected a magnet or v1 infohash"))?;
        if url.scheme() != "magnet" {
            return Err(Error::InvalidInput("only magnet links are accepted"));
        }
        let mut id = None;
        let mut selection_count = 0usize;
        for (key, val) in url.query_pairs() {
            if key == "xt"
                && let Some(hash) = val.strip_prefix("urn:btih:")
            {
                let parsed = TorrentId::parse(hash)?;
                if id.as_ref().is_some_and(|old| old != &parsed) {
                    return Err(Error::InvalidInput("conflicting infohashes"));
                }
                id = Some(parsed);
            }
            if key == "so" {
                for selection in val.split(',') {
                    let count = match selection.split_once('-') {
                        Some((start, end)) => {
                            let start = selection_index(start)?;
                            let end = selection_index(end)?;
                            end.checked_sub(start).and_then(|n| n.checked_add(1))
                        }
                        None => {
                            selection_index(selection)?;
                            Some(1)
                        }
                    }
                    .ok_or(Error::InvalidInput("invalid file selection range"))?;
                    selection_count = selection_count
                        .checked_add(count)
                        .filter(|count| *count <= MAX_SELECTED_FILES)
                        .ok_or(Error::InvalidInput("file selection exceeds 10,000 entries"))?;
                }
            }
        }
        let id = id.ok_or(Error::InvalidInput(
            "a v1 infohash is required; v2-only is unsupported",
        ))?;
        // The native parser requires uppercase base32. Normalize only when needed;
        // retain the original hex magnet and every tracker/extension parameter.
        let needs_normalization = url.query_pairs().any(|(key, val)| {
            key == "xt" && val.strip_prefix("urn:btih:").is_some_and(|h| h.len() == 32)
        });
        let magnet = if needs_normalization {
            let pairs: Vec<_> = url
                .query_pairs()
                .map(|(key, val)| {
                    let val = if key == "xt" && val.starts_with("urn:btih:") {
                        format!("urn:btih:{}", id.as_str())
                    } else {
                        val.into_owned()
                    };
                    (key.into_owned(), val)
                })
                .collect();
            url.query_pairs_mut().clear().extend_pairs(pairs);
            url.to_string()
        } else {
            value.to_owned()
        };
        Ok(Self {
            id,
            source: Source::Magnet(magnet),
        })
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > MAX_METAINFO_BYTES {
            return Err(Error::InvalidInput(
                "metainfo must be between 1 byte and 16 MiB",
            ));
        }
        let parsed = torrent_from_bytes(&bytes)
            .map_err(|_| Error::InvalidInput("invalid .torrent metainfo"))?;
        parsed
            .info
            .data
            .clone()
            .validate()
            .map_err(|_| Error::InvalidInput("invalid torrent file layout"))?;
        let id = TorrentId(parsed.info_hash.as_string());
        Ok(Self {
            id,
            source: Source::Metainfo(bytes),
        })
    }

    pub fn id(&self) -> &TorrentId {
        &self.id
    }

    pub fn magnet(&self) -> Option<&str> {
        match &self.source {
            Source::Magnet(value) => Some(value),
            Source::Metainfo(_) => None,
        }
    }

    pub(crate) fn into_native(self) -> AddTorrent<'static> {
        match self.source {
            Source::Magnet(value) => AddTorrent::from_url(value),
            Source::Metainfo(bytes) => AddTorrent::from_bytes(bytes),
        }
    }
}

fn selection_index(value: &str) -> Result<usize, Error> {
    value
        .parse::<usize>()
        .ok()
        .filter(|v| *v < MAX_SELECTED_FILES)
        .ok_or(Error::InvalidInput("invalid file selection index"))
}
