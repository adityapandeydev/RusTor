use rustor_search::{
    Provider, ResultIndex, SearchQuery, SearchRequest,
    providers::{EztvProvider, FitGirlProvider, TpbProvider, X1337Provider},
};
async fn check(provider: impl Provider, required: bool) {
    let result = provider.search(&SearchQuery::new("").unwrap()).await;
    match result {
        Err(error) => {
            assert!(!required, "{}: {error:?}", provider.descriptor().id);
            println!("{} unavailable: {error:?}", provider.descriptor().id);
        }
        Ok(rows) => {
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
                "{}: {count} candidates, {valid} validated; metadata only",
                provider.descriptor().id
            );
        }
    }
}
#[tokio::test]
#[ignore = "public metadata endpoint"]
async fn live_fitgirl() {
    check(FitGirlProvider::new().unwrap(), true).await;
}
#[tokio::test]
#[ignore = "public metadata endpoint"]
async fn live_tpb_movies() {
    check(TpbProvider::movies().unwrap(), true).await;
}
#[tokio::test]
#[ignore = "public metadata endpoint"]
async fn live_tpb_tv() {
    check(TpbProvider::tv().unwrap(), true).await;
}
#[tokio::test]
#[ignore = "public endpoint availability diagnostic"]
async fn live_eztv_availability() {
    check(EztvProvider::new().unwrap(), false).await;
}
#[tokio::test]
#[ignore = "public endpoint availability diagnostic"]
async fn live_1337x_availability() {
    check(X1337Provider::movies().unwrap(), false).await;
}
