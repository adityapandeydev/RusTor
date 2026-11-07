//! Best-effort, exact-title artwork matching. Never resolves torrent links.
use crate::{SearchResult, artwork::read_response};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};
use tokio::{sync::Mutex, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Movie,
    Tv,
    Anime,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Query {
    kind: Kind,
    title: String,
    year: Option<u64>,
}
struct Cached {
    at: Instant,
    poster: Option<String>,
}
struct State {
    entries: BTreeMap<Query, Cached>,
    next: Instant,
}
pub(crate) struct Metadata(Mutex<State>);
impl Metadata {
    pub fn new() -> Self {
        Self(Mutex::new(State {
            entries: BTreeMap::new(),
            next: Instant::now(),
        }))
    }
    pub async fn poster(&self, client: &reqwest::Client, row: &SearchResult) -> Option<String> {
        let query = query(row)?;
        // Coalesce episodes/qualities of the same title and keep at most one lookup per second.
        let mut state = self.0.lock().await;
        state.entries.retain(|_, entry| {
            entry.at.elapsed()
                < Duration::from_secs(if entry.poster.is_some() { 3600 } else { 300 })
        });
        if let Some(entry) = state.entries.get(&query) {
            return entry.poster.clone();
        }
        // A rate-limit cooldown must not leave queued thumbnail commands sleeping for minutes.
        if state.next.saturating_duration_since(Instant::now()) > Duration::from_secs(1) {
            return None;
        }
        tokio::time::sleep_until(state.next).await;
        state.next = Instant::now() + Duration::from_secs(1);
        let url = endpoint(&query)?;
        let response = client
            .get(url)
            .header("Accept", "application/json")
            .send()
            .await
            .ok();
        if response
            .as_ref()
            .is_some_and(|response| response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS)
        {
            state.next = Instant::now() + Duration::from_secs(60);
            return None;
        }
        let bytes = match response {
            Some(response) => read_response(response, &["application/json"], 512 * 1024).await,
            None => None,
        };
        let poster = bytes
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .and_then(|root| select(&query, &root));
        if state.entries.len() >= 256 {
            let oldest = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.at)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                state.entries.remove(&key);
            }
        }
        state.entries.insert(
            query,
            Cached {
                at: Instant::now(),
                poster: poster.clone(),
            },
        );
        poster
    }
}
fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn query(row: &SearchResult) -> Option<Query> {
    let has = |ids: &[&str]| row.providers.iter().any(|id| ids.contains(&id.as_str()));
    let kind = if has(&["nyaa", "subsplease"]) {
        Kind::Anime
    } else if has(&["eztv", "tpb-tv", "x1337-tv"]) {
        Kind::Tv
    } else if has(&["yts", "tpb-movies", "x1337-movies", "bittorrented"]) {
        Kind::Movie
    } else {
        return None;
    };
    clean(&row.name, kind)
}
fn clean(value: &str, kind: Kind) -> Option<Query> {
    if value.len() > 1024 || value.contains("://") || value.contains('\\') {
        return None;
    }
    let mut name = value.trim();
    // Release groups are not part of the title and are not sent to metadata services.
    for _ in 0..8 {
        if !name.starts_with('[') {
            break;
        }
        name = name.split_once(']')?.1.trim_start();
    }
    let name = name.split(" [").next()?.replace(['.', '_'], " ");
    let words: Vec<_> = name.split_whitespace().collect();
    let mut end = words.len();
    let mut year = None;
    for (i, word) in words.iter().enumerate() {
        let token = word
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if i > 0
            && token.len() == 4
            && let Ok(value @ 1900..=2100) = token.parse::<u64>()
        {
            // Last year before release tags handles titles such as Blade Runner 2049 (2017).
            end = i;
            year = Some(value);
            continue;
        }
        let season = token.starts_with('s')
            && token.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
            && token[1..].bytes().all(|c| c.is_ascii_digit() || c == b'e');
        let episode = kind != Kind::Movie
            && i > 0
            && !token.is_empty()
            && token.bytes().all(|c| c.is_ascii_digit());
        let quality = matches!(
            token.as_str(),
            "480p"
                | "576p"
                | "720p"
                | "1080p"
                | "1080i"
                | "2160p"
                | "4k"
                | "web"
                | "webrip"
                | "webdl"
                | "hdtv"
                | "bluray"
                | "brrip"
                | "bdrip"
                | "dvdrip"
                | "remux"
                | "x264"
                | "x265"
                | "h264"
                | "h265"
                | "hevc"
                | "season"
                | "episode"
                | "complete"
                | "batch"
        );
        if season || episode || quality {
            end = end.min(i);
            break;
        }
    }
    let title = words[..end]
        .join(" ")
        .trim_end_matches([' ', '-'])
        .to_owned();
    if title.chars().count() < 2 || title.len() > 160 || normalized(&title).is_empty() {
        return None;
    }
    Some(Query { kind, title, year })
}
fn endpoint(query: &Query) -> Option<url::Url> {
    if query.kind == Kind::Movie {
        let mut url = url::Url::parse("https://v3.sg.media-imdb.com/suggestion/").ok()?;
        let initial = query
            .title
            .chars()
            .find(char::is_ascii_alphanumeric)?
            .to_ascii_lowercase();
        url.path_segments_mut()
            .ok()?
            .pop_if_empty()
            .push(&initial.to_string())
            .push(&format!("{}.json", query.title));
        Some(url)
    } else {
        let mut url = url::Url::parse("https://api.tvmaze.com/search/shows").ok()?;
        url.query_pairs_mut().append_pair("q", &query.title);
        Some(url)
    }
}
fn select(query: &Query, root: &Value) -> Option<String> {
    let rows = if query.kind == Kind::Movie {
        root.get("d")?.as_array()?
    } else {
        root.as_array()?
    };
    let mut matches = BTreeMap::new();
    for entry in rows.iter().take(50) {
        let row = if query.kind == Kind::Movie {
            entry
        } else {
            entry.get("show")?
        };
        let name = row
            .get(if query.kind == Kind::Movie {
                "l"
            } else {
                "name"
            })
            .and_then(Value::as_str)
            .unwrap_or("");
        if normalized(name) != normalized(&query.title) {
            continue;
        }
        if query.kind == Kind::Movie && row.get("qid").and_then(Value::as_str) != Some("movie") {
            continue;
        }
        if query.kind == Kind::Anime && row.get("type").and_then(Value::as_str) != Some("Animation")
        {
            continue;
        }
        let year = if query.kind == Kind::Movie {
            row.get("y").and_then(Value::as_u64)
        } else {
            row.get("premiered")
                .and_then(Value::as_str)
                .and_then(|date| date.get(..4))
                .and_then(|year| year.parse().ok())
        };
        if query.year.is_some() && year != query.year {
            continue;
        }
        let (id, image) = if query.kind == Kind::Movie {
            (
                row.get("id")?.as_str()?.to_owned(),
                row.get("i")?.get("imageUrl")?.as_str()?,
            )
        } else {
            (
                row.get("id")?.as_u64()?.to_string(),
                row.get("image")?.get("medium")?.as_str()?,
            )
        };
        if let Some(image) = crate::artwork::artwork_url("metadata", image) {
            matches.insert(id, image);
        }
    }
    // Never pick the first result when remakes/identical names remain ambiguous.
    (matches.len() == 1)
        .then(|| matches.into_values().next())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn strips_release_tags_without_sending_paths_or_urls() {
        let movie = clean("The.Dark.Knight.2008.1080p.BluRay.x264", Kind::Movie).unwrap();
        assert_eq!(
            (movie.title.as_str(), movie.year),
            ("The Dark Knight", Some(2008))
        );
        assert_eq!(
            clean("[Group] One Piece - 1100 [1080p] [ABCDEF]", Kind::Anime)
                .unwrap()
                .title,
            "One Piece"
        );
        assert_eq!(
            clean("Breaking.Bad.S02E03.720p", Kind::Tv).unwrap().title,
            "Breaking Bad"
        );
        assert!(clean("C:\\private\\title.mkv", Kind::Tv).is_none());
        assert!(clean("https://example.org/title", Kind::Tv).is_none());
    }
    #[test]
    fn requires_exact_type_title_and_year_and_rejects_ambiguous_remakes() {
        let query = clean("Fixture.2008.1080p", Kind::Movie).unwrap();
        let image = "https://m.media-amazon.com/images/M/fixture._V1_.jpg";
        let root = json!({"d":[{"id":"tt1","l":"Fixture","qid":"movie","y":2008,"i":{"imageUrl":image}}, {"id":"tt2","l":"Fixture","qid":"movie","y":2020,"i":{"imageUrl":image}}]});
        assert!(select(&query, &root).is_some());
        assert!(select(&clean("Fixture", Kind::Movie).unwrap(), &root).is_none());
        assert!(select(&clean("Fixture Extra", Kind::Movie).unwrap(), &root).is_none());
    }
    #[test]
    fn anime_requires_animation_and_bad_image_hosts_are_ignored() {
        let query = clean("[Group] One Piece - 1100 [1080p]", Kind::Anime).unwrap();
        let root = json!([{"show":{"id":1,"name":"One Piece","type":"Animation","image":{"medium":"https://static.tvmaze.com/uploads/images/medium_portrait/1/1.jpg"}}}, {"show":{"id":2,"name":"One Piece","type":"Scripted","image":{"medium":"https://evil.example/image.jpg"}}}]);
        assert!(select(&query, &root).is_some());
        assert!(select(&query, &json!([{"show":{"id":1,"name":"One Piece","type":"Animation","image":{"medium":"https://127.0.0.1/image.jpg"}}}])).is_none());
    }
}
