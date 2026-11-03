use super::{
    http::{Http, MAX_BODY_BYTES, Media},
    metadata::*,
};
use crate::{
    Candidate, Category, Provider, ProviderDescriptor, ProviderFailure, ProviderFuture,
    SearchError, SearchQuery,
};
use serde_json::Value;
use std::time::Duration;

pub struct TpbProvider {
    descriptor: ProviderDescriptor,
    tv: bool,
    http: Http,
    base: url::Url,
}
impl TpbProvider {
    pub fn movies() -> Result<Self, SearchError> {
        Self::new(false)
    }
    pub fn tv() -> Result<Self, SearchError> {
        Self::new(true)
    }
    fn new(tv: bool) -> Result<Self, SearchError> {
        Ok(Self {
            descriptor: ProviderDescriptor::new(
                if tv { "tpb-tv" } else { "tpb-movies" },
                "TPB",
                vec![if tv { Category::Tv } else { Category::Movies }],
                true,
            )?,
            tv,
            http: Http::new()?,
            base: url::Url::parse("https://apibay.org/")
                .map_err(|_| SearchError::InvalidConfig("invalid fixed endpoint"))?,
        })
    }
    fn url(&self, query: &SearchQuery, alternate: bool) -> url::Url {
        let mut url = self.base.clone();
        if query.as_str().is_empty() {
            url.set_path(if self.tv {
                "/precompiled/data_top100_208.json"
            } else {
                "/precompiled/data_top100_207.json"
            });
        } else {
            url.set_path("/q.php");
            url.query_pairs_mut().append_pair("q", query.as_str());
            if alternate {
                url.query_pairs_mut().append_pair("cat", "0");
            }
        }
        url
    }
    async fn fetch(&self, query: &SearchQuery) -> Result<Vec<Candidate>, ProviderFailure> {
        let mut body = self
            .http
            .get(self.url(query, false), Media::Json, Duration::from_secs(8))
            .await?;
        let (bytes, is_sentinel) = tokio::task::spawn_blocking(move || {
            let empty = sentinel(&body)?;
            Ok::<_, ProviderFailure>((body, empty))
        })
        .await
        .map_err(|_| ProviderFailure::InvalidResponse)??;
        body = bytes;
        if !query.as_str().is_empty() && is_sentinel {
            body = self
                .http
                .get(self.url(query, true), Media::Json, Duration::from_secs(8))
                .await?;
        }
        let tv = self.tv;
        let search = !query.as_str().is_empty();
        tokio::task::spawn_blocking(move || parse(&body, tv, search))
            .await
            .map_err(|_| ProviderFailure::InvalidResponse)?
    }
}
impl Provider for TpbProvider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
    fn search<'a>(&'a self, query: &'a SearchQuery) -> ProviderFuture<'a> {
        Box::pin(self.fetch(query))
    }
}
fn envelope(body: &[u8]) -> Result<Vec<Value>, ProviderFailure> {
    if body.len() > MAX_BODY_BYTES {
        return Err(ProviderFailure::InvalidResponse);
    }
    serde_json::from_slice(body).map_err(|_| ProviderFailure::InvalidResponse)
}
fn sentinel(body: &[u8]) -> Result<bool, ProviderFailure> {
    let rows = envelope(body)?;
    Ok(rows.len() == 1 && rows[0].get("id").and_then(Value::as_str) == Some("0"))
}
fn numeric(row: &Value, key: &str) -> Option<u64> {
    row.get(key).and_then(|v| {
        v.as_u64().or_else(|| {
            v.as_str()
                .filter(|s| s.len() <= 20 && !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse().ok())
        })
    })
}
fn parse(body: &[u8], tv: bool, search: bool) -> Result<Vec<Candidate>, ProviderFailure> {
    let rows = envelope(body)?;
    let mut out = vec![];
    for value in &rows {
        if value.get("id").and_then(Value::as_str) == Some("0") {
            continue;
        }
        let hash = text(value, "info_hash", 64);
        if hash.bytes().all(|b| b == b'0') {
            continue;
        }
        if search
            && !if tv {
                matches!(numeric(value, "category"), Some(205 | 208))
            } else {
                matches!(numeric(value, "category"), Some(201 | 202 | 207 | 209))
            }
        {
            continue;
        }
        let mut row = candidate(hash, text(value, "name", 1024));
        row.size_bytes = numeric(value, "size");
        row.seeders = numeric(value, "seeders").and_then(|v| v.try_into().ok());
        row.leechers = numeric(value, "leechers").and_then(|v| v.try_into().ok());
        row.file_count = numeric(value, "num_files")
            .filter(|v| *v > 0)
            .and_then(|v| v.try_into().ok());
        row.added_unix_ms = numeric(value, "added").and_then(|n| n.checked_mul(1000));
        out.push(row);
        if out.len() == 101 {
            break;
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn filters_categories_sentinels_and_preserves_unknown_numeric_values() {
        let row = json!({"id":"1","info_hash":"0123456789abcdef0123456789abcdef01234567","name":"Authorized fixture","category":"207","size":"4096","seeders":"0","leechers":"2","num_files":"3","added":"1704067200"});
        let body = serde_json::to_vec(&vec![row]).unwrap();
        let result = parse(&body, false, true).unwrap();
        assert_eq!(result.len(), 1);
        assert!(parse(&body, true, true).unwrap().is_empty());
        assert_eq!(result[0].seeders, Some(0));
        assert_eq!(result[0].leechers, Some(2));
        assert_eq!(result[0].added_unix_ms, Some(1_704_067_200_000));
        assert_eq!(numeric(&json!({"size":"-1"}), "size"), None);
        assert!(sentinel(br#"[{"id":"0"}]"#).unwrap());
        assert!(parse(br#"[{"id":"0"}]"#, false, true).unwrap().is_empty());
        assert!(parse(b"{}", false, false).is_err());
    }
    #[test]
    fn uses_distinct_browse_categories_and_encodes_query() {
        let movie = TpbProvider::movies().unwrap();
        let tv = TpbProvider::tv().unwrap();
        let empty = SearchQuery::new("").unwrap();
        assert!(movie.url(&empty, false).path().contains("207"));
        assert!(tv.url(&empty, false).path().contains("208"));
        let query = SearchQuery::new("fixture & cat=999").unwrap();
        let url = movie.url(&query, true);
        assert_eq!(url.query_pairs().count(), 2);
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "q" && v == query.as_str())
        );
    }
    #[tokio::test]
    async fn retries_only_the_no_result_sentinel_with_explicit_category() {
        let(base,server)=super::super::test_support::server(2,"application/json",|url|{
            if url.query_pairs().any(|(k,v)|k=="cat"&&v=="0") {(200,serde_json::json!([{"id":"1","category":"208","name":"Fixture TV","info_hash":"0123456789abcdef0123456789abcdef01234567","seeders":"5"}]).to_string())}
            else {(200,r#"[{"id":"0","info_hash":"0000000000000000000000000000000000000000"}]"#.into())}
        }).await;
        let mut provider = TpbProvider::tv().unwrap();
        provider.base = base;
        provider.http = Http::local();
        let rows = provider
            .fetch(&SearchQuery::new("fixture & cat=999").unwrap())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 2);
        assert!(
            seen[0]
                .query_pairs()
                .any(|(k, v)| k == "q" && v == "fixture & cat=999")
        );
    }
}
