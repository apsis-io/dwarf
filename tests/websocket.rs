//! End-to-end test for the `WebSocketServer` WASI-backed polyfill
//! (crates/core/polyfills/websocket-server.js, wired in polyfills.rs's
//! `generate_websocket`): a real client (tokio-tungstenite, an independent,
//! well-tested WebSocket implementation - not dwarf's own code) connects
//! over a real TCP socket to a dwarf-built component's `WebSocketServer`,
//! matching this codebase's established cross-testing methodology (verify a
//! ported/reimplemented protocol against a real, canonical implementation
//! rather than trusting the port in isolation).

mod common;

use std::path::PathBuf;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use wasmtime::component::Val;

use common::TestCase;

/// Minimal raw HTTP/1.1 client for testing `WebSocketServer`'s general
/// request router - deliberately not a real HTTP client crate, so the test
/// exercises the actual bytes on the wire (status line, headers,
/// Content-Length-based body framing) rather than trusting a library to
/// paper over a wire-format bug on either side.
struct RawHttpClient {
    stream: TcpStream,
}

struct RawHttpResponse {
    status: u16,
    headers: std::collections::HashMap<String, String>,
    body: Vec<u8>,
}

impl RawHttpClient {
    async fn connect(port: u16) -> Self {
        let stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("should connect");
        Self { stream }
    }

    /// Sends one HTTP/1.1 request and reads exactly one response back off
    /// the same connection (so a caller can send a second request after,
    /// to exercise keep-alive).
    async fn request(&mut self, method: &str, path: &str, keep_alive: bool) -> RawHttpResponse {
        let connection = if keep_alive { "keep-alive" } else { "close" };
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: {connection}\r\n\r\n"
        );
        self.stream
            .write_all(req.as_bytes())
            .await
            .expect("should write request");

        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let n = self
                .stream
                .read(&mut chunk)
                .await
                .expect("should read response");
            assert!(n > 0, "connection closed before headers completed");
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = find_header_end(&buf) {
                break pos;
            }
        };

        let header_text = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let mut lines = header_text.split("\r\n");
        let status_line = lines.next().expect("status line");
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .expect("status code")
            .parse()
            .expect("numeric status code");

        let mut headers = std::collections::HashMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_lowercase(), value.trim().to_string());
            }
        }

        let content_length: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let mut body = buf[header_end..].to_vec();
        while body.len() < content_length {
            let n = self
                .stream
                .read(&mut chunk)
                .await
                .expect("should read body");
            assert!(n > 0, "connection closed before body completed");
            body.extend_from_slice(&chunk[..n]);
        }
        body.truncate(content_length);

        RawHttpResponse {
            status,
            headers,
            body,
        }
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

