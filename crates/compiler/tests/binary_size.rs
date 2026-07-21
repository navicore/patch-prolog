//! Binary hygiene: release binaries must stay standalone.
//!
//! The size ceiling is enforced by `just size-gate` (in `just ci`), which
//! builds the release compiler in a fresh target dir — deterministic by
//! construction. It used to be a cargo test here, but the debug test
//! environment embeds whichever libplg_runtime variant build.rs's mtime
//! scan lands on, and under `cargo test --workspace` several differently-
//! sized variants coexist (#63) — the measurement flapped. What remains
//! here is the standalone-contract check, which is variant-independent.

mod harness;

#[allow(unused_imports)]
use std::process::Command;

/// The compiled hello-world still answers (default text format). The size
/// ceiling lives in `just size-gate` — see the header comment.
#[test]
fn hello_world_binary_answers() {
    let c = harness::compile("hello(world).\n");
    let (out, code) = c.query("hello(X)", &[]);
    assert!(out.contains("X = world"), "{out}");
    assert_eq!(code, 1);
}

#[test]
#[cfg(target_os = "linux")]
fn binary_links_only_system_libraries() {
    // The standalone contract: no Rust, no LLVM, nothing beyond the
    // base system (libc/libm/libgcc_s + loader/vdso).
    let c = harness::compile("hello(world).\n");
    let out = Command::new("ldd").arg(&c.bin).output().expect("run ldd");
    let text = String::from_utf8_lossy(&out.stdout);
    const ALLOWED: &[&str] = &[
        "linux-vdso",
        "libm.so",
        "libgcc_s.so",
        "libc.so",
        "ld-linux",
    ];
    for line in text.lines() {
        let lib = line.split_whitespace().next().unwrap_or("");
        if lib.is_empty() {
            continue;
        }
        assert!(
            ALLOWED.iter().any(|a| lib.contains(a)),
            "unexpected dynamic dependency `{lib}` — the standalone contract \
             allows only base system libraries:\n{text}"
        );
    }
}
