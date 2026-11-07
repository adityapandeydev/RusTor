use crate::*;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

struct Row {
    candidate: Candidate,
    provider: String,
    trackers: BTreeSet<String>,
}
#[derive(Default)]
pub struct ResultIndex {
    rows: BTreeMap<InfoHash, Vec<Row>>,
    candidates: usize,
}
impl ResultIndex {
    pub fn insert(
        &mut self,
        source: &ProviderDescriptor,
        mut candidate: Candidate,
    ) -> Result<(), SearchError> {
        if self.candidates >= MAX_CANDIDATES {
            return Err(SearchError::InvalidCandidate("candidate budget exceeded"));
        }
        let hash = InfoHash::parse(&candidate.info_hash)?;
        candidate.info_hash = hash.as_str().to_owned();
        if candidate.name.len() > 1024
            || candidate
                .name
                .chars()
                .any(|c| c.is_control() && !c.is_whitespace())
        {
            return Err(SearchError::InvalidCandidate("invalid result name"));
        }
        candidate.name = candidate
            .name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if candidate.name.is_empty() {
            return Err(SearchError::InvalidCandidate("missing result name"));
        }
        candidate.magnet = candidate.magnet.trim().to_owned();
        if candidate.magnet.len() > MAX_MAGNET_BYTES || !candidate.magnet.starts_with("magnet:?") {
            return Err(SearchError::InvalidCandidate("invalid or oversized magnet"));
        }
        let url = url::Url::parse(&candidate.magnet)
            .map_err(|_| SearchError::InvalidCandidate("invalid magnet"))?;
        if url.fragment().is_some() {
            return Err(SearchError::InvalidCandidate(
                "magnet fragments are unsupported",
            ));
        }
        let mut found = false;
        let mut trackers = BTreeSet::new();
        for (key, value) in url.query_pairs() {
            if key == "xt"
                && let Some(raw) = value.strip_prefix("urn:btih:")
            {
                if InfoHash::parse(raw)? != hash {
                    return Err(SearchError::InvalidCandidate(
                        "magnet and result identities differ",
                    ));
                }
                found = true;
            }
            if key == "tr" {
                if value.len() > 2048 {
                    return Err(SearchError::InvalidCandidate("tracker URL is oversized"));
                }
                let tracker = url::Url::parse(value.trim())
                    .map_err(|_| SearchError::InvalidCandidate("invalid tracker URL"))?;
                if !matches!(tracker.scheme(), "http" | "https" | "udp")
                    || tracker.host_str().is_none()
                {
                    return Err(SearchError::InvalidCandidate("unsupported tracker URL"));
                }
                trackers.insert(tracker.to_string());
                if trackers.len() > MAX_TRACKERS {
                    return Err(SearchError::InvalidCandidate("too many tracker URLs"));
                }
            }
        }
        if !found {
            return Err(SearchError::InvalidCandidate(
                "magnet requires a matching v1 infohash",
            ));
        }
        if !source.reports_health {
            candidate.seeders = None;
            candidate.leechers = None;
        }
        candidate.artwork_url = candidate
            .artwork_url
            .as_deref()
            .and_then(|value| crate::artwork::artwork_url(&source.id, value));
        candidate.file_count = candidate.file_count.filter(|count| *count > 0);
        self.rows.entry(hash).or_default().push(Row {
            candidate,
            provider: source.id.clone(),
            trackers,
        });
        self.candidates += 1;
        Ok(())
    }

