//! Integration tests for the `webcrypto` polyfill (`--polyfill webcrypto`)
//! and the always-on `crypto.getRandomValues` (wired to `wasi:random/random`).
#![cfg(feature = "component-model-async")]

mod common;

use common::{TestCase, wasi_wit_dir};
use wasmtime::component::Val;

#[tokio::test]
async fn test_webcrypto_full_roundtrip_with_random() {
    let mut instance = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("webcrypto-test")
        .polyfills(&["webcrypto"])
        .script(
            r#"
            export async function run() {
                // Known-answer digest check.
                const helloDigest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode("hello"));
                const helloHex = [...new Uint8Array(helloDigest)].map((b) => b.toString(16).padStart(2, "0")).join("");
                if (helloHex !== "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824") {
                    return "FAIL: digest mismatch: " + helloHex;
                }

                // getRandomValues sanity.
                const r1 = crypto.getRandomValues(new Uint8Array(16));
                const r2 = crypto.getRandomValues(new Uint8Array(16));
                if (r1.length !== 16) return "FAIL: getRandomValues wrong length";
                if (r1.every((b, i) => b === r2[i])) return "FAIL: getRandomValues returned identical bytes twice";

                // HMAC sign/verify roundtrip.
                const hmacKey = await crypto.subtle.generateKey({ name: "HMAC", hash: "SHA-256" }, true, ["sign", "verify"]);
                const msg = new TextEncoder().encode("dwarf webcrypto test");
                const sig = await crypto.subtle.sign("HMAC", hmacKey, msg);
                if (!(await crypto.subtle.verify("HMAC", hmacKey, sig, msg))) return "FAIL: HMAC verify failed";

                // AES-GCM roundtrip + tamper rejection.
                const aesKey = await crypto.subtle.generateKey({ name: "AES-GCM", length: 128 }, true, ["encrypt", "decrypt"]);
                const iv = crypto.getRandomValues(new Uint8Array(12));
                const pt = new TextEncoder().encode("secret");
                const ct = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, aesKey, pt);
                const decrypted = await crypto.subtle.decrypt({ name: "AES-GCM", iv }, aesKey, ct);
                if (new TextDecoder().decode(decrypted) !== "secret") return "FAIL: AES-GCM roundtrip failed";
                const tampered = new Uint8Array(ct);
                tampered[0] ^= 0xff;
                let tamperRejected = false;
                try {
                    await crypto.subtle.decrypt({ name: "AES-GCM", iv }, aesKey, tampered);
                } catch {
                    tamperRejected = true;
                }
                if (!tamperRejected) return "FAIL: AES-GCM should reject tampered ciphertext";

                // ECDSA P-256 sign/verify roundtrip.
                const { privateKey, publicKey } = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
                const ecdsaSig = await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, privateKey, msg);
                if (!(await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, publicKey, ecdsaSig, msg))) {
                    return "FAIL: ECDSA verify failed";
                }

                // ECDH P-256 shared-secret agreement.
                const alice = await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"]);
                const bob = await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"]);
                const aliceShared = await crypto.subtle.deriveBits({ name: "ECDH", public: bob.publicKey }, alice.privateKey, 256);
                const bobShared = await crypto.subtle.deriveBits({ name: "ECDH", public: alice.publicKey }, bob.privateKey, 256);
                const toHex = (buf) => [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");
                if (toHex(aliceShared) !== toHex(bobShared)) return "FAIL: ECDH shared secret mismatch";

                // HKDF derive.
                const hkdfBase = await crypto.subtle.importKey("raw", aliceShared, "HKDF", false, ["deriveBits"]);
                const okm = await crypto.subtle.deriveBits(
                    { name: "HKDF", hash: "SHA-256", salt: new Uint8Array(0), info: new TextEncoder().encode("info") },
                    hkdfBase,
                    256,
                );
                if (new Uint8Array(okm).length !== 32) return "FAIL: HKDF length mismatch";

                return "OK";
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build webcrypto component");

    let result = instance.call1_async("run", &[]).await.unwrap();
    assert_eq!(result, Val::String("OK".into()));
}

#[tokio::test]
async fn test_get_random_values_requires_wasi_random_import() {
    let mut instance = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("webcrypto-no-random")
        .polyfills(&["webcrypto"])
        .script(
            r#"
            export async function run() {
                try {
                    crypto.getRandomValues(new Uint8Array(4));
                    return "should have thrown";
                } catch (e) {
                    return e.message;
                }
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build a webcrypto component without a random import");

    let result = instance.call1_async("run", &[]).await.unwrap();
    match result {
        Val::String(msg) => assert!(
            msg.contains("wasi:random/random"),
            "expected a clear error naming wasi:random/random, got: {msg}"
        ),
        other => panic!("expected string, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_digest_works_without_random_import() {
    // crypto.subtle's pure-computation methods (digest, and sign/verify/
    // encrypt/decrypt given an imported key) need no entropy at all, so they
    // must work even when wasi:random/random isn't imported - only
    // generateKey/getRandomValues themselves require it.
    let mut instance = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("webcrypto-no-random")
        .polyfills(&["webcrypto"])
        .script(
            r#"
            export async function run() {
                const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode("hello"));
                return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build a digest-only webcrypto component");

    let result = instance.call1_async("run", &[]).await.unwrap();
    assert_eq!(
        result,
        Val::String("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into())
    );
}
