use rustor_search::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct FixtureProvider {
    descriptor: ProviderDescriptor,
    delay: Duration,
    outcome: Result<Vec<Candidate>, ProviderFailure>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    panic: bool,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Provider for FixtureProvider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
    fn search<'a>(&'a self, _: &'a SearchQuery) -> ProviderFuture<'a> {
        Box::pin(async move {
            let count = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(count, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            tokio::time::sleep(self.delay).await;
            assert!(!self.panic, "controlled provider panic");
            self.outcome.clone()
        })
    }
}
fn row(id: u32) -> Candidate {
    let hash = format!("{id:040x}");
    Candidate {
        info_hash: hash.clone(),
        name: format!("Synthetic fixture {id}"),
        magnet: format!("magnet:?xt=urn:btih:{hash}"),
        size_bytes: None,
        seeders: Some(id),
        leechers: None,
        file_count: None,
        added_unix_ms: None,
        artwork_url: None,
    }
}
fn fixture(
    id: &str,
    delay: u64,
    outcome: Result<Vec<Candidate>, ProviderFailure>,
    active: &Arc<AtomicUsize>,
    peak: &Arc<AtomicUsize>,
) -> Arc<FixtureProvider> {
    Arc::new(FixtureProvider {
        descriptor: ProviderDescriptor::new(id, id, vec![Category::Movies], true).unwrap(),
        delay: Duration::from_secs(delay),
        outcome,
        active: active.clone(),
        peak: peak.clone(),
        panic: false,
    })
}
async fn final_snapshot(handle: &mut SearchHandle) -> Arc<SearchSnapshot> {
    loop {
        let snapshot = handle.snapshot();
        if snapshot.phase != SearchPhase::Running {
            return snapshot;
        }
        handle.changed().await.unwrap();
    }
}
#[tokio::test(start_paused = true)]
async fn progressive_results_survive_failure_panic_and_timeout_with_bounded_concurrency() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut broken = fixture("panic", 2, Ok(vec![]), &active, &peak);
    Arc::get_mut(&mut broken).unwrap().panic = true;
    let providers: Vec<Arc<dyn Provider>> = vec![
        fixture("fast", 1, Ok(vec![row(1)]), &active, &peak),
        fixture(
            "offline",
            2,
            Err(ProviderFailure::Unavailable),
            &active,
            &peak,
        ),
        broken,
        fixture("slow", 100, Ok(vec![row(2)]), &active, &peak),
    ];
    let service = SearchService::new(
        providers,
        SearchConfig {
            concurrency: 2,
            provider_timeout: Duration::from_secs(5),
            overall_timeout: Duration::from_secs(30),
            ..Default::default()
        },
    )
    .unwrap();
    let mut handle = service
        .start(SearchRequest::new(SearchQuery::new("fixture").unwrap()))
        .unwrap();
    loop {
        let snapshot = handle.snapshot();
        if !snapshot.results.is_empty() {
            assert_eq!(snapshot.phase, SearchPhase::Running);
            break;
        }
        handle.changed().await.unwrap();
    }
    let result = final_snapshot(&mut handle).await;
    assert_eq!(result.phase, SearchPhase::Completed);
    assert_eq!(result.results.len(), 1);
    assert!(result.partial);
    assert!(result.providers.iter().any(|p| matches!(
        p.status,
        ProviderStatus::Failed {
            failure: ProviderFailure::Panicked
        }
    )));
    assert!(
        result
            .providers
            .iter()
            .any(|p| p.status == ProviderStatus::TimedOut)
    );
    assert!(peak.load(Ordering::SeqCst) <= 2);
}
#[tokio::test(start_paused = true)]
async fn cancelling_retains_partial_results_and_cannot_publish_late_rows_into_a_new_search() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let providers: Vec<Arc<dyn Provider>> = vec![
        fixture("fast", 1, Ok(vec![row(1)]), &active, &peak),
        fixture("slow", 100, Ok(vec![row(2)]), &active, &peak),
    ];
    let service = SearchService::new(providers, SearchConfig::default()).unwrap();
    let mut old = service
        .start(SearchRequest::new(SearchQuery::new("old").unwrap()))
        .unwrap();
    while old.snapshot().results.is_empty() {
        old.changed().await.unwrap();
    }
    old.cancel();
    let cancelled = final_snapshot(&mut old).await;
    assert_eq!(cancelled.phase, SearchPhase::Cancelled);
    assert_eq!(cancelled.results.len(), 1);
    let mut new = service
        .start(SearchRequest::new(SearchQuery::new("new").unwrap()))
        .unwrap();
    let current = final_snapshot(&mut new).await;
    assert_ne!(current.id, cancelled.id);
    assert_eq!(current.query.as_str(), "new");
    assert_eq!(old.snapshot(), cancelled);
}
#[tokio::test(start_paused = true)]
async fn overall_deadline_marks_queued_work_and_drop_cancels_provider_futures() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let providers: Vec<Arc<dyn Provider>> = vec![
        fixture("a", 100, Ok(vec![row(1)]), &active, &peak),
        fixture("b", 100, Ok(vec![row(2)]), &active, &peak),
    ];
    let service = SearchService::new(
        providers,
        SearchConfig {
            concurrency: 1,
            overall_timeout: Duration::from_secs(3),
            provider_timeout: Duration::from_secs(20),
            ..Default::default()
        },
    )
    .unwrap();
    let mut handle = service
        .start(SearchRequest::new(SearchQuery::new("").unwrap()))
        .unwrap();
    let result = final_snapshot(&mut handle).await;
    assert_eq!(result.phase, SearchPhase::TimedOut);
    assert!(
        result
            .providers
            .iter()
            .all(|p| p.status == ProviderStatus::DeadlineExceeded)
    );
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(active.load(Ordering::SeqCst), 0);
    let second = service
        .start(SearchRequest::new(SearchQuery::new("").unwrap()))
        .unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
        if active.load(Ordering::SeqCst) == 1 {
            break;
        }
    }
    assert_eq!(active.load(Ordering::SeqCst), 1);
    drop(second);
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(active.load(Ordering::SeqCst), 0);
}
#[tokio::test(start_paused = true)]
async fn filters_providers_rejects_bad_rows_and_reports_caps() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut bad = row(9);
    bad.info_hash = "invalid".into();
    let providers: Vec<Arc<dyn Provider>> = vec![fixture(
        "a",
        1,
        Ok(vec![bad, row(2), row(3)]),
        &active,
        &peak,
    )];
    let service = SearchService::new(
        providers,
        SearchConfig {
            max_per_provider: 2,
            max_results: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let mut handle = service
        .start(SearchRequest::new(SearchQuery::new("").unwrap()))
        .unwrap();
    let result = final_snapshot(&mut handle).await;
    assert_eq!(result.results.len(), 1);
    assert!(result.truncated);
    assert!(matches!(
        result.providers[0].status,
        ProviderStatus::Complete {
            accepted: 1,
            rejected: 1,
            truncated: true
        }
    ));
    let mut request = SearchRequest::new(SearchQuery::new("").unwrap());
    request.category = Some(Category::Anime);
    let mut empty = service.start(request).unwrap();
    let result = final_snapshot(&mut empty).await;
    assert_eq!(result.phase, SearchPhase::Completed);
    assert!(result.providers.is_empty());
}
#[test]
fn invalid_registry_and_configuration_are_rejected() {
    assert!(
        SearchService::new(
            vec![],
            SearchConfig {
                concurrency: 0,
                ..Default::default()
            }
        )
        .is_err()
    );
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let a = fixture("a", 0, Ok(vec![]), &active, &peak);
    assert!(SearchService::new(vec![a.clone(), a], SearchConfig::default()).is_err());
    let service = SearchService::new(vec![], SearchConfig::default()).unwrap();
    assert!(matches!(
        service.start(SearchRequest::new(SearchQuery::new("").unwrap())),
        Err(SearchError::RuntimeUnavailable)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_new_search_supersedes_the_old_one_without_exceeding_the_shared_limit() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let providers: Vec<Arc<dyn Provider>> = vec![fixture("a", 3, Ok(vec![row(1)]), &active, &peak)];
    let service = SearchService::new(
        providers,
        SearchConfig {
            concurrency: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let mut old = service
        .start(SearchRequest::new(SearchQuery::new("old").unwrap()))
        .unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
        if active.load(Ordering::SeqCst) == 1 {
            break;
        }
    }
    assert_eq!(active.load(Ordering::SeqCst), 1);
    let mut new = service
        .start(SearchRequest::new(SearchQuery::new("new").unwrap()))
        .unwrap();
    assert!(!service.cancel(old.snapshot().id).unwrap());
    assert_eq!(final_snapshot(&mut old).await.phase, SearchPhase::Cancelled);
    let latest = final_snapshot(&mut new).await;
    assert_eq!(latest.phase, SearchPhase::Completed);
    assert_eq!(latest.results.len(), 1);
    assert_eq!(peak.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn slow_consumers_keep_only_the_latest_bounded_snapshot() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let providers: Vec<Arc<dyn Provider>> = vec![fixture(
        "a",
        1,
        Ok(vec![row(1), row(2), row(3)]),
        &active,
        &peak,
    )];
    let service = SearchService::new(
        providers,
        SearchConfig {
            max_results: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let mut handle = service
        .start(SearchRequest::new(SearchQuery::new("").unwrap()))
        .unwrap();
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    let latest = final_snapshot(&mut handle).await;
    assert_eq!(latest.results.len(), 1);
    assert!(latest.truncated);
    assert_eq!(latest.results[0].seeders, Some(3));
}

#[tokio::test(start_paused = true)]
async fn cancellation_before_first_poll_does_not_call_a_provider() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let providers: Vec<Arc<dyn Provider>> = vec![fixture("a", 1, Ok(vec![row(1)]), &active, &peak)];
    let service = SearchService::new(providers, SearchConfig::default()).unwrap();
    let mut handle = service
        .start(SearchRequest::new(SearchQuery::new("").unwrap()))
        .unwrap();
    assert!(service.cancel(handle.snapshot().id).unwrap());
    assert_eq!(
        final_snapshot(&mut handle).await.phase,
        SearchPhase::Cancelled
    );
    assert_eq!(peak.load(Ordering::SeqCst), 0);
}