    pub fn results(&self, request: &SearchRequest) -> Vec<SearchResult> {
        let mut rows = self
            .rows
            .iter()
            .map(|(hash, rows)| merge(hash, rows))
            .filter(|row| !request.hide_dead || row.seeders != Some(0) || row.has_unknown_health)
            .filter_map(|row| {
                relevance(&row.name, request.text_filter.as_str()).map(|score| (score, row))
            })
            .collect::<Vec<_>>();
        rows.sort_by(|(sa, a), (sb, b)| {
            if request.sort_field == SortField::Default {
                sb.cmp(sa).then_with(|| compare(a, b, request))
            } else {
                compare(a, b, request)
            }
        });
        rows.into_iter().map(|(_, row)| row).collect()
    }
}
fn rank(a: &Row, b: &Row) -> Ordering {
    b.candidate
        .seeders
        .unwrap_or(0)
        .cmp(&a.candidate.seeders.unwrap_or(0))
        .then_with(|| {
            b.candidate
                .added_unix_ms
                .unwrap_or(0)
                .cmp(&a.candidate.added_unix_ms.unwrap_or(0))
        })
        .then_with(|| a.provider.cmp(&b.provider))
        .then_with(|| a.candidate.name.cmp(&b.candidate.name))
        .then_with(|| a.candidate.magnet.cmp(&b.candidate.magnet))
        .then_with(|| a.candidate.size_bytes.cmp(&b.candidate.size_bytes))
        .then_with(|| a.candidate.file_count.cmp(&b.candidate.file_count))
        .then_with(|| a.candidate.leechers.cmp(&b.candidate.leechers))
        .then_with(|| a.candidate.seeders.cmp(&b.candidate.seeders))
}
fn merge(hash: &InfoHash, rows: &[Row]) -> SearchResult {
    let mut ranked = rows.iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| rank(a, b));
    let winner = ranked[0];
    let c = &winner.candidate;
    let trackers = rows
        .iter()
        .flat_map(|row| row.trackers.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut magnet = c.magnet.clone();
    let mut count = winner.trackers.len();
    let mut truncated = false;
    for tracker in trackers.difference(&winner.trackers) {
        let extra = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("tr", tracker)
            .finish();
        let separator = if magnet.ends_with('&') { "" } else { "&" };
        if count == MAX_TRACKERS || magnet.len() + separator.len() + extra.len() > MAX_MAGNET_BYTES
        {
            truncated = true;
            continue;
        }
        magnet.push_str(separator);
        magnet.push_str(&extra);
        count += 1;
    }
    SearchResult {
        info_hash: hash.clone(),
        name: c.name.clone(),
        magnet,
        size_bytes: c.size_bytes,
        seeders: c.seeders,
        leechers: c.leechers,
        file_count: c
            .file_count
            .or_else(|| ranked.iter().find_map(|r| r.candidate.file_count)),
        added_unix_ms: c
            .added_unix_ms
            .or_else(|| ranked.iter().find_map(|r| r.candidate.added_unix_ms)),
        artwork_url: ranked.iter().find_map(|r| r.candidate.artwork_url.clone()),
        winning_provider: winner.provider.clone(),
        providers: rows
            .iter()
            .map(|r| r.provider.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        has_unknown_health: rows.iter().any(|r| r.candidate.seeders.is_none()),
        trackers_truncated: truncated,
    }
}
fn optional<T: Ord>(a: Option<T>, b: Option<T>, direction: SortDirection) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            if direction == SortDirection::Ascending {
                a.cmp(&b)
            } else {
                b.cmp(&a)
            }
        }
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}
fn compare(a: &SearchResult, b: &SearchResult, request: &SearchRequest) -> Ordering {
    let direction = request.sort_direction;
    let primary = match request.sort_field {
        SortField::Default => optional(a.seeders, b.seeders, SortDirection::Descending),
        SortField::Seeders => optional(a.seeders, b.seeders, direction),
        SortField::Size => optional(a.size_bytes, b.size_bytes, direction),
        SortField::Added => optional(a.added_unix_ms, b.added_unix_ms, direction),
        SortField::Source => {
            if direction == SortDirection::Ascending {
                a.winning_provider.cmp(&b.winning_provider)
            } else {
                b.winning_provider.cmp(&a.winning_provider)
            }
        }
    };
    primary
        .then_with(|| optional(a.seeders, b.seeders, SortDirection::Descending))
        .then_with(|| optional(a.added_unix_ms, b.added_unix_ms, SortDirection::Descending))
        .then_with(|| a.info_hash.cmp(&b.info_hash))
}
fn relevance(name: &str, filter: &str) -> Option<u8> {
    if filter.is_empty() {
        return Some(0);
    }
    let name = name.to_lowercase();
    let filter = filter.to_lowercase();
    let tokens = filter.split_whitespace().collect::<Vec<_>>();
    if !tokens.iter().all(|token| name.contains(token)) {
        return None;
    }
    if name.contains(&filter) {
        return Some(60);
    }
    let mut offset = 0;
    for token in tokens {
        match name[offset..].find(token) {
            Some(index) => {
                let start = offset + index;
                offset = start + name[start..].chars().next().map_or(0, char::len_utf8);
            }
            None => return Some(10),
        }
    }
    Some(30)
}
