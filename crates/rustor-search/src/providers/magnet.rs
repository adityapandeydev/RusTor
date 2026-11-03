pub(super) const TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://tracker.openbittorrent.com:6969/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://exodus.desync.com:6969/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.dler.org:6969/announce",
    "http://tracker.opentrackr.org:1337/announce",
    "http://tracker.openbittorrent.com:80/announce",
    "http://tracker.dler.org:6969/announce",
    "https://tracker.tamersunion.org:443/announce",
];

pub(super) fn build(hash: &str, name: &str) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("xt", &format!("urn:btih:{hash}"))
        .append_pair("dn", name);
    for tracker in TRACKERS {
        query.append_pair("tr", tracker);
    }
    format!("magnet:?{}", query.finish())
}
