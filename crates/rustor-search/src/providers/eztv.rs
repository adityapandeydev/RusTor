use super::{
    http::{Http, MAX_BODY_BYTES, Media},
    metadata::*,
};
use crate::{
    Candidate, Category, Provider, ProviderDescriptor, ProviderFailure, ProviderFuture,
    SearchError, SearchQuery,
};
use serde_json::Value;
use std::{collections::BTreeSet, time::Duration};
use tokio::{sync::Mutex, time::Instant};
#[derive(Clone)]
struct Row {
    candidate: Candidate,
    imdb_id: String,
    haystack: String,
}
struct Index {
    at: Instant,
    rows: Vec<Row>,
}
pub struct EztvProvider {
    descriptor: ProviderDescriptor,
    http: Http,
    endpoint: url::Url,
    index: Mutex<Option<Index>>,
}
impl EztvProvider {
    pub fn new() -> Result<Self, SearchError> {
        Ok(Self {
            descriptor: ProviderDescriptor::new("eztv", "EZTV", vec![Category::Tv], true)?,
            http: Http::new()?,
            endpoint: url::Url::parse("https://eztvx.to/api/get-torrents")
                .map_err(|_| SearchError::InvalidConfig("invalid fixed endpoint"))?,
            index: Mutex::new(None),
        })
    }
    async fn page(&self, page: usize, imdb: Option<&str>) -> Result<Vec<Row>, ProviderFailure> {
        let mut url = self.endpoint.clone();
        url.query_pairs_mut()
            .append_pair("limit", "100")
            .append_pair("page", &page.to_string());
        if let Some(id) = imdb {
            url.query_pairs_mut().append_pair("imdb_id", id);
        }
        let bytes = self
            .http
            .get(url, Media::Json, Duration::from_secs(6))
            .await?;
        tokio::task::spawn_blocking(move || parse(&bytes))
            .await
            .map_err(|_| ProviderFailure::InvalidResponse)?
    }
    async fn recent(&self) -> Result<Vec<Row>, ProviderFailure> {
        let mut index = self.index.lock().await;
        if let Some(cache) = index.as_ref()
            && cache.at.elapsed() < Duration::from_secs(300)
        {
            return Ok(cache.rows.clone());
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut rows = self.page(1, None).await?;
        for page in 2..=10 {
            match tokio::time::timeout_at(deadline, self.page(page, None)).await {
                Ok(Ok(next)) => rows.extend(next),
                Ok(Err(ProviderFailure::RateLimited { .. })) | Err(_) => break,
                Ok(Err(_)) => {}
            }
        }
        *index = Some(Index {
            at: Instant::now(),
            rows: rows.clone(),
        });
        Ok(rows)
    }
    async fn fetch(&self, query: &SearchQuery) -> Result<Vec<Candidate>, ProviderFailure> {
        if query.as_str().is_empty() {
            return Ok(results(self.page(1, None).await?));
        }
        let tokens: Vec<String> = query
            .as_str()
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let recent = self.recent().await?;
        let mut hits: Vec<Row> = recent
            .into_iter()
            .filter(|row| matches(row, &tokens))
            .collect();
        let ids = show_ids(&hits);
        for id in ids {
            if let Ok(rows) = self.page(1, Some(&id)).await {
                hits.extend(rows.into_iter().filter(|row| matches(row, &tokens)));
            }
        }
        Ok(results(hits))
    }
}
impl Provider for EztvProvider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
    fn search<'a>(&'a self, query: &'a SearchQuery) -> ProviderFuture<'a> {
        Box::pin(self.fetch(query))
    }
}
fn parse(bytes: &[u8]) -> Result<Vec<Row>, ProviderFailure> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(ProviderFailure::InvalidResponse);
    }
    let root: Value =
        serde_json::from_slice(bytes).map_err(|_| ProviderFailure::InvalidResponse)?;
    let values = root
        .get("torrents")
        .and_then(Value::as_array)
        .ok_or(ProviderFailure::InvalidResponse)?;
    Ok(values
        .iter()
        .take(100)
        .map(|value| {
            let title = text(value, "title", 1024);
            let filename = text(value, "filename", 1024);
            let hash = text(value, "hash", 64);
            let name = if title.is_empty() {
                filename.clone()
            } else {
                title.clone()
            };
            let mut row = candidate(hash, name);
            let magnet = text(value, "magnet_url", crate::MAX_MAGNET_BYTES);
            if !magnet.is_empty() {
                row.magnet = native_magnet(magnet);
            }
            row.size_bytes = integer(value, "size_bytes").or_else(|| {
                value
                    .get("size_bytes")
                    .and_then(Value::as_str)
                    .filter(|s| s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()))
                    .and_then(|s| s.parse().ok())
            });
            row.seeders = count(value, "seeds");
            row.leechers = count(value, "peers");
            row.added_unix_ms =
                integer(value, "date_released_unix").and_then(|n| n.checked_mul(1000));
            let id = text(value, "imdb_id", 16);
            let id = if !id.is_empty() && id != "0" && id.bytes().all(|b| b.is_ascii_digit()) {
                id
            } else {
                String::new()
            };
            Row {
                candidate: row,
                imdb_id: id,
                haystack: format!("{title} {filename}").to_lowercase(),
            }
        })
        .collect())
}
fn matches(row: &Row, tokens: &[String]) -> bool {
    tokens.iter().all(|token| row.haystack.contains(token))
}
fn show_ids(rows: &[Row]) -> Vec<String> {
    let mut ids = vec![];
    for row in rows {
        if !row.imdb_id.is_empty() && !ids.contains(&row.imdb_id) {
            ids.push(row.imdb_id.clone());
            if ids.len() == 2 {
                break;
            }
        }
    }
    ids
}
fn results(rows: Vec<Row>) -> Vec<Candidate> {
    let mut seen = BTreeSet::new();
    let mut results: Vec<Candidate> = rows
        .into_iter()
        .map(|row| row.candidate)
        .filter(|row| seen.insert(row.info_hash.clone()))
        .collect();
    results.sort_by_key(|row| std::cmp::Reverse(row.seeders));
    results.truncate(101);
    results
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn matches_all_tokens_collects_two_show_ids_and_deduplicates_catalogues() {
        let values = json!({"torrents":[{"title":"Fixture Show S01E01","filename":"fixture.show.1080p","hash":"0123456789abcdef0123456789abcdef01234567","imdb_id":"123","size_bytes":"2048","seeds":12,"peers":3,"date_released_unix":1704067200},{"title":"Different","hash":"1111111111111111111111111111111111111111","imdb_id":"456"},{"title":"Fixture Show S01E02","hash":"2222222222222222222222222222222222222222","imdb_id":"789"}]});
        let rows = parse(&serde_json::to_vec(&values).unwrap()).unwrap();
        assert!(matches(&rows[0], &["fixture".into(), "1080p".into()]));
        assert!(!matches(&rows[1], &["fixture".into()]));
        assert_eq!(show_ids(&rows), vec!["123", "456"]);
        assert_eq!(rows[0].candidate.size_bytes, Some(2048));
        assert_eq!(rows[0].candidate.added_unix_ms, Some(1_704_067_200_000));
        let mut duplicated = rows.clone();
        duplicated.extend(rows);
        assert_eq!(results(duplicated).len(), 3);
        assert!(parse(b"{\"error\":\"blocked\"}").is_err());
    }
    #[tokio::test]
    async fn recent_index_cache_avoids_network_until_expiry() {
        let provider = EztvProvider::new().unwrap();
        *provider.index.lock().await = Some(Index {
            at: Instant::now(),
            rows: vec![],
        });
        assert!(provider.recent().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn indexes_ten_pages_expands_matched_shows_and_reuses_cached_index() {
        let(base,server)=super::super::test_support::server(12,"application/json",|url|{
            let imdb=url.query_pairs().any(|(k,v)|k=="imdb_id"&&v=="123");let page=url.query_pairs().find(|(k,_)|k=="page").unwrap().1.parse::<usize>().unwrap();
            let rows=if imdb{vec![serde_json::json!({"title":"Fixture Show older","hash":"2222222222222222222222222222222222222222","imdb_id":"123","seeds":15})]}else if page==4{vec![serde_json::json!({"title":"Fixture Show current","hash":"1111111111111111111111111111111111111111","imdb_id":"123","seeds":5})]}else{vec![]};
            (200,serde_json::json!({"torrents":rows}).to_string())
        }).await;
        let mut provider = EztvProvider::new().unwrap();
        provider.endpoint = base;
        provider.http = Http::local();
        let query = SearchQuery::new("fixture show").unwrap();
        let rows = provider.fetch(&query).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seeders, Some(15));
        assert_eq!(provider.fetch(&query).await.unwrap().len(), 2);
        let seen = server.await.unwrap();
        assert_eq!(
            seen.iter()
                .filter(|url| url.query_pairs().any(|(k, _)| k == "imdb_id"))
                .count(),
            2
        );
    }
}
