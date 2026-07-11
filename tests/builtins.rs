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

#[test]
fn test_abort_controller_signals_listeners_and_onabort() {
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:builtins;
            world builtins-test {
                export check: func() -> string;
            }
        "#,
        )
        .script(
            r#"
            export function check() {
                const controller = new AbortController();
                const signal = controller.signal;
                if (signal.aborted) return "FAIL: aborted before abort()";
                if (signal.reason !== undefined) return "FAIL: reason set before abort()";

                let listenerFired = false;
                let onabortFired = false;
                signal.addEventListener("abort", (e) => {
                    listenerFired = e.type === "abort" && e.target === signal;
                });
                signal.onabort = (e) => { onabortFired = e.type === "abort"; };

                controller.abort("custom reason");

                if (!signal.aborted) return "FAIL: not aborted after abort()";
                if (signal.reason !== "custom reason") return "FAIL: wrong reason: " + signal.reason;
                if (!listenerFired) return "FAIL: addEventListener listener did not fire";
                if (!onabortFired) return "FAIL: onabort did not fire";

                // Second abort() call is a no-op (reason must not change).
                controller.abort("ignored");
                if (signal.reason !== "custom reason") return "FAIL: reason changed on second abort()";

                let threw = false;
                try {
                    signal.throwIfAborted();
                } catch (e) {
                    threw = e === "custom reason";
                }
                if (!threw) return "FAIL: throwIfAborted did not throw the reason";

                return "OK";
            }
            "#,
        )
        .build()
        .expect("should build builtins component");

    let result = instance.call1("check", &[]);
    assert_eq!(result, Val::String("OK".into()));
}

#[test]
fn test_abort_signal_static_abort_and_default_reason() {
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:builtins;
            world builtins-test {
                export check: func() -> string;
            }
        "#,
        )
        .script(
            r#"
            export function check() {
                const signal = AbortSignal.abort();
                if (!signal.aborted) return "FAIL: AbortSignal.abort() didn't abort";
                if (!(signal.reason instanceof Error)) return "FAIL: default reason isn't an Error";
                if (signal.reason.name !== "AbortError") return "FAIL: default reason name: " + signal.reason.name;

                const withReason = AbortSignal.abort("boom");
                if (withReason.reason !== "boom") return "FAIL: explicit reason not preserved";

                return "OK";
            }
            "#,
        )
        .build()
        .expect("should build builtins component");

    let result = instance.call1("check", &[]);
    assert_eq!(result, Val::String("OK".into()));
}

#[test]
fn test_abort_signal_remove_event_listener() {
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
                const controller = new AbortController();
                let fired = false;
                const listener = () => { fired = true; };
                controller.signal.addEventListener("abort", listener);
                controller.signal.removeEventListener("abort", listener);
                controller.abort();
                return !fired;
            }
            "#,
        )
        .build()
        .expect("should build builtins component");

    let result = instance.call1("check", &[]);
    assert_eq!(result, Val::Bool(true));
}