#[tokio::test]
async fn test_websocket_echo_server() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/websocket");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("websocket-test")
        .polyfills(&["webcrypto"])
        .script(
            r#"
            export async function run(port) {
                const server = new WebSocketServer();
                server.on("connection", (conn) => {
                    conn.on("message", (data) => {
                        if (typeof data === "string") conn.send("echo:" + data);
                        else conn.send(data);
                    });
                });
                await server.listen(port, "127.0.0.1");
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build the websocket component");

    let port: u16 = 18901;
    let (instance, store) = inst.parts();
    let func = instance
        .get_func(&mut *store, "run")
        .expect("run export not found");

    let server_task = async {
        let mut results = [];
        func.call_async(&mut *store, &[Val::U16(port)], &mut results)
            .await
    };

    let client_task = async {
        // Give the server a moment to bind and start its accept loop.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let url = format!("ws://127.0.0.1:{port}/");
        let (mut ws, _response) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("client should connect and complete the handshake");

        ws.send(Message::Text("hello".into()))
            .await
            .expect("should send text message");
        let msg = ws
            .next()
            .await
            .expect("should receive a reply")
            .expect("reply should not be an error");
        assert_eq!(msg.into_text().unwrap(), "echo:hello");

        let binary = vec![1u8, 2, 3, 4, 5];
        ws.send(Message::Binary(binary.clone().into()))
            .await
            .expect("should send binary message");
        let msg = ws
            .next()
            .await
            .expect("should receive a binary reply")
            .expect("reply should not be an error");
        assert_eq!(msg.into_data().to_vec(), binary);

        ws.close(None).await.expect("should close cleanly");
    };

    tokio::select! {
        result = server_task => panic!("server task should not finish first: {result:?}"),
        () = client_task => {}
    }
}

/// The general single-port HTTP+WS router: `WebSocketServer`'s raw-socket
/// HTTP parser also serves plain (non-upgrade) requests to an `on("request",
/// ...)` handler, so one `wasi:sockets` listener can carry both a normal
/// HTTP response path (e.g. SSR) and WebSocket upgrades - needed by hosts
/// that can only reach a component on a single port (see the WebSockets
/// README section). Exercises, against the real bytes on the wire (not a
/// library's own HTTP parsing): a plain GET routed to the handler, HTTP/1.1
/// keep-alive (two requests on one connection), a 404 from the handler's own
/// routing, and that WS upgrades still work on the very same server/port.
#[tokio::test]
async fn test_websocket_server_also_routes_plain_http_requests() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/websocket");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("websocket-test")
        .polyfills(&["webcrypto", "fetch-classes", "url"])
        .script(
            r#"
            export async function run(port) {
                const server = new WebSocketServer();
                server.on("request", async (request) => {
                    const url = new URL(request.url);
                    if (request.method === "GET" && url.pathname === "/hello") {
                        return new Response("hello world", {
                            status: 200,
                            headers: { "X-Test": "yes" },
                        });
                    }
                    return new Response("not found", { status: 404 });
                });
                server.on("connection", (conn) => {
                    conn.on("message", (data) => {
                        if (typeof data === "string") conn.send("echo:" + data);
                        else conn.send(data);
                    });
                });
                await server.listen(port, "127.0.0.1");
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build the websocket+http router component");

    let port: u16 = 18902;
    let (instance, store) = inst.parts();
    let func = instance
        .get_func(&mut *store, "run")
        .expect("run export not found");

    let server_task = async {
        let mut results = [];
        func.call_async(&mut *store, &[Val::U16(port)], &mut results)
            .await
    };

    let client_task = async {
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Two requests on the SAME connection - proves HTTP/1.1 keep-alive,
        // not just "can serve one request then die".
        let mut http = RawHttpClient::connect(port).await;

        let resp = http.request("GET", "/hello", true).await;
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"hello world");
        assert_eq!(resp.headers.get("x-test").map(String::as_str), Some("yes"));

        let resp = http.request("GET", "/missing", false).await;
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, b"not found");

        // A fresh connection still upgrades to a WebSocket on the exact
        // same listening port - the router doesn't break the WS path.
        let url = format!("ws://127.0.0.1:{port}/");
        let (mut ws, _response) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("client should still be able to upgrade to a WebSocket");

        ws.send(Message::Text("hi".into()))
            .await
            .expect("should send text message");
        let msg = ws
            .next()
            .await
            .expect("should receive a reply")
            .expect("reply should not be an error");
        assert_eq!(msg.into_text().unwrap(), "echo:hi");
        ws.close(None).await.expect("should close cleanly");
    };

    tokio::select! {
        result = server_task => panic!("server task should not finish first: {result:?}"),
        () = client_task => {}
    }
}

/// Without an `on("request", ...)` handler registered, a plain HTTP request
/// is dropped exactly like before this feature existed - the general router
/// is opt-in, not a behavior change for existing `WebSocketServer` users.
#[tokio::test]
async fn test_websocket_server_drops_plain_http_without_a_request_handler() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/websocket");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("websocket-test")
        .polyfills(&["webcrypto"])
        .script(
            r#"
            export async function run(port) {
                const server = new WebSocketServer();
                server.on("connection", (conn) => {});
                await server.listen(port, "127.0.0.1");
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build the websocket component");

    let port: u16 = 18903;
    let (instance, store) = inst.parts();
    let func = instance
        .get_func(&mut *store, "run")
        .expect("run export not found");

    let server_task = async {
        let mut results = [];
        func.call_async(&mut *store, &[Val::U16(port)], &mut results)
            .await
    };

    let client_task = async {
        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("should connect");
        stream
            .write_all(b"GET /hello HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("should write request");

        let mut buf = [0u8; 16];
        let n = stream.read(&mut buf).await.expect("read should not error");
        assert_eq!(n, 0, "connection should be dropped, not answered");
    };

    tokio::select! {
        result = server_task => panic!("server task should not finish first: {result:?}"),
        () = client_task => {}
    }
}
