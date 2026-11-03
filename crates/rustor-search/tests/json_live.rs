use rustor_search::{Provider, ResultIndex, SearchQuery, SearchRequest, providers::JsonProvider};

async fn check(provider: JsonProvider, label: &str) {
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
    println!("{label}: {count} candidates, {valid} validated results; metadata only.");
}
#[tokio::test]
#[ignore = "contacts public YTS metadata API"]
async fn live_yts_metadata() {
    check(JsonProvider::yts().unwrap(), "YTS").await;
}
#[tokio::test]
#[ignore = "contacts public SubsPlease metadata API"]
async fn live_subsplease_metadata() {
    check(JsonProvider::subsplease().unwrap(), "SubsPlease").await;
}
