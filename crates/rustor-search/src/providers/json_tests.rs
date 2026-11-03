use super::*;
use crate::{ResultIndex, SearchRequest};
use serde_json::json;
const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
fn yts() -> Value {
    json!({"status":"ok","data":{"movies":[{"title_long":"Authorized fixture (2024)","date_uploaded_unix":1704067200_u64,"torrents":[{"hash":HASH.to_uppercase(),"quality":"1080p","type":"web","size_bytes":4096,"seeds":12,"peers":3}]}]}})
}
fn parse(kind: Kind, value: Value) -> Vec<Candidate> {
    decode(kind, &serde_json::to_vec(&value).unwrap()).unwrap()
}
#[test]
fn yts_maps_all_variants_counts_dates_and_keeps_unknown_metadata() {
    let mut value = yts();
    value["data"]["movies"][0]["torrents"]
        .as_array_mut()
        .unwrap()
        .push(json!({"hash":"A".repeat(32),"quality":"720p","seeds":-1,"peers":4294967296_u64}));
    let rows = parse(Kind::Yts, value);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].info_hash, HASH);
    assert_eq!(rows[0].name, "Authorized fixture (2024) [1080p web]");
    assert_eq!(rows[0].seeders, Some(12));
    assert_eq!(rows[0].leechers, Some(3));
    assert_eq!(rows[0].size_bytes, Some(4096));
    assert_eq!(rows[0].added_unix_ms, Some(1_704_067_200_000));
    assert_eq!(rows[1].seeders, None);
    assert_eq!(rows[1].leechers, None);
    assert_eq!(rows[1].size_bytes, None);
    let url = url::Url::parse(&rows[0].magnet).unwrap();
    assert!(
        url.query_pairs()
            .any(|(key, value)| key == "dn" && value == rows[0].name)
    );
    assert_eq!(
        url.query_pairs().filter(|(key, _)| key == "tr").count(),
        magnet::TRACKERS.len()
    );
}
#[test]
fn bittorrented_preserves_metadata_unknown_counts_and_rejected_rows() {
    let rows = parse(
        Kind::BitTorrented,
        json!({"results":[{"torrent_infohash":HASH,"torrent_name":"Public fixture","torrent_total_size":0,"torrent_seeders":0,"torrent_leechers":null,"torrent_file_count":4,"torrent_created_at":"2024-01-01T00:00:00Z"},{"torrent_infohash":"invalid","torrent_name":"Rejected"}]}),
    );
    assert_eq!(rows[0].size_bytes, Some(0));
    assert_eq!(rows[0].seeders, Some(0));
    assert_eq!(rows[0].leechers, None);
    assert_eq!(rows[0].file_count, Some(4));
    assert_eq!(rows[0].added_unix_ms, Some(1_704_067_200_000));
    let provider = JsonProvider::bittorrented().unwrap();
    let mut index = ResultIndex::default();
    assert!(index.insert(provider.descriptor(), rows[0].clone()).is_ok());
    assert!(
        index
            .insert(provider.descriptor(), rows[1].clone())
            .is_err()
    );
}
#[test]
fn subsplease_selects_valid_preferred_resolution_and_preserves_original_magnet() {
    let original = format!(
        "magnet:?xt=urn:btih:{HASH}&xl=2048&tr=udp%3A%2F%2Ftracker.example%3A80%2Fannounce&x.test=keep"
    );
    let rows = parse(
        Kind::SubsPlease,
        json!({"fixture":{"show":"Authorized series","episode":"01","release_date":"Mon, 01 Jan 2024 00:00:00 +0000","downloads":[{"res":"480","magnet":original},{"res":"1080","magnet":"https://example.invalid/not-a-magnet"},{"res":"720","magnet":original}]}}),
    );
    assert_eq!(rows[0].name, "Authorized series - 01 [720p]");
    assert_eq!(rows[0].magnet, original);
    assert_eq!(rows[0].size_bytes, Some(2048));
    assert_eq!(rows[0].added_unix_ms, Some(1_704_067_200_000));
    assert_eq!(rows[0].seeders, None);
    let provider = JsonProvider::subsplease().unwrap();
    let mut index = ResultIndex::default();
    index
        .insert(provider.descriptor(), rows[0].clone())
        .unwrap();
    let mut request = SearchRequest::new(SearchQuery::new("").unwrap());
    request.hide_dead = true;
    let results = index.results(&request);
    assert_eq!(results.len(), 1);
    assert!(results[0].has_unknown_health);
}
#[test]
fn bounds_rows_and_strings_without_fabricating_missing_values() {
    let mut value = yts();
    value["data"]["movies"][0]["title_long"] = json!("x".repeat(1_000_000));
    value["data"]["movies"][0]["torrents"] =
        json!((0..150).map(|_| json!({"hash":HASH})).collect::<Vec<_>>());
    let rows = parse(Kind::Yts, value);
    assert_eq!(rows.len(), MAX_ROWS);
    assert!(
        rows.iter()
            .all(|r| r.name.is_empty() && r.name.len() < 1025)
    );
    let entries = (0..150)
        .map(|i| {
            (
                format!("fixture{i}"),
                json!({"show":"Fixture","downloads":[]}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    assert_eq!(
        parse(Kind::SubsPlease, Value::Object(entries)).len(),
        MAX_ROWS
    );
}
#[test]
fn rejects_bad_envelopes_oversized_json_and_excessive_nesting() {
    for (kind, value) in [
        (Kind::Yts, json!({})),
        (Kind::Yts, json!({"status":"error","data":{"movies":[]}})),
        (Kind::BitTorrented, json!({"error":"not authorized"})),
        (Kind::SubsPlease, json!([{"unexpected":true}])),
    ] {
        assert!(decode(kind, &serde_json::to_vec(&value).unwrap()).is_err());
    }
    assert!(decode(Kind::Yts, b"<html>challenge</html>").is_err());
    assert!(decode(Kind::Yts, &vec![b' '; MAX_BODY_BYTES + 1]).is_err());
    assert!(
        decode(
            Kind::Yts,
            format!("{}0{}", "[".repeat(150), "]".repeat(150)).as_bytes()
        )
        .is_err()
    );
    assert!(parse(Kind::Yts, json!({"data":{"movie_count":0}})).is_empty());
    assert!(parse(Kind::BitTorrented, json!({"results":[]})).is_empty());
    assert!(parse(Kind::SubsPlease, json!([])).is_empty());
    assert!(parse(Kind::SubsPlease, json!({})).is_empty());
}
#[test]
fn encodes_queries_and_preserves_upstream_category_and_browse_parameters() {
    let query = SearchQuery::new("public & limit=999 #α").unwrap();
    for provider in [
        JsonProvider::yts().unwrap(),
        JsonProvider::bittorrented().unwrap(),
        JsonProvider::subsplease().unwrap(),
    ] {
        let url = provider.request_url(&provider.endpoints[0], &query);
        assert!(url.query_pairs().any(|(_, value)| value == query.as_str()));
        assert!(
            !url.query_pairs()
                .any(|(key, value)| key == "limit" && value == "999")
        );
        if matches!(provider.kind, Kind::BitTorrented) {
            assert!(
                url.query_pairs()
                    .any(|(key, value)| key == "type" && value == "video")
            );
        }
    }
    let provider = JsonProvider::yts().unwrap();
    assert!(
        provider
            .request_url(&provider.endpoints[0], &SearchQuery::new("").unwrap())
            .query_pairs()
            .any(|(k, v)| k == "sort_by" && v == "date_added")
    );
}

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
async fn server(
    status: &str,
    media: &str,
    body: String,
) -> (url::Url, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let wire = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![];
        loop {
            request.push(socket.read_u8().await.unwrap());
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
            assert!(request.len() < 16384);
        }
        let _ = socket.write_all(wire.as_bytes()).await;
        String::from_utf8(request).unwrap()
    });
    (endpoint, task)
}
fn local(mut provider: JsonProvider, endpoints: Vec<url::Url>) -> JsonProvider {
    provider.http.client = super::super::http::client(false).unwrap();
    provider.endpoints = endpoints;
    provider
}
#[tokio::test]
async fn yts_falls_back_to_next_configured_mirror_with_original_query() {
    let (first, one) = server("503 Service Unavailable", "application/json", "{}".into()).await;
    let (second, two) = server("200 OK", "application/json", yts().to_string()).await;
    let provider = local(JsonProvider::yts().unwrap(), vec![first, second]);
    let query = SearchQuery::new("public & domain").unwrap();
    let rows = provider.fetch(&query).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(one.await.unwrap().contains("query_term=public+%26+domain"));
    assert!(two.await.unwrap().contains("query_term=public+%26+domain"));
}
#[tokio::test]
async fn rate_limit_does_not_rotate_mirrors_and_short_queries_do_not_request_bittorrented() {
    let (endpoint, task) = server("429 Too Many Requests", "application/json", "{}".into()).await;
    let provider = local(
        JsonProvider::yts().unwrap(),
        vec![endpoint, url::Url::parse("http://127.0.0.1:1/").unwrap()],
    );
    assert!(matches!(
        provider.fetch(&SearchQuery::new("public").unwrap()).await,
        Err(ProviderFailure::RateLimited { .. })
    ));
    task.await.unwrap();
    let provider = local(
        JsonProvider::bittorrented().unwrap(),
        vec![url::Url::parse("http://127.0.0.1:1/").unwrap()],
    );
    assert!(
        provider
            .fetch(&SearchQuery::new("ab").unwrap())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        provider
            .fetch(&SearchQuery::new("abc").unwrap())
            .await
            .unwrap_err(),
        ProviderFailure::Unavailable
    );
}
#[tokio::test]
async fn subsplease_accepts_its_mislabeled_json_but_rejects_html_and_auth_errors() {
    let query = SearchQuery::new("").unwrap();
    for (status, body, expected) in [
        ("200 OK", "{}", None),
        (
            "200 OK",
            "<html>challenge</html>",
            Some(ProviderFailure::InvalidResponse),
        ),
        ("401 Unauthorized", "{}", Some(ProviderFailure::Unavailable)),
    ] {
        let (endpoint, task) = server(status, "text/html; charset=UTF-8", body.into()).await;
        let provider = local(JsonProvider::subsplease().unwrap(), vec![endpoint]);
        let result = provider.fetch(&query).await;
        match expected {
            None => assert!(result.unwrap().is_empty()),
            Some(failure) => assert_eq!(result.unwrap_err(), failure),
        }
        task.await.unwrap();
    }
}

#[test]
fn strips_only_websocket_trackers_without_rewriting_other_magnet_parameters() {
    let supported = format!(
        "magnet:?xt=urn:btih:{HASH}&tr=udp%3A%2F%2Ftracker.example%3A80%2Fannounce&x.test=keep%20this"
    );
    let original = format!(
        "{supported}&tr=wss%3A%2F%2Ftracker.example%2Fannounce&tr=ws%3A%2F%2Ftracker.example"
    );
    let rows = parse(
        Kind::SubsPlease,
        json!({"fixture":{"show":"Fixture","downloads":[{"res":"1080","magnet":original}]}}),
    );
    assert_eq!(rows[0].magnet, supported);
    let provider = JsonProvider::subsplease().unwrap();
    let mut index = ResultIndex::default();
    assert!(index.insert(provider.descriptor(), rows[0].clone()).is_ok());
    let invalid = format!("{supported}&tr=ftp%3A%2F%2Ftracker.example");
    assert_eq!(native_magnet(invalid.clone()), invalid);
    let fragmented = format!("{supported}&tr=wss://tracker.example/#fragment");
    assert_eq!(native_magnet(fragmented.clone()), fragmented);
}
#[test]
fn registry_exposes_connected_categories_without_network_calls() {
    let providers = super::super::desktop_providers().unwrap();
    assert_eq!(providers.len(), 10);
    for (category, count) in [
        (Category::Anime, 2),
        (Category::Movies, 4),
        (Category::Tv, 4),
        (Category::Games, 1),
    ] {
        assert_eq!(
            providers
                .iter()
                .filter(|p| p.descriptor().categories.contains(&category))
                .count(),
            count
        );
    }
}
