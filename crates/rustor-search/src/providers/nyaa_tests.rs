use super::*;
use crate::{ResultIndex, SearchQuery, SearchRequest};

const FEED: &str = include_str!("../../tests/fixtures/nyaa.xml");
#[test]
fn maps_namespaces_entities_metadata_and_unknown_values() {
    let rows = parse_feed(FEED.as_bytes()).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].name, "Public-domain & fixture α");
    assert_eq!(
        rows[0].info_hash,
        "0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(rows[0].seeders, Some(12));
    assert_eq!(rows[0].leechers, Some(3));
    assert_eq!(rows[0].size_bytes, Some(1_610_612_736));
    assert_eq!(rows[0].added_unix_ms, Some(1_704_067_200_000));
    assert_eq!(rows[1].name, "Second & fixture");
    assert_eq!(rows[1].seeders, None);
    assert_eq!(rows[1].size_bytes, None);
    assert_eq!(rows[1].added_unix_ms, None);
    let magnet = url::Url::parse(&rows[0].magnet).unwrap();
    assert_eq!(
        magnet.query_pairs().filter(|(key, _)| key == "tr").count(),
        TRACKERS.len()
    );
    assert!(
        magnet
            .query_pairs()
            .any(|(key, value)| key == "dn" && value == rows[0].name)
    );
    let descriptor = descriptor().unwrap();
    let mut index = ResultIndex::default();
    let mut rejected = 0;
    for row in rows {
        if index.insert(&descriptor, row).is_err() {
            rejected += 1;
        }
    }
    assert_eq!(rejected, 1);
    assert_eq!(
        index
            .results(&SearchRequest::new(SearchQuery::new("").unwrap()))
            .len(),
        2
    );
}
#[test]
fn rejects_non_rss_dtd_node_bombs_and_oversized_bodies() {
    for text in [
        "<html>challenge</html>",
        r#"<rss version="2.0"></rss>"#,
        r#"<!DOCTYPE rss [<!ENTITY x SYSTEM 'file:///not-read'>]><rss version="2.0"><channel><title>&x;</title></channel></rss>"#,
    ] {
        assert!(matches!(
            parse_feed(text.as_bytes()),
            Err(ProviderFailure::InvalidResponse)
        ));
    }
    let many = format!(
        r#"<rss version="2.0"><channel>{}</channel></rss>"#,
        "<x/>".repeat(MAX_NODES as usize + 1)
    );
    assert!(parse_feed(many.as_bytes()).is_err());
    assert!(parse_feed(&vec![b' '; MAX_BODY_BYTES + 1]).is_err());
    assert!(parse_feed(&[0xff, 0xfe]).is_err());
}
#[test]
fn valid_empty_feeds_and_wrong_namespaces_do_not_fabricate_results() {
    assert!(
        parse_feed(br#"<rss version="2.0"><channel/></rss>"#)
            .unwrap()
            .is_empty()
    );
    let wrong = FEED.replace(
        "https://nyaa.si/xmlns/nyaa",
        "https://example.invalid/wrong",
    );
    assert!(
        parse_feed(wrong.as_bytes())
            .unwrap()
            .iter()
            .all(|row| row.info_hash.is_empty())
    );
}
#[test]
fn parses_sizes_without_float_overflow_and_preserves_missing_health() {
    for (input, expected) in [
        ("1.5 GiB", Some(1_610_612_736)),
        ("1.25 MB", Some(1_250_000)),
        ("0 B", Some(0)),
        ("0.5 B", Some(1)),
        ("garbage", None),
        ("-1 MiB", None),
        ("NaN GiB", None),
        ("18446744073709551616 TiB", None),
    ] {
        assert_eq!(parse_size(input), expected, "{input}");
    }
    assert_eq!(count("0"), Some(0));
    assert_eq!(count(""), None);
    assert_eq!(count("-1"), None);
    assert_eq!(count("4294967296"), None);
}
#[test]
fn parses_retry_after_seconds_dates_and_invalid_values() {
    let now = std::time::UNIX_EPOCH + Duration::from_secs(1_704_067_200);
    assert_eq!(retry_after("60", now), Some(60));
    assert_eq!(retry_after("Mon, 01 Jan 2024 00:01:00 GMT", now), Some(60));
    assert_eq!(retry_after("Mon, 01 Jan 2024 00:00:00 GMT", now), Some(0));
    assert_eq!(retry_after("-1", now), None);
    assert_eq!(retry_after("invalid", now), None);
}

use crate::{ProviderStatus, SearchConfig, SearchPhase, SearchService};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

async fn read_request(socket: &mut TcpStream) -> String {
    let mut request = Vec::new();
    loop {
        let byte = socket.read_u8().await.unwrap();
        request.push(byte);
        if request.ends_with(b"\r\n\r\n") {
            break;
        }
        assert!(request.len() < 16_384);
    }
    String::from_utf8(request).unwrap()
}
async fn fixture_server(response: String) -> (NyaaProvider, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut provider = NyaaProvider::new().unwrap();
    provider.client = client(false).unwrap();
    provider.endpoint =
        url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        // Rejection of an oversized body may close the connection before this write completes.
        let _ = socket.write_all(response.as_bytes()).await;
        request
    });
    (provider, task)
}
fn response(status: &str, headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}",
        body.len()
    )
}
#[tokio::test]
async fn http_adapter_encodes_query_and_flows_through_search_validation() {
    let (provider, server) = fixture_server(response(
        "200 OK",
        "Content-Type: application/xml; charset=utf-8\r\n",
        FEED,
    ))
    .await;
    let query = SearchQuery::new("demo & f=1 #α").unwrap();
    let service = SearchService::new(vec![Arc::new(provider)], SearchConfig::default()).unwrap();
    let mut handle = service.start(SearchRequest::new(query.clone())).unwrap();
    let snapshot = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let snapshot = handle.changed().await.unwrap();
            if snapshot.phase != SearchPhase::Running {
                break snapshot;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(snapshot.phase, SearchPhase::Completed);
    assert_eq!(snapshot.results.len(), 2);
    assert!(matches!(
        snapshot.providers[0].status,
        ProviderStatus::Complete {
            accepted: 2,
            rejected: 1,
            truncated: false
        }
    ));
    let request = server.await.unwrap();
    let target = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    let pairs: Vec<_> = url.query_pairs().collect();
    assert_eq!(pairs.len(), 4);
    assert!(pairs.iter().any(|(k, v)| k == "q" && v == query.as_str()));
    assert!(pairs.iter().any(|(k, v)| k == "f" && v == "0"));
}
#[tokio::test]
async fn rejects_html_redirects_encoding_and_both_body_size_overflows() {
    let cases = [
        (
            response(
                "200 OK",
                "Content-Type: text/html\r\n",
                "<html>challenge</html>",
            ),
            ProviderFailure::InvalidResponse,
        ),
        (
            response(
                "302 Found",
                "Location: http://127.0.0.1:1/not-followed\r\n",
                "",
            ),
            ProviderFailure::Unavailable,
        ),
        (
            response(
                "200 OK",
                "Content-Type: application/xml\r\nContent-Encoding: gzip\r\n",
                FEED,
            ),
            ProviderFailure::InvalidResponse,
        ),
        (
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\n\r\n",
                MAX_BODY_BYTES + 1
            ),
            ProviderFailure::InvalidResponse,
        ),
        (
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                MAX_BODY_BYTES + 1,
                " ".repeat(MAX_BODY_BYTES + 1)
            ),
            ProviderFailure::InvalidResponse,
        ),
    ];
    for (wire, expected) in cases {
        let (provider, server) = fixture_server(wire).await;
        assert_eq!(
            provider
                .fetch(&SearchQuery::new("").unwrap())
                .await
                .unwrap_err(),
            expected
        );
        server.await.unwrap();
    }
}
#[tokio::test]
async fn honors_retry_after_without_sending_another_request() {
    for (status, expected) in [
        (
            "429 Too Many Requests",
            ProviderFailure::RateLimited {
                retry_after_seconds: Some(60),
            },
        ),
        ("503 Service Unavailable", ProviderFailure::Unavailable),
    ] {
        let (provider, server) = fixture_server(response(status, "Retry-After: 60\r\n", "")).await;
        let query = SearchQuery::new("").unwrap();
        assert_eq!(provider.fetch(&query).await.unwrap_err(), expected);
        server.await.unwrap(); // Listener has closed: another HTTP attempt would return Unavailable.
        let second = tokio::time::timeout(Duration::from_millis(200), provider.fetch(&query))
            .await
            .unwrap()
            .unwrap_err();
        match second {
            ProviderFailure::RateLimited {
                retry_after_seconds: Some(s),
            } => assert!((59..=60).contains(&s)),
            other => assert_eq!(other, expected),
        }
        assert!(provider.gate.lock().await.cooldown.is_some());
    }
}
#[tokio::test]
async fn cancellation_drops_a_pending_http_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut provider = NyaaProvider::new().unwrap();
    provider.client = client(false).unwrap();
    provider.endpoint =
        url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: 100\r\n\r\n",
            )
            .await
            .unwrap();
        ready_tx.send(()).unwrap();
        let mut byte = [0];
        tokio::time::timeout(Duration::from_secs(3), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap()
    });
    let service = SearchService::new(vec![Arc::new(provider)], SearchConfig::default()).unwrap();
    let mut handle = service
        .start(SearchRequest::new(SearchQuery::new("").unwrap()))
        .unwrap();
    ready_rx.await.unwrap();
    handle.cancel();
    tokio::time::timeout(Duration::from_secs(3), async {
        while handle.snapshot().phase == SearchPhase::Running {
            handle.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(handle.snapshot().phase, SearchPhase::Cancelled);
    assert_eq!(server.await.unwrap(), 0);
}
#[tokio::test(start_paused = true)]
async fn waiting_requests_recheck_new_cooldowns_and_preserve_spacing() {
    let provider = NyaaProvider::new().unwrap();
    provider.reserve().await.unwrap();
    let wait = provider.reserve();
    tokio::pin!(wait);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), &mut wait)
            .await
            .is_err()
    );
    provider
        .cooldown(
            60,
            ProviderFailure::RateLimited {
                retry_after_seconds: Some(60),
            },
        )
        .await;
    assert!(matches!(
        wait.await,
        Err(ProviderFailure::RateLimited { .. })
    ));
    tokio::time::advance(Duration::from_secs(60)).await;
    provider.reserve().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), provider.reserve())
            .await
            .is_err()
    );
}
#[test]
fn limits_decoded_rows_and_exposes_overflow_to_the_service() {
    let item = FEED
        .split("<item>")
        .nth(1)
        .unwrap()
        .split("</item>")
        .next()
        .unwrap();
    let xml = format!(
        r#"<rss version="2.0" xmlns:n="https://nyaa.si/xmlns/nyaa"><channel>{}</channel></rss>"#,
        format!("<item>{item}</item>").repeat(150)
    );
    assert_eq!(parse_feed(xml.as_bytes()).unwrap().len(), MAX_ROWS);
}
