use super::{
    html,
    http::{Http, MAX_BODY_BYTES, Media},
    metadata::*,
};
use crate::{
    Candidate, Category, Provider, ProviderDescriptor, ProviderFailure, ProviderFuture,
    SearchError, SearchQuery,
};
use std::{collections::BTreeSet, time::Duration};

pub struct FitGirlProvider {
    descriptor: ProviderDescriptor,
    http: Http,
    base: url::Url,
}
impl FitGirlProvider {
    pub fn new() -> Result<Self, SearchError> {
        Ok(Self {
            descriptor: ProviderDescriptor::new(
                "fitgirl",
                "FitGirl",
                vec![Category::Games],
                false,
            )?,
            http: Http::new()?,
            base: url::Url::parse("https://fitgirl-repacks.site/")
                .map_err(|_| SearchError::InvalidConfig("invalid fixed endpoint"))?,
        })
    }
    fn url(&self, query: &SearchQuery, page: usize) -> url::Url {
        let mut url = self.base.clone();
        if query.as_str().is_empty() {
            url.set_path("/feed/");
        } else {
            url.query_pairs_mut()
                .append_pair("s", query.as_str())
                .append_pair("feed", "rss2")
                .append_pair("orderby", "relevance");
        }
        if page > 1 {
            url.query_pairs_mut()
                .append_pair("paged", &page.to_string());
        }
        url
    }
    async fn fetch(&self, query: &SearchQuery) -> Result<Vec<Candidate>, ProviderFailure> {
        let bytes = self
            .http
            .get(self.url(query, 1), Media::Xml, Duration::from_secs(8))
            .await?;
        let (mut out, count) = tokio::task::spawn_blocking(move || parse(&bytes))
            .await
            .map_err(|_| ProviderFailure::InvalidResponse)??;
        if count >= 10 {
            for page in 2..=3 {
                let bytes = match self
                    .http
                    .get(self.url(query, page), Media::Xml, Duration::from_secs(8))
                    .await
                {
                    Ok(bytes) => bytes,
                    Err(ProviderFailure::RateLimited { .. }) => break,
                    Err(_) => continue,
                };
                if let Ok(Ok((rows, _))) = tokio::task::spawn_blocking(move || parse(&bytes)).await
                {
                    out.extend(rows);
                }
            }
        }
        let mut seen = BTreeSet::new();
        out.retain(|row| seen.insert(row.info_hash.clone()));
        out.truncate(101);
        Ok(out)
    }
}
impl Provider for FitGirlProvider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
    fn search<'a>(&'a self, query: &'a SearchQuery) -> ProviderFuture<'a> {
        Box::pin(self.fetch(query))
    }
}
fn parse(bytes: &[u8]) -> Result<(Vec<Candidate>, usize), ProviderFailure> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(ProviderFailure::InvalidResponse);
    }
    let xml = std::str::from_utf8(bytes).map_err(|_| ProviderFailure::InvalidResponse)?;
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 20_000,
            ..Default::default()
        },
    )
    .map_err(|_| ProviderFailure::InvalidResponse)?;
    let root = doc.root_element();
    if !root.has_tag_name("rss") {
        return Err(ProviderFailure::InvalidResponse);
    }
    let channel = root
        .children()
        .find(|n| n.has_tag_name("channel"))
        .ok_or(ProviderFailure::InvalidResponse)?;
    let mut out = vec![];
    let mut count = 0;
    for item in channel
        .children()
        .filter(|n| n.has_tag_name("item"))
        .take(101)
    {
        count += 1;
        let field = |name: &str| {
            item.children()
                .find(|n| n.is_element() && n.tag_name().name() == name)
                .and_then(|n| n.text())
                .unwrap_or("")
        };
        let content = field("encoded");
        let content = if content.is_empty() {
            field("description")
        } else {
            content
        };
        let Some(magnet) = html::first_magnet(content)? else {
            continue;
        };
        let title = field("title");
        let name = if title.len() <= 1024 {
            title.to_owned()
        } else {
            String::new()
        };
        let mut row = candidate(magnet_hash(&magnet).unwrap_or_default(), name);
        row.artwork_url = html::first_artwork(content, "fitgirl")?;
        row.magnet = native_magnet(magnet);
        row.added_unix_ms = timestamp(field("pubDate"));
        out.push(row);
    }
    Ok((out, count))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_cdata_entities_dates_and_unknown_health_without_fetching_item_links() {
        let xml=br#"<rss xmlns:content="http://purl.org/rss/1.0/modules/content/"><channel><item><title>Fixture &amp; extras</title><link>http://127.0.0.1:1/not-fetched</link><pubDate>Mon, 01 Jan 2024 00:00:00 GMT</pubDate><content:encoded><![CDATA[<a href="magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&amp;tr=udp%3A%2F%2Ftracker.example%3A80%2Fannounce">Get</a>]]></content:encoded></item><item><title>No magnet</title></item></channel></rss>"#;
        let (rows, count) = parse(xml).unwrap();
        assert_eq!(count, 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Fixture & extras");
        assert!(rows[0].magnet.contains("&tr="));
        assert_eq!(rows[0].seeders, None);
        assert_eq!(rows[0].added_unix_ms, Some(1_704_067_200_000));
        assert!(parse(b"<html>challenge</html>").is_err());
        assert!(parse(br#"<!DOCTYPE rss [<!ENTITY x SYSTEM 'file:///no'>]><rss><channel>&x;</channel></rss>"#).is_err());
    }
    #[test]
    fn builds_relevant_three_page_queries() {
        let p = FitGirlProvider::new().unwrap();
        let q = SearchQuery::new("fixture & feed=bad").unwrap();
        let url = p.url(&q, 3);
        assert_eq!(url.query_pairs().count(), 4);
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "orderby" && v == "relevance")
        );
        assert!(url.query_pairs().any(|(k, v)| k == "s" && v == q.as_str()));
        assert_eq!(p.url(&SearchQuery::new("").unwrap(), 1).path(), "/feed/");
    }
    #[test]
    fn artwork_skips_decorative_images_and_keeps_the_game_cover() {
        let content = r#"<img src="https://media.tenor.com/decorative.gif"><img src="https://i3.imageban.ru/out/2026/01/cover.jpg">"#;
        assert_eq!(
            html::first_artwork(content, "fitgirl").unwrap().as_deref(),
            Some("https://i3.imageban.ru/out/2026/01/cover.jpg")
        );
    }
    #[tokio::test]
    async fn widens_full_feeds_keeps_partial_results_and_deduplicates_pages() {
        let(base,server)=super::super::test_support::server(3,"application/rss+xml",|url|{
            let page=url.query_pairs().find(|(k,_)|k=="paged").map(|(_,v)|v.into_owned()).unwrap_or("1".into());
            if page=="2" {return (503,String::new());}
            let item=|id:usize| format!(r#"<item><title>Fixture {id}</title><description><![CDATA[<a href="magnet:?xt=urn:btih:{id:040x}">Get</a>]]></description></item>"#);
            let body=if page=="1"{(1..=10).map(item).collect::<String>()}else{format!("{}{}",item(1),item(11))};
            (200,format!("<rss><channel>{body}</channel></rss>"))
        }).await;
        let mut provider = FitGirlProvider::new().unwrap();
        provider.base = base;
        provider.http = Http::local();
        let rows = provider
            .fetch(&SearchQuery::new("fixture").unwrap())
            .await
            .unwrap();
        assert_eq!(rows.len(), 11);
        assert_eq!(server.await.unwrap().len(), 3);
    }
}
