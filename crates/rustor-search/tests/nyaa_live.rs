use rustor_search::{Provider, ResultIndex, SearchQuery, SearchRequest, providers::NyaaProvider};

/// Opt-in network smoke check; CI uses synthetic feeds and loopback HTTP only.
#[tokio::test]
#[ignore = "contacts the public Nyaa RSS endpoint"]
async fn live_feed_returns_validated_metadata() {
    let provider = NyaaProvider::new().unwrap();
    let rows = provider
        .search(&SearchQuery::new("").unwrap())
        .await
        .unwrap();
    let count = rows.len();
    let mut index = ResultIndex::default();
    for row in rows {
        index.insert(provider.descriptor(), row).unwrap();
    }
    let valid = index
        .results(&SearchRequest::new(SearchQuery::new("").unwrap()))
        .len();
    assert!(valid > 0);
    println!(
        "Live RSS metadata: {count} candidates, {valid} validated results; no transfers started."
    );
}
