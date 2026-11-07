//! Process-local successful provider responses. No queries or rows are written to disk.
use crate::{Candidate, SearchQuery};
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::time::Instant;
const TTL: Duration = Duration::from_secs(300);
const MAX_ENTRIES: usize = 100;
const MAX_BYTES: usize = 16 * 1024 * 1024;
struct Entry {
    source: String,
    query: String,
    at: Instant,
    rows: Arc<[Candidate]>,
    bytes: usize,
}
#[derive(Default)]
pub(crate) struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}
impl Cache {
    fn prune(&mut self) {
        self.entries.retain(|entry| entry.at.elapsed() < TTL);
        self.bytes = self.entries.iter().map(|entry| entry.bytes).sum();
    }
    pub(crate) fn get(&mut self, source: &str, query: &SearchQuery) -> Option<Arc<[Candidate]>> {
        self.prune();
        let key = query.as_str().to_lowercase();
        self.entries
            .iter()
            .find(|entry| entry.source == source && entry.query == key)
            .map(|entry| entry.rows.clone())
    }
    pub(crate) fn insert(
        &mut self,
        source: &str,
        query: &SearchQuery,
        rows: &[Candidate],
        cap: usize,
    ) {
        self.prune();
        let query = query.as_str().to_lowercase();
        // Keep one overflow row so a hit preserves the service's truncation report.
        let rows = &rows[..rows.len().min(cap.saturating_add(1))];
        let mut bytes = std::mem::size_of::<Entry>()
            .saturating_add(source.len())
            .saturating_add(query.len())
            .saturating_add(std::mem::size_of_val(rows));
        for row in rows {
            bytes = bytes
                .saturating_add(row.info_hash.len())
                .saturating_add(row.name.len())
                .saturating_add(row.magnet.len());
            if bytes > MAX_BYTES {
                return;
            }
        }
        if bytes > MAX_BYTES {
            return;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.source == source && entry.query == query)
        {
            self.bytes -= self.entries.remove(index).expect("located entry").bytes;
        }
        self.entries.push_back(Entry {
            source: source.to_owned(),
            query,
            at: Instant::now(),
            rows: rows.to_vec().into(),
            bytes,
        });
        self.bytes += bytes;
        // Fetch order, not hit order: reading never extends freshness or retention.
        while self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            if let Some(entry) = self.entries.pop_front() {
                self.bytes -= entry.bytes;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn query(value: &str) -> SearchQuery {
        SearchQuery::new(value).unwrap()
    }
    fn row() -> Candidate {
        Candidate {
            info_hash: "1".repeat(40),
            name: "Fixture".into(),
            magnet: format!("magnet:?xt=urn:btih:{}", "1".repeat(40)),
            size_bytes: None,
            seeders: None,
            leechers: None,
            file_count: None,
            added_unix_ms: None,
            artwork_url: None,
        }
    }
    #[test]
    fn evicts_oldest_fetch_and_keeps_sources_separate() {
        let mut cache = Cache::default();
        for i in 0..MAX_ENTRIES {
            cache.insert("source", &query(&format!("query{i}")), &[row()], 100);
        }
        assert!(cache.get("source", &query("query0")).is_some());
        assert!(cache.get("other", &query("query0")).is_none());
        cache.insert("source", &query("new"), &[row()], 100);
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.get("source", &query("query0")).is_none());
        assert!(cache.get("source", &query("query1")).is_some());
    }
    #[test]
    fn bounds_accounted_storage_and_preserves_one_overflow_row() {
        let mut cache = Cache::default();
        cache.insert("source", &query("small"), &[], 100);
        let mut huge = row();
        huge.name = "x".repeat(MAX_BYTES + 1);
        cache.insert("source", &query("oversized"), &[huge], 100);
        assert!(cache.get("source", &query("oversized")).is_none());
        assert!(cache.get("source", &query("small")).is_some());
        let mut large = row();
        large.magnet = "x".repeat(100_000);
        for i in 0..30 {
            cache.insert(
                "source",
                &query(&format!("large{i}")),
                &vec![large.clone(); 20],
                100,
            );
            assert!(cache.bytes <= MAX_BYTES);
        }
        cache.insert("source", &query("cap"), &vec![row(); 150], 100);
        assert_eq!(cache.get("source", &query("cap")).unwrap().len(), 101);
    }
    #[tokio::test(start_paused = true)]
    async fn expires_and_prunes_entries_at_the_ttl_boundary() {
        let mut cache = Cache::default();
        cache.insert("source", &query("fixture"), &[row()], 100);
        tokio::time::advance(TTL).await;
        assert!(cache.get("source", &query("fixture")).is_none());
        assert!(cache.entries.is_empty());
        assert_eq!(cache.bytes, 0);
    }
}
