//! Tests for dwarf's always-on builtins (TextEncoder/TextDecoder) that need
//! no WIT/host dependency - see crates/core/src/polyfills.rs's
//! `generate_builtins`.
mod common;

use common::TestCase;
use wasmtime::component::Val;

#[test]
fn test_text_decoder_accepts_raw_array_buffer() {
    // TextDecoder.decode() must accept a plain ArrayBuffer (not just an
    // ArrayBufferView/TypedArray), matching the real WHATWG Encoding spec's
    // `BufferSource = ArrayBuffer | ArrayBufferView` input type. Real-world
    // callers hit this via any API that returns a raw ArrayBuffer, e.g. the
    // `webcrypto` polyfill's `subtle.digest`/`decrypt`.
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:builtins;
            world builtins-test {
                export decode-array-buffer: func() -> string;
            }
            "#,
        )
        .script(
            r#"
            export function decodeArrayBuffer() {
                const bytes = new TextEncoder().encode("hello, array buffer");
                // .buffer is a *plain* ArrayBuffer, not a view.
                return new TextDecoder().decode(bytes.buffer);
            }
            "#,
        )
        .build()
        .expect("should build builtins component");

    let result = instance.call1("decode-array-buffer", &[]);
    assert_eq!(result, Val::String("hello, array buffer".into()));
}

#[test]
fn test_text_decoder_still_accepts_typed_array_and_nullish() {
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:builtins;
            world builtins-test {
                export check: func() -> bool;
            }
            "#,
        )
        .script(
            r#"
            export function check() {
                const bytes = new TextEncoder().encode("typed array");
                if (new TextDecoder().decode(bytes) !== "typed array") return false;
                if (new TextDecoder().decode() !== "") return false;
                if (new TextDecoder().decode(undefined) !== "") return false;
                return true;
            }
            "#,
        )
        .build()
        .expect("should build builtins component");

    let result = instance.call1("check", &[]);
    assert_eq!(result, Val::Bool(true));
}
