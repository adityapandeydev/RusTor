use rustor_torrent::{Error, TorrentId, TorrentInput};

#[test]
fn accepts_v1_magnets_and_bare_hashes_without_losing_trackers() {
    let hash = "0123456789ABCDEF0123456789ABCDEF01234567";
    let magnet = format!("magnet:?xt=urn:btih:{hash}&tr=https%3A%2F%2Ftracker.example%2Fannounce");
    let input = TorrentInput::parse(&magnet).unwrap();
    assert_eq!(input.id().as_str(), hash.to_ascii_lowercase());
    assert_eq!(input.magnet(), Some(magnet.as_str()));
    assert_eq!(TorrentInput::parse(hash).unwrap().id(), input.id());
}

#[test]
fn normalizes_base32_hashes_to_the_same_identity() {
    let input = TorrentInput::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    assert_eq!(
        input.id().as_str(),
        "0000000000000000000000000000000000000000"
    );
}

#[test]
fn rejects_non_torrent_inputs_and_malformed_magnets() {
    for value in [
        "",
        "some movie",
        "https://example.com/file.torrent",
        "magnet:?xt=urn:btih:nope",
    ] {
        assert!(matches!(
            TorrentInput::parse(value),
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(TorrentId::parse("../outside").is_err());
}

#[test]
fn rejects_empty_malformed_and_oversized_metainfo() {
    for bytes in [
        vec![],
        b"not bencode".to_vec(),
        vec![0; 16 * 1024 * 1024 + 1],
    ] {
        assert!(matches!(
            TorrentInput::from_bytes(bytes),
            Err(Error::InvalidInput(_))
        ));
    }
}

#[test]
fn rejects_unbounded_file_selection_before_entering_the_native_parser() {
    let magnet =
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&so=0-18446744073709551615";
    assert!(matches!(
        TorrentInput::parse(magnet),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn identity_parser_cannot_be_used_to_inject_magnet_parameters() {
    assert!(TorrentId::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&so=0-10").is_err());
}
