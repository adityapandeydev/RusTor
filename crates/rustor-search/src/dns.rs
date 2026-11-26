//! Encrypted (DNS-over-HTTPS) name lookup for fixed provider and artwork origins.
//!
//! Some ISPs answer torrent-index host names with a sinkhole address, so the
//! system resolver makes a reachable site look offline. Lookups go to public
//! DoH services addressed by IP literal, so no plaintext DNS is involved.
//! Only fixed origin host names are sent; never queries, magnets or paths.
//! If every DoH service is unreachable, the system resolver is used instead.
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde_json::Value;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::time::Instant;

/// Cloudflare first, Google as an independent fallback. Both serve JSON DoH on
/// IP literals covered by their certificates.
const ENDPOINTS: [&str; 4] = [
    "https://1.1.1.1/dns-query",
    "https://1.0.0.1/dns-query",
    "https://8.8.8.8/resolve",
    "https://8.8.4.4/resolve",
];
const MAX_BODY: usize = 64 * 1024;
const MAX_ENTRIES: usize = 64;
const MAX_ADDRESSES: usize = 16;
// Four endpoint attempts must leave room for system DNS and the provider's
// connection within the HTTP clients' four-second connect timeout.
const ENDPOINT_TIMEOUT: Duration = Duration::from_millis(600);
const MIN_TTL: u64 = 60;
const MAX_TTL: u64 = 3600;
const A: u64 = 1;
const AAAA: u64 = 28;

type Error = Box<dyn std::error::Error + Send + Sync>;
/// Expiry and the resolved public addresses.
type Cached = (Instant, Arc<[IpAddr]>);

/// Cheap handle; clones share one cache.
#[derive(Clone)]
pub(crate) struct DohResolver(Arc<Inner>);

struct Inner {
    client: Option<reqwest::Client>,
    cache: Mutex<HashMap<String, Cached>>,
}

/// One process-wide resolver so provider and artwork clients share its cache.
pub(crate) fn shared() -> DohResolver {
    static RESOLVER: OnceLock<DohResolver> = OnceLock::new();
    RESOLVER
        .get_or_init(|| DohResolver(Arc::new(Inner::new())))
        .clone()
}

/// A resolved answer from one DoH service.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// The name exists with these public addresses (and the shortest TTL).
    Found(Vec<IpAddr>, u64),
    /// The service answered authoritatively that no usable address exists.
    Missing,
}

impl Inner {
    fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(5))
                .user_agent(concat!("RusTor/", env!("CARGO_PKG_VERSION")))
                .build()
                .ok(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn cached(&self, host: &str) -> Option<Arc<[IpAddr]>> {
        let cache = self.cache.lock().ok()?;
        cache
            .get(host)
            .filter(|(expires, _)| *expires > Instant::now())
            .map(|(_, addresses)| addresses.clone())
    }

    fn remember(&self, host: &str, addresses: Arc<[IpAddr]>, ttl: u64) {
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        let now = Instant::now();
        cache.retain(|_, (expires, _)| *expires > now);
        if cache.len() >= MAX_ENTRIES && !cache.contains_key(host) {
            return;
        }
        let ttl = Duration::from_secs(ttl.clamp(MIN_TTL, MAX_TTL));
        cache.insert(host.to_owned(), (now + ttl, addresses));
    }

    async fn query(&self, endpoint: &str, host: &str, kind: u64) -> Result<Answer, Error> {
        let client = self.client.as_ref().ok_or("DoH client unavailable")?;
        let mut url = url::Url::parse(endpoint)?;
        url.query_pairs_mut()
            .append_pair("name", host)
            .append_pair("type", if kind == A { "A" } else { "AAAA" });
        let response = client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/dns-json")
            .send()
            .await?;
        let body = crate::artwork::read_response(
            response,
            &[
                "application/dns-json",
                "application/json",
                "application/x-javascript",
            ],
            MAX_BODY,
        )
        .await
        .ok_or("invalid DoH response")?;
        parse(&body, kind).ok_or_else(|| "invalid DoH answer".into())
    }

    /// Try each service; prefer IPv4 and fall back to IPv6 records.
    async fn lookup(&self, host: &str) -> Result<Answer, Error> {
        let mut last: Error = "no DoH service answered".into();
        for endpoint in ENDPOINTS {
            let lookup = async {
                match self.query(endpoint, host, A).await? {
                    answer @ Answer::Found(..) => Ok(answer),
                    Answer::Missing => self.query(endpoint, host, AAAA).await,
                }
            };
            match tokio::time::timeout(ENDPOINT_TIMEOUT, lookup).await {
                Ok(Ok(answer)) => return Ok(answer),
                Ok(Err(error)) => {
                    last = error;
                }
                Err(error) => last = error.into(),
            }
        }
        Err(last)
    }

    async fn resolve_host(&self, host: String) -> Result<Arc<[IpAddr]>, Error> {
        if let Some(addresses) = self.cached(&host) {
            return Ok(addresses);
        }
        match self.lookup(&host).await {
            Ok(Answer::Found(addresses, ttl)) => {
                let addresses: Arc<[IpAddr]> = addresses.into();
                self.remember(&host, addresses.clone(), ttl);
                Ok(addresses)
            }
            Ok(Answer::Missing) => Err(format!("{host} has no public address").into()),
            // Keep the public-address constraint when encrypted DNS is unreachable.
            Err(_) => {
                let addresses = tokio::net::lookup_host((host.as_str(), 0))
                    .await?
                    .map(|address| address.ip())
                    .filter(|ip| public(*ip))
                    .take(MAX_ADDRESSES)
                    .collect::<Vec<_>>();
                if addresses.is_empty() {
                    return Err(format!("{host} could not be resolved").into());
                }
                Ok(addresses.into())
            }
        }
    }
}

