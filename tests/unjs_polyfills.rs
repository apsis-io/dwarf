//! Tests for the unjs-backed static polyfills: ufo, scule, klona, ohash,
//! knitwork, unstorage - see crates/core/src/polyfills.rs's `POLYFILLS`.
#![cfg(feature = "component-model-async")]

mod common;

use common::TestCase;
use wasmtime::component::Val;

#[test]
fn test_ufo_polyfill() {
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:ufo;
            world ufo-test {
                export check: func() -> string;
            }
        "#,
        )
        .polyfills(&["ufo"])
        .script(
            r##"
            export function check() {
                if (ufo.joinURL("http://example.com", "a", "b/") !== "http://example.com/a/b/") return "FAIL: joinURL";
                if (ufo.withoutTrailingSlash("/a/b/") !== "/a/b") return "FAIL: withoutTrailingSlash";
                if (ufo.withTrailingSlash("/a/b") !== "/a/b/") return "FAIL: withTrailingSlash";

                const parsed = ufo.parseURL("http://foo.com/bar?x=1#frag");
                if (parsed.host !== "foo.com" || parsed.pathname !== "/bar" || parsed.search !== "?x=1" || parsed.hash !== "#frag") {
                    return "FAIL: parseURL: " + JSON.stringify(parsed);
                }

                const q = ufo.parseQuery("a=1&b=2");
                if (q.a !== "1" || q.b !== "2") return "FAIL: parseQuery";

                if (ufo.withQuery("/x", { a: "1" }) !== "/x?a=1") return "FAIL: withQuery";

                return "OK";
            }
            "##,
        )
        .build()
        .expect("should build ufo component");

    let result = instance.call1("check", &[]);
    assert_eq!(result, Val::String("OK".into()));
}

#[test]
fn test_scule_polyfill() {
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:scule;
            world scule-test {
                export check: func() -> string;
            }
        "#,
        )
        .polyfills(&["scule"])
        .script(
            r#"
            export function check() {
                if (scule.camelCase("hello-world") !== "helloWorld") return "FAIL: camelCase";
                if (scule.pascalCase("hello-world") !== "HelloWorld") return "FAIL: pascalCase";
                if (scule.kebabCase("helloWorld") !== "hello-world") return "FAIL: kebabCase";
                if (scule.snakeCase("helloWorld") !== "hello_world") return "FAIL: snakeCase";
                return "OK";
            }
            "#,
        )
        .build()
        .expect("should build scule component");

    let result = instance.call1("check", &[]);
    assert_eq!(result, Val::String("OK".into()));
}

#[test]
fn test_klona_polyfill() {
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:klona;
            world klona-test {
                export check: func() -> string;
            }
        "#,
        )
        .polyfills(&["klona"])
        .script(
            r#"
            export function check() {
                const original = { a: 1, nested: { b: [1, 2, 3] } };
                const cloned = klona(original);
                cloned.nested.b.push(4);
                if (original.nested.b.length !== 3) return "FAIL: mutation leaked into original";
                if (cloned.nested.b.length !== 4) return "FAIL: clone did not get the mutation";
                if (JSON.stringify(original) !== '{"a":1,"nested":{"b":[1,2,3]}}') return "FAIL: original mutated";
                return "OK";
            }
            "#,
        )
        .build()
        .expect("should build klona component");

    let result = instance.call1("check", &[]);
    assert_eq!(result, Val::String("OK".into()));
}

#[test]
fn test_ohash_polyfill() {
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:ohash;
            world ohash-test {
                export check: func() -> string;
            }
        "#,
        )
        .polyfills(&["ohash"])
        .script(
            r#"
            export function check() {
                const h1 = ohash.hash({ a: 1, b: 2 });
                const h2 = ohash.hash({ b: 2, a: 1 });
                const h3 = ohash.hash({ a: 1, b: 3 });
                if (typeof h1 !== "string" || h1.length === 0) return "FAIL: hash not a non-empty string";
                if (h1 !== h2) return "FAIL: key order should not affect hash";
                if (h1 === h3) return "FAIL: different values hashed the same";
                if (!ohash.isEqual({ a: 1 }, { a: 1 })) return "FAIL: isEqual should be true";
                if (ohash.isEqual({ a: 1 }, { a: 2 })) return "FAIL: isEqual should be false";
                return "OK";
            }
            "#,
        )
        .build()
        .expect("should build ohash component");

    let result = instance.call1("check", &[]);
    assert_eq!(result, Val::String("OK".into()));
}

#[test]
fn test_knitwork_polyfill() {
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:knitwork;
            world knitwork-test {
                export check: func() -> string;
            }
        "#,
        )
        .polyfills(&["knitwork"])
        .script(
            r#"
            export function check() {
                const imp = knitwork.genImport("some-module", ["foo", { name: "bar", as: "baz" }]);
                if (!imp.includes('import { foo, bar as baz } from "some-module";')) {
                    return "FAIL: genImport: " + imp;
                }

                const obj = knitwork.genObjectFromRaw({ a: "1", b: "2" });
                if (!obj.includes("a: 1") || !obj.includes("b: 2")) return "FAIL: genObjectFromRaw: " + obj;

                if (knitwork.genString("it's a test") !== `"it's a test"`) {
                    return "FAIL: genString: " + knitwork.genString("it's a test");
                }

                return "OK";
            }
            "#,
        )
        .build()
        .expect("should build knitwork component");

    let result = instance.call1("check", &[]);
    assert_eq!(result, Val::String("OK".into()));
}

#[tokio::test]
async fn test_unstorage_polyfill() {
    let mut instance = TestCase::new()
        .wit(
            r#"
            package test:unstorage;
            world unstorage-test {
                export check: async func() -> string;
            }
        "#,
        )
        .polyfills(&["unstorage"])
        .script(
            r#"
            export async function check() {
                const storage = unstorage.createStorage();

                if (await storage.hasItem("a")) return "FAIL: hasItem before set";

                await storage.setItem("a", { value: 1 });
                if (!(await storage.hasItem("a"))) return "FAIL: hasItem after set";

                const value = await storage.getItem("a");
                if (JSON.stringify(value) !== '{"value":1}') return "FAIL: getItem: " + JSON.stringify(value);

                await storage.setItem("b", "hello");
                const keys = (await storage.getKeys()).sort();
                if (JSON.stringify(keys) !== '["a","b"]') return "FAIL: getKeys: " + JSON.stringify(keys);

                await storage.removeItem("a");
                if (await storage.hasItem("a")) return "FAIL: removeItem did not remove";

                await storage.clear();
                if ((await storage.getKeys()).length !== 0) return "FAIL: clear did not clear";

                return "OK";
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build unstorage component");

    let result = instance.call1_async("check", &[]).await.unwrap();
    assert_eq!(result, Val::String("OK".into()));
}
