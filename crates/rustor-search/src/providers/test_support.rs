use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
pub(super) async fn server(
    count: usize,
    media: &'static str,
    reply: impl Fn(&url::Url) -> (u16, String) + Send + 'static,
) -> (url::Url, JoinHandle<Vec<url::Url>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let origin = base.clone();
    let task = tokio::spawn(async move {
        let mut seen = vec![];
        for _ in 0..count {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = vec![];
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
                assert!(request.len() < 16_384);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = origin.join(path).unwrap();
            let (status, body) = reply(&url);
            seen.push(url);
            let wire = format!(
                "HTTP/1.1 {status} Test\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(wire.as_bytes()).await.unwrap();
        }
        seen
    });
    (base, task)
}
