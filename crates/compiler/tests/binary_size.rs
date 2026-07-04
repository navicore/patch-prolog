//! Binary hygiene: release binaries must stay lean and standalone.
//!
//! The release link strips the runtime archive's DWARF
//! (`-Wl,--strip-debug`) — without it a ~550K hello-world balloons to
//! ~4.4M. This guard catches regressions in the link flags or runaway
//! growth in what the runtime drags in.

mod harness;

#[allow(unused_imports)]
use std::process::Command;

/// Generous ceiling: hello-world is ~676K today; alert at 2x-ish.
const MAX_HELLO_BYTES: u64 = 1_400_000;

#[test]
fn hello_world_binary_stays_lean() {
    let c = harness::compile("hello(world).\n");
    let bytes = std::fs::metadata(&c.bin).expect("stat binary").len();
    assert!(
        bytes < MAX_HELLO_BYTES,
        "hello-world binary is {bytes} bytes (limit {MAX_HELLO_BYTES}); \
         did the release link lose -Wl,--strip-debug or grow the runtime?"
    );
    // And it still answers (default text format).
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
