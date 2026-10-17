//! The HTTP client the app gives GPUI, so an image with an http(s) address in a Markdown file is
//! fetched (C-011). GPUI itself ships none: without this every remote request fails at once. Each
//! request runs a small blocking client on a thread of its own and hands the response back to the
//! caller's future.

use std::sync::Arc;

use futures::AsyncReadExt as _;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use gpui_kit::http_client::http::{self, HeaderValue};
use gpui_kit::http_client::{AsyncBody, HttpClient, Response, Url};

/// A blocking `ureq` agent behind GPUI's `HttpClient` trait.
pub struct UreqClient {
    agent: ureq::Agent,
    user_agent: HeaderValue,
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqClient {
    pub fn new() -> Self {
        // A 404 is an answer, not a failure: the image element shows nothing for it instead of an error.
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build();
        Self { agent: config.into(), user_agent: HeaderValue::from_static("Sfoglia") }
    }

    /// The client as GPUI takes it.
    pub fn shared() -> Arc<dyn HttpClient> {
        Arc::new(Self::new())
    }
}

impl HttpClient for UreqClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(&self, request: http::Request<AsyncBody>) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let agent = self.agent.clone();
        let user_agent = self.user_agent.clone();
        Box::pin(async move {
            let (parts, mut body) = request.into_parts();
            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes).await?;
            let (tx, rx) = oneshot::channel();
            std::thread::Builder::new()
                .name("sfoglia-http".into())
                .spawn(move || {
                    let _ = tx.send(fetch(&agent, parts, bytes, user_agent));
                })?;
            rx.await.map_err(|_| anyhow::anyhow!("the request thread ended without an answer"))?
        })
    }
}

/// One request, blocking, on the request thread.
fn fetch(
    agent: &ureq::Agent,
    parts: http::request::Parts,
    body: Vec<u8>,
    user_agent: HeaderValue,
) -> anyhow::Result<Response<AsyncBody>> {
    let mut request = http::Request::builder().method(parts.method).uri(parts.uri);
    for (name, value) in &parts.headers {
        request = request.header(name, value);
    }
    if !parts.headers.contains_key(http::header::USER_AGENT) {
        request = request.header(http::header::USER_AGENT, user_agent);
    }
    let response = agent.run(request.body(body)?)?;
    let (head, mut ureq_body) = response.into_parts();
    let bytes = ureq_body.read_to_vec()?;
    let mut out = Response::builder().status(head.status).version(head.version);
    for (name, value) in &head.headers {
        out = out.header(name, value);
    }
    Ok(out.body(AsyncBody::from(bytes))?)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// Serve one HTTP answer on a port of the system's choosing; the request line is handed back.
    fn serve_once(status: &'static str, content_type: &'static str, body: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
                head.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&head).to_string();
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
            request
        });
        (format!("http://127.0.0.1:{port}/dot.png"), handle)
    }

    #[test]
    fn remote_images_are_fetched_through_the_apps_http_client() {
        let png = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dot.png")).unwrap();
        let client = UreqClient::new();
        let (url, server) = serve_once("200 OK", "image/png", png.clone());
        let mut response = futures::executor::block_on(client.get(&url, AsyncBody::empty(), true)).unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers().get("content-type").unwrap(), "image/png");
        let mut bytes = Vec::new();
        futures::executor::block_on(response.body_mut().read_to_end(&mut bytes)).unwrap();
        assert_eq!(bytes, png, "the exact bytes the server sent");
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /dot.png HTTP/1.1\r\n"), "{request}");
        assert!(request.to_ascii_lowercase().contains("user-agent: sfoglia"), "{request}");

        // A missing image is an answer with its status, not a failure.
        let (url, server) = serve_once("404 Not Found", "text/plain", b"gone".to_vec());
        let response = futures::executor::block_on(client.get(&url, AsyncBody::empty(), true)).unwrap();
        assert_eq!(response.status(), 404);
        server.join().unwrap();

        // A closed port is a failure, reported, not a hang.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = futures::executor::block_on(client.get(&format!("http://127.0.0.1:{port}/x.png"), AsyncBody::empty(), true));
        assert!(error.is_err());
    }
}