impl Resolve for DohResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().trim_end_matches('.').to_ascii_lowercase();
        let resolver = self.0.clone();
        Box::pin(async move {
            let addresses = resolver.resolve_host(host).await?;
            let addrs: Addrs = Box::new(
                addresses
                    .iter()
                    .map(|ip| SocketAddr::new(*ip, 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addrs)
        })
    }
}

/// Parse a JSON DoH answer. `None` means the response is unusable and the next
/// service should be tried; NXDOMAIN or an empty answer is `Missing`.
fn parse(body: &[u8], kind: u64) -> Option<Answer> {
    let root: Value = serde_json::from_slice(body).ok()?;
    match root.get("Status")?.as_u64()? {
        0 => {}
        3 => return Some(Answer::Missing),
        _ => return None,
    }
    let mut ttl = MAX_TTL;
    let mut addresses = Vec::new();
    for record in root
        .get("Answer")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if record.get("type").and_then(Value::as_u64) != Some(kind) {
            continue; // CNAME chains precede the final address records.
        }
        let Some(ip) = record
            .get("data")
            .and_then(Value::as_str)
            .and_then(|data| data.parse::<IpAddr>().ok())
        else {
            continue;
        };
        if (kind == A) != ip.is_ipv4() || !public(ip) || addresses.contains(&ip) {
            continue;
        }
        ttl = ttl.min(record.get("TTL").and_then(Value::as_u64).unwrap_or(MIN_TTL));
        addresses.push(ip);
        if addresses.len() == MAX_ADDRESSES {
            break;
        }
    }
    Some(if addresses.is_empty() {
        Answer::Missing
    } else {
        Answer::Found(addresses, ttl)
    })
}

/// Fixed public origins never legitimately resolve to local or private networks.
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1])))
        }
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || ip.to_ipv4_mapped().is_some())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_final_address_records_and_shortest_ttl() {
        let body = br#"{"Status":0,"Answer":[
            {"name":"www.example.org","type":5,"TTL":30,"data":"example.org."},
            {"name":"example.org","type":1,"TTL":300,"data":"104.21.68.109"},
            {"name":"example.org","type":1,"TTL":96,"data":"172.67.194.150"},
            {"name":"example.org","type":1,"TTL":96,"data":"172.67.194.150"}]}"#;
        assert_eq!(
            parse(body, A),
            Some(Answer::Found(
                vec![
                    "104.21.68.109".parse().unwrap(),
                    "172.67.194.150".parse().unwrap()
                ],
                96
            ))
        );
    }

    #[test]
    fn distinguishes_missing_names_from_unusable_responses() {
        assert_eq!(parse(br#"{"Status":3}"#, A), Some(Answer::Missing));
        assert_eq!(parse(br#"{"Status":0}"#, A), Some(Answer::Missing));
        assert_eq!(parse(br#"{"Status":2}"#, A), None);
        assert_eq!(parse(b"not json", A), None);
        assert_eq!(parse(br#"{"Answer":[]}"#, A), None);
    }

    #[test]
    fn rejects_private_mismatched_and_malformed_addresses() {
        let body = br#"{"Status":0,"Answer":[
            {"type":1,"TTL":60,"data":"127.0.0.1"},
            {"type":1,"TTL":60,"data":"192.168.29.1"},
            {"type":1,"TTL":60,"data":"10.0.0.8"},
            {"type":1,"TTL":60,"data":"100.64.0.1"},
            {"type":1,"TTL":60,"data":"::1"},
            {"type":1,"TTL":60,"data":"not an address"}]}"#;
        assert_eq!(parse(body, A), Some(Answer::Missing));
        let v6 = br#"{"Status":0,"Answer":[
            {"type":28,"TTL":120,"data":"fe80::1"},
            {"type":28,"TTL":120,"data":"fd00::1"},
            {"type":28,"TTL":120,"data":"2606:4700:3033::ac43:898f"}]}"#;
        assert_eq!(
            parse(v6, AAAA),
            Some(Answer::Found(
                vec!["2606:4700:3033::ac43:898f".parse().unwrap()],
                120
            ))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cache_respects_bounded_ttls() {
        let resolver = Inner::new();
        let addresses: Arc<[IpAddr]> = vec!["104.21.68.109".parse().unwrap()].into();
        resolver.remember("example.org", addresses.clone(), 1);
        assert!(resolver.cached("example.org").is_some());
        tokio::time::advance(Duration::from_secs(MIN_TTL)).await;
        assert!(resolver.cached("example.org").is_none());
        for i in 0..MAX_ENTRIES + 8 {
            resolver.remember(&format!("host{i}.example"), addresses.clone(), MAX_TTL * 10);
        }
        assert_eq!(resolver.cache.lock().unwrap().len(), MAX_ENTRIES);
    }

    #[tokio::test]
    async fn system_fallback_rejects_loopback() {
        let mut resolver = Inner::new();
        resolver.client = None;
        assert!(resolver.resolve_host("localhost".into()).await.is_err());
    }

    #[tokio::test]
    #[ignore = "contacts public DoH services"]
    async fn live_resolves_a_public_name_over_https() {
        let addresses = shared().0.resolve_host("nyaa.si".into()).await.unwrap();
        assert!(!addresses.is_empty());
        println!("nyaa.si -> {addresses:?}");
    }
}
