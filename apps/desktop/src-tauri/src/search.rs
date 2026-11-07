use crate::service::CommandError;
use rustor_search::{
    SearchConfig, SearchHandle, SearchQuery, SearchRequest, SearchService, SearchSnapshot,
    providers::desktop_providers,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;

/// One active search and one bounded latest snapshot for this desktop session.
/// The UI submits query text, never provider URLs or raw result rows.
pub struct DesktopSearch {
    service: Result<SearchService, CommandError>,
    active: Mutex<Option<SearchHandle>>,
    stopped: AtomicBool,
}
impl DesktopSearch {
    pub fn new() -> Self {
        let service = desktop_providers()
            .and_then(|providers| {
                SearchService::new(
                    providers,
                    SearchConfig {
                        provider_timeout: std::time::Duration::from_secs(45),
                        overall_timeout: std::time::Duration::from_secs(90),
                        ..SearchConfig::default()
                    },
                )
            })
            .map_err(|_| {
                CommandError::new(
                    "search_unavailable",
                    "Search could not initialize. Restart RusTor to try again.",
                )
            });
        Self {
            service,
            active: Mutex::new(None),
            stopped: AtomicBool::new(false),
        }
    }
    #[cfg(test)]
    fn with_providers(providers: Vec<Arc<dyn rustor_search::Provider>>) -> Self {
        Self {
            service: Ok(SearchService::new(providers, SearchConfig::default()).unwrap()),
            active: Mutex::new(None),
            stopped: AtomicBool::new(false),
        }
    }
    pub async fn start(
        &self,
        query: String,
        category: Option<rustor_search::Category>,
    ) -> Result<Arc<SearchSnapshot>, CommandError> {
        let query = SearchQuery::new(&query).map_err(|_| {
            CommandError::new(
                "invalid_query",
                "Use at most 256 bytes of search text without control characters.",
            )
        })?;
        let mut active = self.active.lock().await;
        if self.stopped.load(Ordering::Acquire) {
            return Err(CommandError::new("stopped", "RusTor is shutting down."));
        }
        let handle = self
            .service
            .as_ref()
            .map_err(Clone::clone)?
            .start({
                let mut request = SearchRequest::new(query);
                request.category = category;
                request
            })
            .map_err(|_| {
                CommandError::new("search_unavailable", "Search is unavailable. Try again.")
            })?;
        let snapshot = handle.snapshot();
        *active = Some(handle);
        Ok(snapshot)
    }
    pub async fn snapshot(&self, id: u64) -> Result<Arc<SearchSnapshot>, CommandError> {
        let active = self.active.lock().await;
        let handle = active
            .as_ref()
            .filter(|handle| handle.snapshot().id == id)
            .ok_or_else(|| {
                CommandError::new(
                    "search_expired",
                    "This search has been replaced. Search again.",
                )
            })?;
        Ok(handle.snapshot())
    }
    pub async fn cancel(&self, id: u64) -> bool {
        let active = self.active.lock().await;
        if let Some(handle) = active.as_ref().filter(|handle| handle.snapshot().id == id) {
            handle.cancel();
            return true;
        }
        false
    }
    pub async fn artwork_result(&self, hash: &str) -> Option<rustor_search::SearchResult> {
        let active = self.active.lock().await;
        let snapshot = active.as_ref()?.snapshot();
        snapshot
            .results
            .iter()
            .find(|row| row.info_hash.as_str() == hash)
            .cloned()
    }
    pub async fn shutdown(&self) {
        let mut active = self.active.lock().await;
        self.stopped.store(true, Ordering::Release);
        active.take(); // Dropping the handle cancels and aborts provider work.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustor_search::{Candidate, Provider, ProviderDescriptor, ProviderFuture, SearchPhase};
    struct Pending(ProviderDescriptor);
    impl Provider for Pending {
        fn descriptor(&self) -> &ProviderDescriptor {
            &self.0
        }
        fn search<'a>(&'a self, _: &'a SearchQuery) -> ProviderFuture<'a> {
            Box::pin(std::future::pending::<Result<Vec<Candidate>, _>>())
        }
    }
    #[tokio::test]
    async fn newer_search_replaces_old_handle_and_stale_cancel_is_ignored() {
        let provider = Arc::new(Pending(
            ProviderDescriptor::new("fixture", "Fixture", vec![], true).unwrap(),
        ));
        let search = DesktopSearch::with_providers(vec![provider]);
        let first = search.start("first".into(), None).await.unwrap();
        assert_eq!(
            search.start("x".repeat(257), None).await.unwrap_err().code,
            "invalid_query"
        );
        assert_eq!(search.snapshot(first.id).await.unwrap().id, first.id);
        let second = search.start("second".into(), None).await.unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(
            search.snapshot(first.id).await.unwrap_err().code,
            "search_expired"
        );
        assert!(!search.cancel(first.id).await);
        assert_eq!(
            search.snapshot(second.id).await.unwrap().phase,
            SearchPhase::Running
        );
        assert!(search.cancel(second.id).await);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while search.snapshot(second.id).await.unwrap().phase == SearchPhase::Running {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            search.snapshot(second.id).await.unwrap().phase,
            SearchPhase::Cancelled
        );
        search.shutdown().await;
        assert_eq!(
            search
                .start("after shutdown".into(), None)
                .await
                .unwrap_err()
                .code,
            "stopped"
        );
        assert_eq!(
            search.snapshot(second.id).await.unwrap_err().code,
            "search_expired"
        );
    }
}
