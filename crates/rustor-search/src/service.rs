use crate::*;
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Semaphore, watch},
    task::{Id, JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct SearchConfig {
    pub concurrency: usize,
    /// Measured after acquiring a shared provider permit.
    pub provider_timeout: Duration,
    pub overall_timeout: Duration,
    pub max_per_provider: usize,
    pub max_results: usize,
}
impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            provider_timeout: Duration::from_secs(15),
            overall_timeout: Duration::from_secs(30),
            max_per_provider: 100,
            max_results: 500,
        }
    }
}
impl SearchConfig {
    fn validate(&self) -> Result<(), SearchError> {
        if !(1..=32).contains(&self.concurrency)
            || !(1..=100).contains(&self.max_per_provider)
            || !(1..=1000).contains(&self.max_results)
        {
            return Err(SearchError::InvalidConfig(
                "concurrency must be 1-32, provider result cap 1-100, and total result cap 1-1000",
            ));
        }
        if [self.provider_timeout, self.overall_timeout]
            .into_iter()
            .any(|d| d.is_zero() || d > Duration::from_secs(300))
        {
            return Err(SearchError::InvalidConfig(
                "deadlines must be positive and at most five minutes",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchPhase {
    Running,
    /// All attempts finished, including failed ones; inspect provider statuses.
    Completed,
    Cancelled,
    TimedOut,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProviderStatus {
    Pending,
    Running,
    Complete {
        accepted: usize,
        rejected: usize,
        truncated: bool,
    },
    Failed {
        failure: ProviderFailure,
    },
    TimedOut,
    DeadlineExceeded,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderSnapshot {
    pub descriptor: ProviderDescriptor,
    pub status: ProviderStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SearchSnapshot {
    pub id: u64,
    pub query: SearchQuery,
    pub phase: SearchPhase,
    pub providers: Vec<ProviderSnapshot>,
    pub results: Vec<SearchResult>,
    pub partial: bool,
    pub truncated: bool,
}
#[derive(Clone)]
struct Registered {
    provider: Arc<dyn Provider>,
    descriptor: ProviderDescriptor,
}
struct Shared {
    permits: Semaphore,
    cache: Mutex<crate::cache::Cache>,
}
pub struct SearchService {
    providers: Vec<Registered>,
    config: SearchConfig,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    current: Mutex<Option<(u64, CancellationToken)>>,
}
impl SearchService {
    pub fn new(
        providers: Vec<Arc<dyn Provider>>,
        config: SearchConfig,
    ) -> Result<Self, SearchError> {
        config.validate()?;
        if providers.len() > 32 {
            return Err(SearchError::InvalidConfig("register at most 32 providers"));
        }
        let mut ids = BTreeSet::new();
        let mut registered = Vec::new();
        for provider in providers {
            let d = provider.descriptor();
            let descriptor = ProviderDescriptor::new(
                d.id.clone(),
                d.label.clone(),
                d.categories.clone(),
                d.reports_health,
            )?;
            if !ids.insert(descriptor.id.clone()) {
                return Err(SearchError::InvalidProvider("duplicate provider id"));
            }
            registered.push(Registered {
                provider,
                descriptor,
            });
        }
        registered.sort_by(|a, b| a.descriptor.id.cmp(&b.descriptor.id));
        Ok(Self {
            providers: registered,
            shared: Arc::new(Shared {
                permits: Semaphore::new(config.concurrency),
                cache: Mutex::new(crate::cache::Cache::default()),
            }),
            config,
            next_id: AtomicU64::new(1),
            current: Mutex::new(None),
        })
    }
    /// Cancel the matching search without allowing a stale UI request to stop a newer one.
    pub fn cancel(&self, id: u64) -> Result<bool, SearchError> {
        let current = self
            .current
            .lock()
            .map_err(|_| SearchError::InvalidConfig("search coordination lock unavailable"))?;
        if let Some((active_id, token)) = current.as_ref()
            && *active_id == id
        {
            token.cancel();
            return Ok(true);
        }
        Ok(false)
    }

    /// Start inside a Tokio runtime with time enabled. A new search cancels its predecessor.
    pub fn start(&self, request: SearchRequest) -> Result<SearchHandle, SearchError> {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| SearchError::RuntimeUnavailable)?;
        let mut current = self
            .current
            .lock()
            .map_err(|_| SearchError::InvalidConfig("search coordination lock unavailable"))?;
        let id = self
            .next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| SearchError::IdentityExhausted)?;
        if let Some((_, previous)) = current.take() {
            previous.cancel();
        }
        let cancel = CancellationToken::new();
        *current = Some((id, cancel.clone()));
        let selected = self
            .providers
            .iter()
            .filter(|p| {
                request
                    .category
                    .is_none_or(|category| p.descriptor.categories.contains(&category))
            })
            .cloned()
            .collect::<Vec<_>>();
        let snapshot = SearchSnapshot {
            id,
            query: request.query.clone(),
            phase: SearchPhase::Running,
            providers: selected
                .iter()
                .map(|p| ProviderSnapshot {
                    descriptor: p.descriptor.clone(),
                    status: ProviderStatus::Pending,
                })
                .collect(),
            results: Vec::new(),
            partial: false,
            truncated: false,
        };
        let (sender, updates) = watch::channel(Arc::new(snapshot.clone()));
        let task = runtime.spawn(run(
            selected,
            self.config.clone(),
            self.shared.clone(),
            request,
            snapshot,
            sender,
            cancel.clone(),
        ));
        Ok(SearchHandle {
            updates,
            cancel,
            task,
        })
    }
}
pub struct SearchHandle {
    updates: watch::Receiver<Arc<SearchSnapshot>>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}
impl SearchHandle {
    pub fn snapshot(&self) -> Arc<SearchSnapshot> {
        self.updates.borrow().clone()
    }
    pub async fn changed(&mut self) -> Result<Arc<SearchSnapshot>, watch::error::RecvError> {
        self.updates.changed().await?;
        Ok(self.updates.borrow_and_update().clone())
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}
impl Drop for SearchHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

type Outcome = Result<Result<Vec<Candidate>, ProviderFailure>, tokio::time::error::Elapsed>;
async fn run(
    providers: Vec<Registered>,
    config: SearchConfig,
    shared: Arc<Shared>,
    request: SearchRequest,
    mut snapshot: SearchSnapshot,
    sender: watch::Sender<Arc<SearchSnapshot>>,
    cancel: CancellationToken,
) {
    let mut pending = VecDeque::new();
    let mut jobs = JoinSet::<(usize, Outcome)>::new();
    let mut identities = HashMap::<Id, usize>::new();
    let mut index = ResultIndex::default();
    let deadline = tokio::time::Instant::now() + config.overall_timeout;
    let overall = tokio::time::sleep_until(deadline);
    tokio::pin!(overall);
    // Hydrate every fresh hit before scheduling misses, even when provider permits are busy.
    for (i, provider) in providers.iter().enumerate() {
        let cached = shared
            .cache
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&provider.descriptor.id, &request.query));
        if let Some(rows) = cached {
            snapshot.providers[i].status = accept_rows(
                &mut index,
                &provider.descriptor,
                &rows,
                config.max_per_provider,
            );
        } else {
            pending.push_back(i);
        }
    }
    let (phase, status) = loop {
        if cancel.is_cancelled() {
            break (SearchPhase::Cancelled, ProviderStatus::Cancelled);
        }
        if tokio::time::Instant::now() >= deadline {
            break (SearchPhase::TimedOut, ProviderStatus::DeadlineExceeded);
        }
        while jobs.len() < config.concurrency
            && let Some(i) = pending.pop_front()
        {
            let provider = providers[i].provider.clone();
            let query = request.query.clone();
            let timeout = config.provider_timeout;
            let shared = shared.clone();
            snapshot.providers[i].status = ProviderStatus::Running;
            let job = jobs.spawn(async move {
                let _permit = match shared.permits.acquire().await {
                    Ok(permit) => permit,
                    Err(_) => return (i, Ok(Err(ProviderFailure::Unavailable))),
                };
                (
                    i,
                    tokio::time::timeout(timeout, async { provider.search(&query).await }).await,
                )
            });
            identities.insert(job.id(), i);
        }
        publish(&index, &request, &config, &mut snapshot, &sender);
        if jobs.is_empty() {
            snapshot.phase = SearchPhase::Completed;
            publish(&index, &request, &config, &mut snapshot, &sender);
            return;
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                break (SearchPhase::Cancelled, ProviderStatus::Cancelled);
            }
            _ = &mut overall => {
                break (SearchPhase::TimedOut, ProviderStatus::DeadlineExceeded);
            }
            joined = jobs.join_next_with_id() => {
                match joined {
                    Some(Ok((task_id,(i,outcome)))) => {
                        identities.remove(&task_id);
                        snapshot.providers[i].status = match outcome {
                            Err(_) => ProviderStatus::TimedOut,
                            Ok(Err(failure)) => ProviderStatus::Failed { failure },
                            Ok(Ok(rows)) => {
                                let status = accept_rows(&mut index, &providers[i].descriptor, &rows, config.max_per_provider);
                                // Only fully valid successful responses are reusable. A failed/cancelled
                                // provider or malformed response must be allowed to recover on retry.
                                if matches!(status,ProviderStatus::Complete { rejected: 0,.. }) && !cancel.is_cancelled()
                                    && let Ok(mut cache)=shared.cache.lock() {
                                    cache.insert(&providers[i].descriptor.id,&request.query,&rows,config.max_per_provider);
                                }
                                status
                            }
                        };
                    }
                    Some(Err(error)) => if let Some(i) = identities.remove(&error.id()) {
                        snapshot.providers[i].status = ProviderStatus::Failed { failure: ProviderFailure::Panicked };
                    },
                    None => {}
                }
            }
        }
    };
    finish(&mut snapshot, phase, status);
    jobs.abort_all();
    publish(&index, &request, &config, &mut snapshot, &sender);
}
fn accept_rows(
    index: &mut ResultIndex,
    descriptor: &ProviderDescriptor,
    rows: &[Candidate],
    cap: usize,
) -> ProviderStatus {
    let mut accepted = 0;
    let mut rejected = 0;
    for row in rows.iter().take(cap) {
        if index.insert(descriptor, row.clone()).is_ok() {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    ProviderStatus::Complete {
        accepted,
        rejected,
        truncated: rows.len() > cap,
    }
}
fn finish(snapshot: &mut SearchSnapshot, phase: SearchPhase, status: ProviderStatus) {
    snapshot.phase = phase;
    for provider in &mut snapshot.providers {
        if matches!(
            provider.status,
            ProviderStatus::Pending | ProviderStatus::Running
        ) {
            provider.status = status.clone();
        }
    }
}
fn publish(
    index: &ResultIndex,
    request: &SearchRequest,
    config: &SearchConfig,
    snapshot: &mut SearchSnapshot,
    sender: &watch::Sender<Arc<SearchSnapshot>>,
) {
    let mut rows = index.results(request);
    snapshot.truncated = rows.len() > config.max_results
        || snapshot.providers.iter().any(|p| {
            matches!(
                p.status,
                ProviderStatus::Complete {
                    truncated: true,
                    ..
                }
            )
        });
    rows.truncate(config.max_results);
    snapshot.results = rows;
    snapshot.partial = snapshot.providers.iter().any(|p| {
        matches!(
            p.status,
            ProviderStatus::Failed { .. }
                | ProviderStatus::TimedOut
                | ProviderStatus::DeadlineExceeded
                | ProviderStatus::Cancelled
        ) || matches!(p.status,ProviderStatus::Complete { rejected,.. } if rejected>0)
    });
    sender.send_replace(Arc::new(snapshot.clone()));
}
