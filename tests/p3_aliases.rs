//! Every WASI-backed polyfill's implementation is built under an explicit
//! `...P3` name, with the plain name aliased FROM it (`x = xP3`, never the
//! reverse - see polyfills.rs's module doc for why the direction matters).
//! This confirms both names exist, are strictly identical (`===`), and the
//! plain name still works exactly as before.

mod common;

use wasmtime::component::Val;

use common::{TestCase, wasi_wit_dir};

#[tokio::test]
async fn test_p3_aliases_match_plain_globals() {
    let mut instance = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("p3-aliases-test")
        .script(
            r#"
            export async function check() {
                const pairs = [
                    ["console", "consoleP3"],
                    ["process", "processP3"],
                    ["setTimeout", "setTimeoutP3"],
                    ["setInterval", "setIntervalP3"],
                    ["clearTimeout", "clearTimeoutP3"],
                    ["clearInterval", "clearIntervalP3"],
                    ["fetch", "fetchP3"],
                    ["WebSocketServer", "WebSocketServerP3"],
                ];
                for (const [plain, aliased] of pairs) {
                    if (typeof globalThis[plain] === "undefined") return `FAIL: ${plain} is undefined`;
                    if (typeof globalThis[aliased] === "undefined") return `FAIL: ${aliased} is undefined`;
                    if (globalThis[aliased] !== globalThis[plain]) {
                        return `FAIL: ${aliased} !== ${plain}`;
                    }
                }
                if (crypto.getRandomValuesP3 !== crypto.getRandomValues) {
                    return "FAIL: crypto.getRandomValuesP3 !== crypto.getRandomValues";
                }

                // The plain name still works exactly as before.
                const bytes = crypto.getRandomValues(new Uint8Array(4));
                if (bytes.length !== 4) return "FAIL: crypto.getRandomValues broken";

                return "OK";
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build the p3-aliases component");

    let result = instance.call1_async("check", &[]).await.unwrap();
    assert_eq!(result, Val::String("OK".into()));
}
