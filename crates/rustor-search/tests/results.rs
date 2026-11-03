use rustor_search::*;

fn candidate(hash: &str, name: &str, seeders: Option<u32>, tracker: &str) -> Candidate {
    Candidate {
        info_hash: hash.into(),
        name: name.into(),
        magnet: format!("magnet:?xt=urn:btih:{hash}&dn=Keep%20This&so=0&tr={tracker}"),
        seeders,
        leechers: None,
        size_bytes: Some(1024),
        file_count: None,
        added_unix_ms: None,
    }
}
fn provider(id: &str, health: bool) -> ProviderDescriptor {
    ProviderDescriptor::new(id, id, vec![Category::Movies], health).unwrap()
}
const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn merge_keeps_winning_magnet_extensions_trackers_and_missing_metadata() {
    let mut index = ResultIndex::default();
    let mut lower = candidate(HASH, "Synthetic fixture", Some(3), "udp://low.example:80");
    lower.file_count = Some(2);
    lower.added_unix_ms = Some(123);
    let higher = candidate(
        &HASH.to_uppercase(),
        "Synthetic fixture winner",
        Some(9),
        "https://high.example/announce",
    );
    let original = higher.magnet.clone();
    index.insert(&provider("a", true), lower).unwrap();
    index.insert(&provider("b", true), higher).unwrap();
    let rows = index.results(&SearchRequest::new(SearchQuery::new("").unwrap()));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].info_hash.as_str(), HASH);
    assert_eq!(rows[0].seeders, Some(9));
    assert_eq!(rows[0].file_count, Some(2));
    assert_eq!(rows[0].added_unix_ms, Some(123));
    assert!(rows[0].magnet.starts_with(&original));
    let url = url::Url::parse(&rows[0].magnet).unwrap();
    assert!(url.query_pairs().any(|(k, v)| k == "so" && v == "0"));
    assert_eq!(url.query_pairs().filter(|(k, _)| k == "tr").count(), 2);
    assert_eq!(rows[0].providers, vec!["a", "b"]);
}

#[test]
fn merge_is_independent_of_provider_completion_order_and_preserves_unknown_health() {
    let mut first = ResultIndex::default();
    let mut second = ResultIndex::default();
    let mut a = candidate(HASH, "Alpha fixture", Some(0), "udp://a.example:80");
    a.file_count = Some(1);
    let b = candidate(HASH, "Beta fixture", Some(0), "udp://b.example:80");
    for (index, reversed) in [(&mut first, false), (&mut second, true)] {
        let mut entries = vec![
            (provider("a", true), a.clone()),
            (provider("b", false), b.clone()),
        ];
        if reversed {
            entries.reverse();
        }
        for (source, row) in entries {
            index.insert(&source, row).unwrap();
        }
    }
    let mut request = SearchRequest::new(SearchQuery::new("").unwrap());
    request.hide_dead = true;
    assert_eq!(first.results(&request), second.results(&request));
    assert_eq!(first.results(&request).len(), 1);
    assert!(first.results(&request)[0].has_unknown_health);
}

#[test]
fn validates_identity_magnets_and_search_text() {
    let base32 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    assert_eq!(
        InfoHash::parse(base32).unwrap().as_str(),
        "0000000000000000000000000000000000000000"
    );
    assert!(InfoHash::parse("not-a-hash").is_err());
    assert!(InfoHash::parse(&format!("{}{HASH}", " ".repeat(1024))).is_err());
    assert!(SearchQuery::new(&"x".repeat(257)).is_err());
    assert!(SearchQuery::new("a\0b").is_err());
    assert_eq!(
        SearchQuery::new("  demo   query ").unwrap().as_str(),
        "demo query"
    );
    let mut index = ResultIndex::default();
    let mut invalid = candidate(HASH, "name", Some(1), "udp://ok.example:80");
    invalid.magnet = "https://example.com/file.torrent".into();
    assert!(index.insert(&provider("a", true), invalid).is_err());
    let mut mismatch = candidate(HASH, "name", Some(1), "udp://ok.example:80");
    mismatch.info_hash = "0000000000000000000000000000000000000000".into();
    assert!(index.insert(&provider("a", true), mismatch).is_err());
    assert!(
        index
            .results(&SearchRequest::new(SearchQuery::new("").unwrap()))
            .is_empty()
    );
}

#[test]
fn token_filter_and_sort_keep_unknown_counts_distinct_from_dead_results() {
    let mut index = ResultIndex::default();
    for (i, name, seeders, added) in [
        (1, "demo query exact", Some(0), 100),
        (2, "query demo reversed", Some(9), 200),
        (3, "demo middle query", None, 300),
        (4, "other", Some(99), 400),
    ] {
        let hash = format!("{i:040x}");
        let mut c = candidate(&hash, name, seeders, "udp://t.example:80");
        c.added_unix_ms = Some(added);
        index.insert(&provider("a", true), c).unwrap();
    }
    let mut request = SearchRequest::new(SearchQuery::new("").unwrap());
    request.text_filter = SearchQuery::new("demo query").unwrap();
    let rows = index.results(&request);
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec![
            "demo query exact",
            "demo middle query",
            "query demo reversed"
        ]
    );
    request.hide_dead = true;
    let rows = index.results(&request);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|r| r.seeders.is_none()));
}

#[test]
fn explicit_size_sort_overrides_text_relevance_and_keeps_unknown_sizes_last() {
    let mut index = ResultIndex::default();
    for (i, name, size) in [
        (1, "demo query", Some(900)),
        (2, "query demo", Some(100)),
        (3, "demo query unknown", None),
    ] {
        let hash = format!("{i:040x}");
        let mut row = candidate(&hash, name, Some(1), "udp://t.example:80");
        row.size_bytes = size;
        index.insert(&provider("a", true), row).unwrap();
    }
    let mut request = SearchRequest::new(SearchQuery::new("").unwrap());
    request.text_filter = SearchQuery::new("demo query").unwrap();
    request.sort_field = SortField::Size;
    request.sort_direction = SortDirection::Ascending;
    assert_eq!(
        index
            .results(&request)
            .iter()
            .map(|r| r.size_bytes)
            .collect::<Vec<_>>(),
        vec![Some(100), Some(900), None]
    );
}

#[test]
fn base32_aliases_and_invalid_tracker_urls_cannot_create_extra_results() {
    let mut index = ResultIndex::default();
    let zero = "0000000000000000000000000000000000000000";
    let mut alias = candidate(zero, "Synthetic fixture", Some(1), "udp://t.example:80");
    alias.info_hash = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into();
    index.insert(&provider("a", true), alias).unwrap();
    index
        .insert(
            &provider("b", true),
            candidate(zero, "Synthetic fixture", Some(2), "udp://other.example:80"),
        )
        .unwrap();
    let mut invalid = candidate(HASH, "Invalid source", Some(3), "file:///secret");
    assert!(index.insert(&provider("a", true), invalid.clone()).is_err());
    invalid.magnet = format!("magnet:?xt=urn:btih:{HASH}#fragment");
    assert!(index.insert(&provider("a", true), invalid).is_err());
    assert_eq!(
        index
            .results(&SearchRequest::new(SearchQuery::new("").unwrap()))
            .len(),
        1
    );
}
