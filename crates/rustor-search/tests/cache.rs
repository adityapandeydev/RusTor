use rustor_search::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
struct Counting {
    descriptor: ProviderDescriptor,
    calls: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
    delay: Arc<AtomicU64>,
    rows: Vec<Candidate>,
}
impl Provider for Counting {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
    fn search<'a>(&'a self, _: &'a SearchQuery) -> ProviderFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_secs(self.delay.load(Ordering::SeqCst))).await;
            if self.fail.load(Ordering::SeqCst) {
                return Err(ProviderFailure::Unavailable);
            }
            Ok(self.rows.clone())
        })
    }
}
type Fixture = (
    SearchService,
    Arc<AtomicUsize>,
    Arc<AtomicBool>,
    Arc<AtomicU64>,
);
fn rows() -> Vec<Candidate> {
    (1..=2)
        .map(|id| Candidate {
            info_hash: format!("{id:040x}"),
            name: format!("Authorized fixture {id}"),
            magnet: format!("magnet:?xt=urn:btih:{id:040x}"),
            size_bytes: Some(id * 1024),
            seeders: Some(if id == 1 { 0 } else { 5 }),
            leechers: None,
            file_count: None,
            added_unix_ms: None,
        })
        .collect()
}
fn service() -> Fixture {
    configured(
        rows(),
        SearchConfig {
            provider_timeout: Duration::from_secs(2),
            ..Default::default()
        },
    )
}
fn configured(rows: Vec<Candidate>, config: SearchConfig) -> Fixture {
    let calls = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(false));
    let delay = Arc::new(AtomicU64::new(0));
    let provider = Arc::new(Counting {
        descriptor: ProviderDescriptor::new(
            "fixture",
            "Fixture",
            vec![Category::Movies, Category::Tv],
            true,
        )
        .unwrap(),
        calls: calls.clone(),
        fail: fail.clone(),
        delay: delay.clone(),
        rows,
    });
    (
        SearchService::new(vec![provider], config).unwrap(),
        calls,
        fail,
        delay,
    )
}
async fn finish(service: &SearchService, request: SearchRequest) -> Arc<SearchSnapshot> {
    let mut handle = service.start(request).unwrap();
    while handle.snapshot().phase == SearchPhase::Running {
        handle.changed().await.unwrap();
    }
    handle.snapshot()
}
fn query(value: &str) -> SearchRequest {
    SearchRequest::new(SearchQuery::new(value).unwrap())
}
#[tokio::test(start_paused = true)]
async fn repeated_queries_reuse_provider_rows_but_reapply_category_filters_and_sorting() {
    let (service, calls, _, _) = service();
    let first = finish(&service, query("  Authorized   Fixture ")).await;
    assert_eq!(first.results.len(), 2);
    let mut request = query("authorized fixture");
    request.category = Some(Category::Tv);
    request.hide_dead = true;
    request.sort_field = SortField::Size;
    request.sort_direction = SortDirection::Ascending;
    let next = finish(&service, request).await;
    assert_eq!(next.results.len(), 1);
    assert_eq!(next.results[0].seeders, Some(5));
    assert_ne!(next.id, first.id);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test(start_paused = true)]
async fn successful_hits_expire_after_five_minutes_without_sliding_the_expiry() {
    let (service, calls, _, _) = service();
    finish(&service, query("fixture")).await;
    tokio::time::advance(Duration::from_secs(299)).await;
    finish(&service, query("fixture")).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(1)).await;
    finish(&service, query("fixture")).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
#[tokio::test(start_paused = true)]
async fn failures_and_timeouts_do_not_pin_an_unavailable_source() {
    let (service, calls, fail, delay) = service();
    fail.store(true, Ordering::SeqCst);
    assert!(finish(&service, query("fixture")).await.partial);
    fail.store(false, Ordering::SeqCst);
    finish(&service, query("fixture")).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    finish(&service, query("fixture")).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    delay.store(10, Ordering::SeqCst);
    let timed = finish(&service, query("different")).await;
    assert_eq!(timed.providers[0].status, ProviderStatus::TimedOut);
    delay.store(0, Ordering::SeqCst);
    finish(&service, query("different")).await;
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}
#[tokio::test(start_paused = true)]
async fn cancelling_in_flight_work_does_not_fill_the_cache() {
    let (service, calls, _, delay) = service();
    delay.store(10, Ordering::SeqCst);
    let mut handle = service.start(query("fixture")).unwrap();
    while calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    handle.cancel();
    while handle.snapshot().phase == SearchPhase::Running {
        handle.changed().await.unwrap();
    }
    assert_eq!(handle.snapshot().phase, SearchPhase::Cancelled);
    delay.store(0, Ordering::SeqCst);
    finish(&service, query("fixture")).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn empty_successes_are_cached_but_rejected_candidates_are_retried() {
    let (empty, calls, _, _) = configured(vec![], SearchConfig::default());
    assert!(finish(&empty, query("fixture")).await.results.is_empty());
    finish(&empty, query("fixture")).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut invalid = rows();
    invalid[0].info_hash = "invalid".into();
    let (bad, calls, _, _) = configured(invalid, SearchConfig::default());
    assert!(finish(&bad, query("fixture")).await.partial);
    assert!(finish(&bad, query("fixture")).await.partial);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
#[tokio::test(start_paused = true)]
async fn cache_hits_preserve_truncation_reporting() {
    let (service, calls, _, _) = configured(
        rows(),
        SearchConfig {
            max_per_provider: 1,
            ..Default::default()
        },
    );
    for _ in 0..2 {
        let snapshot = finish(&service, query("fixture")).await;
        assert_eq!(snapshot.results.len(), 1);
        assert!(snapshot.truncated);
        assert!(matches!(
            snapshot.providers[0].status,
            ProviderStatus::Complete {
                truncated: true,
                ..
            }
        ));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test(start_paused = true)]
async fn cached_results_publish_before_a_slow_uncached_provider_finishes() {
    let fail = Arc::new(AtomicBool::new(true));
    let delay = Arc::new(AtomicU64::new(0));
    let cached_calls = Arc::new(AtomicUsize::new(0));
    let a = Arc::new(Counting {
        descriptor: ProviderDescriptor::new("a", "A", vec![], true).unwrap(),
        calls: Arc::new(AtomicUsize::new(0)),
        fail: fail.clone(),
        delay: delay.clone(),
        rows: rows(),
    });
    let z = Arc::new(Counting {
        descriptor: ProviderDescriptor::new("z", "Z", vec![], true).unwrap(),
        calls: cached_calls.clone(),
        fail: Arc::new(AtomicBool::new(false)),
        delay: Arc::new(AtomicU64::new(0)),
        rows: rows(),
    });
    let service = SearchService::new(
        vec![a, z],
        SearchConfig {
            concurrency: 1,
            provider_timeout: Duration::from_secs(2),
            ..Default::default()
        },
    )
    .unwrap();
    finish(&service, query("fixture")).await;
    fail.store(false, Ordering::SeqCst);
    delay.store(10, Ordering::SeqCst);
    let mut handle = service.start(query("fixture")).unwrap();
    while handle.snapshot().results.is_empty() {
        handle.changed().await.unwrap();
    }
    assert_eq!(handle.snapshot().phase, SearchPhase::Running);
    assert_eq!(handle.snapshot().results.len(), 2);
    assert_eq!(cached_calls.load(Ordering::SeqCst), 1);
    handle.cancel();
}
