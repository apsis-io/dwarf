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
use tokio_tungstenite::tungstenite::Message;
use wasmtime::component::Val;

use common::TestCase;

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
