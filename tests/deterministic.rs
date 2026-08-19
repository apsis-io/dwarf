//! Reproducibility: the same inputs produce the same bytes.
//!
//! Wizer freezes the initialized guest heap into the artifact, so anything
//! the guest observes while being snapshotted is baked in — and anything
//! that varies between runs makes two builds of one input differ. These
//! tests are the guard on that, at the level a consumer cares about: the
//! sha256 of the component.
//!
//! What had to be pinned to get here (crates/core/src/lib.rs):
//!   - the wall clock, because QuickJS seeds `ctx->random_state` from
//!     `js__gettimeofday_us()` — Math.random's state was the build time
//!   - `wasi:random`, because something in the guest caches 16 bytes of it
//!     at init (std's RandomState shape; the runtime's own maps were
//!     already fixed-seed)
//!   - the implicit module root, which was the process's CWD and decided
//!     each module's guest-visible path (crates/core/src/resolver.rs)
mod common;

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use common::dwarf_cmd;

const WIT: &str = r#"
    package test:det;
    world det {
        export greet: func(name: string) -> string;
    }
"#;

const JS: &str = r#"
    export function greet(name) {
      return `hello, ${name}`;
    }
"#;

/// Byte-compare two components, reporting WHERE they diverge rather than
/// dumping megabytes of Vec<u8> into the failure output.
fn assert_same_bytes(a: &Path, b: &Path, what: &str) {
    let (x, y) = (
        std::fs::read(a).expect("read component"),
        std::fs::read(b).expect("read component"),
    );
    if x == y {
        return;
    }
    let at = x.iter().zip(&y).position(|(p, q)| p != q);
    panic!(
        "{what}: components differ ({} vs {} bytes){}",
        x.len(),
        y.len(),
        match at {
            Some(i) => format!(", first at byte {i}"),
            None => " (one is a prefix of the other)".into(),
        }
    );
}

fn assert_differ(a: &Path, b: &Path, what: &str) {
    let (x, y) = (
        std::fs::read(a).expect("read component"),
        std::fs::read(b).expect("read component"),
    );
    assert_ne!(x, y, "{what}");
}

/// Write the inputs once; every build in a test reads these same files.
fn fixture(dir: &TempDir) -> (PathBuf, PathBuf) {
    let wit = dir.path().join("det.wit");
    let js = dir.path().join("det.js");
    std::fs::write(&wit, WIT).unwrap();
    std::fs::write(&js, JS).unwrap();
    (wit, js)
}

#[test]
fn two_builds_of_one_input_are_byte_identical() {
    let dir = TempDir::new().unwrap();
    let (wit, js) = fixture(&dir);

    let mut outs = Vec::new();
    for n in 0..3 {
        let out = dir.path().join(format!("out{n}.wasm"));
        dwarf_cmd()
            .args(["--wit", wit.to_str().unwrap()])
            .args(["--js", js.to_str().unwrap()])
            .args(["--output", out.to_str().unwrap()])
            .assert()
            .success();
        outs.push(out);
    }

    assert_same_bytes(&outs[0], &outs[1], "builds 1 and 2");
    assert_same_bytes(&outs[1], &outs[2], "builds 2 and 3");
}

#[test]
fn the_working_directory_does_not_change_the_output() {
    // The build must depend on its INPUTS, not on where it was invoked.
    // This is what the cwd-rooted module-root default broke: the guest path
    // of the entry module changed with the cwd, which changed the heap.
    let dir = TempDir::new().unwrap();
    let (wit, js) = fixture(&dir);
    let elsewhere = TempDir::new().unwrap();

    let mut outs = Vec::new();
    for (n, cwd) in [dir.path(), elsewhere.path(), Path::new("/")]
        .into_iter()
        .enumerate()
    {
        let out = dir.path().join(format!("cwd{n}.wasm"));
        dwarf_cmd()
            .current_dir(cwd)
            .args(["--wit", wit.to_str().unwrap()])
            .args(["--js", js.to_str().unwrap()])
            .args(["--output", out.to_str().unwrap()])
            .assert()
            .success();
        outs.push(out);
    }

    assert_same_bytes(&outs[0], &outs[1], "cwd changed the bytes");
    assert_same_bytes(&outs[1], &outs[2], "cwd changed the bytes");
}

#[test]
fn source_date_epoch_is_the_one_deliberate_knob() {
    // The build-time clock is pinned, and SOURCE_DATE_EPOCH (the
    // reproducible-builds convention) is how a caller chooses the value.
    // Same epoch, same bytes; a different epoch is allowed to differ,
    // because QuickJS seeds its PRNG from that clock.
    let dir = TempDir::new().unwrap();
    let (wit, js) = fixture(&dir);

    let build = |epoch: &str, out: &Path| {
        dwarf_cmd()
            .env("SOURCE_DATE_EPOCH", epoch)
            .args(["--wit", wit.to_str().unwrap()])
            .args(["--js", js.to_str().unwrap()])
            .args(["--output", out.to_str().unwrap()])
            .assert()
            .success();
    };

    let (a, b, c) = (
        dir.path().join("e1.wasm"),
        dir.path().join("e2.wasm"),
        dir.path().join("e3.wasm"),
    );
    build("1700000000", &a);
    build("1700000000", &b);
    build("1800000000", &c);

    assert_same_bytes(&a, &b, "one epoch must give one artifact");
    assert_differ(&a, &c, "the epoch is meant to reach the snapshot");
}
