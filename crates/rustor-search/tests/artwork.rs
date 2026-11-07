use rustor_search::artwork::artwork_url;

#[tokio::test]
#[ignore = "explicit real metadata and image check; requires Internet"]
async fn public_metadata_supplies_movie_tv_and_anime_covers() {
    use rustor_search::*;
    let directory = tempfile::tempdir().unwrap();
    let cache = rustor_search::artwork::ArtworkCache::new(directory.path().to_owned());
    for (id, provider, title) in [
        (1, "tpb-movies", "The.Dark.Knight.2008.1080p.BluRay"),
        (2, "tpb-tv", "Breaking.Bad.S01E01.720p"),
        (3, "nyaa", "[Group] One Piece - 1100 [1080p]"),
    ] {
        let hash = format!("{id:040x}");
        let mut index = ResultIndex::default();
        index
            .insert(
                &ProviderDescriptor::new(provider, provider, vec![], false).unwrap(),
                Candidate {
                    info_hash: hash.clone(),
                    name: title.into(),
                    magnet: format!("magnet:?xt=urn:btih:{hash}"),
                    size_bytes: None,
                    seeders: None,
                    leechers: None,
                    file_count: None,
                    added_unix_ms: None,
                    artwork_url: None,
                },
            )
            .unwrap();
        let row = index
            .results(&SearchRequest::new(SearchQuery::new("").unwrap()))
            .remove(0);
        let image = cache.get_for_result(&row).await.expect(provider);
        assert!(image.starts_with("data:image/png;base64,"));
        assert_eq!(Some(image), cache.get(&hash, None).await);
    }
}

#[tokio::test]
#[ignore = "explicit real-provider artwork check; requires Internet"]
async fn provider_artwork_downloads_and_survives_a_new_cache_instance() {
    let source = std::env::var("RUSTOR_TEST_ARTWORK_URL").expect("set the provider image URL");
    let dir = tempfile::tempdir().unwrap();
    let hash = "c".repeat(40);
    let cache = rustor_search::artwork::ArtworkCache::new(dir.path().to_owned());
    let result = cache
        .get(&hash, Some(&source))
        .await
        .expect("provider image should decode");
    let reopened = rustor_search::artwork::ArtworkCache::new(dir.path().to_owned());
    assert_eq!(result, reopened.get(&hash, None).await.unwrap());
}

#[test]
fn artwork_is_limited_to_the_providers_image_origins_and_paths() {
    assert_eq!(
        artwork_url(
            "yts",
            "https://yts.rs/images/images/tt1361349/medium_cover_image.jpg"
        )
        .as_deref(),
        Some("https://yts.rs/images/images/tt1361349/medium_cover_image.jpg")
    );
    assert!(
        artwork_url(
            "yts",
            "https://yts.mx/assets/images/movies/fixture/medium-cover.jpg"
        )
        .is_some()
    );
    assert_eq!(
        artwork_url("subsplease", "/wp-content/uploads/2026/01/cover.jpg").as_deref(),
        Some("https://subsplease.org/wp-content/uploads/2026/01/cover.jpg")
    );
    assert!(artwork_url("fitgirl", "https://i3.imageban.ru/out/2026/01/cover.jpg").is_some());
    for value in [
        "http://subsplease.org/wp-content/uploads/a.jpg",
        "https://127.0.0.1/a.jpg",
        "https://subsplease.org.evil.test/wp-content/uploads/a.jpg",
        "https://user@subsplease.org/wp-content/uploads/a.jpg",
        "https://subsplease.org:444/wp-content/uploads/a.jpg",
        "https://subsplease.org/api/",
        "data:image/svg+xml,evil",
        "https://subsplease.org/wp-content/uploads/a.jpg?tracking=1",
    ] {
        assert!(artwork_url("subsplease", value).is_none(), "{value}");
    }
    assert!(artwork_url("nyaa", "https://subsplease.org/wp-content/uploads/a.jpg").is_none());
}
