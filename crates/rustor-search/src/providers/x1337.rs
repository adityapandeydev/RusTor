use super::{
    html,
    http::{Http, Media},
    metadata::*,
    nyaa::parse_size,
};
use crate::{
    Candidate, Category, Provider, ProviderDescriptor, ProviderFailure, ProviderFuture,
    SearchError, SearchQuery,
};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
#[derive(Clone)]
struct Row {
    name: String,
    path: String,
    seeders: Option<u32>,
    leechers: Option<u32>,
    size: Option<u64>,
}
pub struct X1337Provider {
    descriptor: ProviderDescriptor,
    tv: bool,
    http: Http,
    bases: Vec<url::Url>,
    working: AtomicUsize,
}
impl X1337Provider {
    pub fn movies() -> Result<Self, SearchError> {
        Self::new(false)
    }
    pub fn tv() -> Result<Self, SearchError> {
        Self::new(true)
    }
    fn new(tv: bool) -> Result<Self, SearchError> {
        Ok(Self {
            descriptor: ProviderDescriptor::new(
                if tv { "x1337-tv" } else { "x1337-movies" },
                "1337x",
                vec![if tv { Category::Tv } else { Category::Movies }],
                true,
            )?,
            tv,
            http: Http::new()?,
            bases: [
                "https://1337x.to/",
                "https://1337x.st/",
                "https://x1337x.ws/",
                "https://1337xx.to/",
            ]
            .into_iter()
            .map(|s| {
                url::Url::parse(s).map_err(|_| SearchError::InvalidConfig("invalid fixed endpoint"))
            })
            .collect::<Result<_, _>>()?,
            working: AtomicUsize::new(0),
        })
    }
    fn url(&self, base: &url::Url, query: &SearchQuery) -> url::Url {
        let mut url = base.clone();
        if query.as_str().is_empty() {
            url.set_path(if self.tv {
                "/popular-tv"
            } else {
                "/popular-movies"
            });
        } else {
            url.path_segments_mut()
                .expect("fixed hierarchical URL")
                .clear()
                .push("category-search")
                .push(query.as_str())
                .push(if self.tv { "TV" } else { "Movies" })
                .push("1")
                .push("");
        }
        url
    }
    async fn fetch(&self, query: &SearchQuery) -> Result<Vec<Candidate>, ProviderFailure> {
        let mut selected = None;
        let mut failure = ProviderFailure::Unavailable;
        for offset in 0..self.bases.len() {
            let index = (self.working.load(Ordering::Relaxed) + offset) % self.bases.len();
            let base = self.bases[index].clone();
            match self
                .http
                .get(self.url(&base, query), Media::Html, Duration::from_secs(3))
                .await
            {
                Ok(bytes) => {
                    let query = query.as_str().to_owned();
                    match tokio::task::spawn_blocking(move || parse_rows(&bytes, &query))
                        .await
                        .map_err(|_| ProviderFailure::InvalidResponse)?
                    {
                        Ok(rows) => {
                            self.working.store(index, Ordering::Relaxed);
                            selected = Some((base, rows));
                            break;
                        }
                        Err(error) => failure = error,
                    }
                }
                Err(error @ ProviderFailure::RateLimited { .. }) => return Err(error),
                Err(error) => failure = error,
            }
        }
        let (base, rows) = selected.ok_or(failure)?;
        let had_rows = !rows.is_empty();
        let mut out = vec![];
        for row in rows {
            let Some(url) = detail_url(&base, &row.path) else {
                continue;
            };
            let bytes = match self
                .http
                .get(url, Media::Html, Duration::from_secs(6))
                .await
            {
                Ok(bytes) => bytes,
                Err(ProviderFailure::RateLimited { .. }) if out.is_empty() => {
                    return Err(ProviderFailure::RateLimited {
                        retry_after_seconds: None,
                    });
                }
                Err(ProviderFailure::RateLimited { .. }) => break,
                Err(_) => continue,
            };
            let detail = tokio::task::spawn_blocking(move || parse_detail(&bytes))
                .await
                .map_err(|_| ProviderFailure::InvalidResponse)?;
            if let Ok(Some((magnet, added))) = detail {
                let mut candidate = candidate(magnet_hash(&magnet).unwrap_or_default(), row.name);
                candidate.magnet = native_magnet(magnet);
                candidate.seeders = row.seeders;
                candidate.leechers = row.leechers;
                candidate.size_bytes = row.size;
                candidate.added_unix_ms = added;
                out.push(candidate);
            }
        }
        if had_rows && out.is_empty() {
            return Err(ProviderFailure::Unavailable);
        }
        Ok(out)
    }
}
impl Provider for X1337Provider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
    fn search<'a>(&'a self, query: &'a SearchQuery) -> ProviderFuture<'a> {
        Box::pin(self.fetch(query))
    }
}
fn detail_url(base: &url::Url, path: &str) -> Option<url::Url> {
    if !path.starts_with("/torrent/") || path.contains('\\') || path.len() > 2048 {
        return None;
    }
    let url = base.join(path).ok()?;
    if url.origin() != base.origin() || url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    let mut segments = url.path_segments()?;
    if segments.next() != Some("torrent") {
        return None;
    }
    let id = segments.next()?;
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(url)
}
fn parse_rows(bytes: &[u8], query: &str) -> Result<Vec<Row>, ProviderFailure> {
    let text = std::str::from_utf8(bytes).map_err(|_| ProviderFailure::InvalidResponse)?;
    let doc = html::document(text)?;
    let table = html::selector(".table-list")?;
    if doc.select(&table).next().is_none() {
        return if text.to_lowercase().contains("no results were returned") {
            Ok(vec![])
        } else {
            Err(ProviderFailure::InvalidResponse)
        };
    }
    let tr = html::selector(".table-list tr")?;
    let anchor = html::selector("a[href^='/torrent/']")?;
    let seeds = html::selector(".coll-2.seeds")?;
    let leeches = html::selector(".coll-3.leeches")?;
    let size = html::selector(".coll-4.size")?;
    let all_tokens: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let meaningful: Vec<_> = all_tokens
        .iter()
        .filter(|t| !matches!(t.as_str(), "the" | "a" | "an" | "of" | "and" | "or" | "to"))
        .collect();
    let tokens: Vec<_> = if meaningful.is_empty() {
        all_tokens.iter().collect()
    } else {
        meaningful
    };
    let mut rows = vec![];
    for tr in doc.select(&tr).take(101) {
        let Some(link) = tr.select(&anchor).next() else {
            continue;
        };
        let name = html::bounded_text(link.text(), 1024);
        if !tokens
            .iter()
            .all(|token| name.to_lowercase().contains(token.as_str()))
        {
            continue;
        }
        let count = |selector: &scraper::Selector| {
            tr.select(selector)
                .next()
                .and_then(|n| html::bounded_text(n.text(), 32).parse().ok())
        };
        rows.push(Row {
            name,
            path: link.value().attr("href").unwrap_or("").to_owned(),
            seeders: count(&seeds),
            leechers: count(&leeches),
            size: tr
                .select(&size)
                .next()
                .and_then(|n| parse_size(&html::bounded_text(n.text().take(1), 64))),
        });
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.seeders));
    rows.truncate(4);
    Ok(rows)
}
fn parse_detail(bytes: &[u8]) -> Result<Option<(String, Option<u64>)>, ProviderFailure> {
    let text = std::str::from_utf8(bytes).map_err(|_| ProviderFailure::InvalidResponse)?;
    let Some(magnet) = html::first_magnet(text)? else {
        return Ok(None);
    };
    let doc = html::document(text)?;
    let li = html::selector("li")?;
    let strong = html::selector("strong")?;
    let span = html::selector("span")?;
    let added =
        doc.select(&li).find_map(|li| {
            if li.select(&strong).next().is_some_and(|s| {
                html::bounded_text(s.text(), 64).eq_ignore_ascii_case("Date uploaded")
            }) {
                li.select(&span)
                    .next()
                    .and_then(|s| upload_date(&html::bounded_text(s.text(), 64)))
            } else {
                None
            }
        });
    Ok(Some((magnet, added)))
}
fn upload_date(value: &str) -> Option<u64> {
    let parts: Vec<_> = value.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| *m == parts[0].trim_end_matches('.').to_lowercase())? as u32
        + 1;
    let day = parts[1]
        .trim_end_matches(|c: char| c.is_ascii_alphabetic())
        .parse()
        .ok()?;
    let year = parts[2].strip_prefix('\'')?;
    if year.len() != 2 {
        return None;
    }
    let date = chrono::NaiveDate::from_ymd_opt(2000 + year.parse::<i32>().ok()?, month, day)?
        .and_hms_opt(0, 0, 0)?
        .and_utc();
    date.timestamp_millis().try_into().ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricts_detail_links_to_fixed_origin_torrent_routes() {
        let base = url::Url::parse("https://1337x.to/").unwrap();
        assert!(detail_url(&base, "/torrent/123/fixture/").is_some());
        for path in [
            "https://evil.invalid/torrent/1",
            "//evil.invalid/torrent/1",
            "/torrent/../api",
            "/torrent/%2e%2e/admin",
            "/torrent/123/?url=http://localhost",
            "/torrent/1#fragment",
            "/torrent/nope/name",
        ] {
            assert!(detail_url(&base, path).is_none(), "{path}");
        }
    }
    #[test]
    fn parses_rows_stop_words_counts_sizes_and_detail_date() {
        let body=br#"<table class="table-list"><tr><td><a href="/torrent/123/fixture/">Fixture &amp; Collection</a></td><td class="coll-2 seeds">12</td><td class="coll-3 leeches">3</td><td class="coll-4 size">1.5 GiB<span>Uploader</span></td></tr></table>"#;
        let rows = parse_rows(body, "the fixture").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Fixture & Collection");
        assert_eq!(rows[0].size, Some(1_610_612_736));
        assert_eq!(rows[0].seeders, Some(12));
        assert!(parse_rows(body, "missing").unwrap().is_empty());
        let detail=br#"<a href="magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&amp;dn=Fixture">Download</a><ul><li><strong>Date uploaded</strong><span>Jan. 1st '24</span></li></ul>"#;
        let (magnet, date) = parse_detail(detail).unwrap().unwrap();
        assert!(magnet.contains("&dn="));
        assert_eq!(date, Some(1_704_067_200_000));
        assert!(upload_date("Feb. 31st '24").is_none());
        assert!(parse_rows(b"<html>challenge</html>", "").is_err());
    }
    #[test]
    fn encodes_query_as_one_path_segment() {
        let provider = X1337Provider::movies().unwrap();
        let query = SearchQuery::new("fixture/../TV ?x=1").unwrap();
        let url = provider.url(&provider.bases[0], &query);
        assert!(url.path().contains("%2F"));
        assert!(url.query().is_none());
        assert!(url.path().ends_with("/Movies/1/"));
    }
    #[tokio::test]
    async fn resolves_safe_details_and_never_follows_untrusted_paths() {
        let(base,server)=super::super::test_support::server(2,"text/html",|url|{
            if url.path()=="/popular-movies"{(200,r#"<table class="table-list"><tr><td><a href="/torrent/123/fixture/">Fixture</a></td><td class="coll-2 seeds">4</td></tr><tr><td><a href="/torrent/999/?redirect=http://127.0.0.1:1">Unsafe link</a></td><td class="coll-2 seeds">9</td></tr></table>"#.into())}
            else {assert_eq!(url.path(),"/torrent/123/fixture/");(200,r#"<a href="magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567">Get</a>"#.into())}
        }).await;
        let mut provider = X1337Provider::movies().unwrap();
        provider.bases = vec![base];
        provider.http = Http::local();
        let rows = provider
            .fetch(&SearchQuery::new("").unwrap())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Fixture");
        assert_eq!(server.await.unwrap().len(), 2);
    }
}
