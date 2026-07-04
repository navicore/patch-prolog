//! Per-directory session persistence.
//!
//! The session source (the clauses the user has typed) is restored across
//! restarts, keyed by the *current working directory* — each project gets
//! its own REPL program, never a global one. `:reset` clears both the
//! in-memory buffer and the persisted file so a fresh start sticks.
//!
//! The store lives under `$HOME/.local/share/plgr/sessions/` (mirroring the
//! `plgr_history` convention). The filename is a reversible percent-encoding
//! of the absolute cwd, which is collision-free (unlike a naive `/`→`_`
//! sanitize that conflates `/a/b` with `/a_b`) and dependency-free and stable
//! across toolchain upgrades (unlike `DefaultHasher`, whose algorithm isn't
//! fixed and would silently orphan sessions). A leading `% cwd: <path>` line
//! is written into each file so the mapping is `cat`-readable; it is stripped
//! on load so it never pollutes the buffer.

use std::path::{Path, PathBuf};

const CWD_TAG: &str = "% cwd: ";

/// The sessions directory: `$HOME/.local/share/plgr/sessions/`.
fn dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/plgr/sessions"))
}

/// A reversible, filesystem-safe encoding of a path: `%`-escape every byte
/// that isn't `[A-Za-z0-9_.-]`. Injective (each byte maps exactly one way),
/// so two distinct paths never share a filename.
pub fn filename(cwd: &Path) -> String {
    let mut out = String::new();
    for &b in cwd.to_string_lossy().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// The persisted-session path for `cwd`, or `None` if `$HOME` is unset.
pub fn path_for(cwd: &Path) -> Option<PathBuf> {
    dir().map(|d| d.join(format!("{}.pl", filename(cwd))))
}

/// Strip leading `% cwd: ` metadata lines so they don't enter the buffer.
fn strip_cwd_tag(raw: &str) -> String {
    raw.lines()
        .skip_while(|l| l.trim_start().starts_with(CWD_TAG))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Load the session source for `cwd`, if any. Missing/unreadable = none.
pub fn load(cwd: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path_for(cwd)?).ok()?;
    Some(strip_cwd_tag(&raw))
}

/// Persist `source` as the session for `cwd`. Empty `source` deletes any
/// existing file so a `:reset` (or empty exit) doesn't resurrect on restart.
pub fn save(cwd: &Path, source: &str) {
    let Some(path) = path_for(cwd) else {
        return;
    };
    if source.trim().is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = format!("{CWD_TAG}{}\n{}", cwd.display(), source);
    let _ = std::fs::write(&path, body);
}

/// Delete the persisted session for `cwd` (best effort) — used by `:reset`.
pub fn remove(cwd: &Path) {
    if let Some(path) = path_for(cwd) {
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_is_collision_free_vs_underscore_paths() {
        // The naive `/`→`_` sanitize would conflate these two directories.
        let slash = filename(Path::new("/a/b"));
        let under = filename(Path::new("/a_b"));
        assert_ne!(slash, under, "distinct paths must not collide");
    }

    #[test]
    fn filename_encodes_separators_and_spaces() {
        // `/` and ` ` are escaped; alnum/`_`/`-`/`.` pass through (so a real
        // hyphen in `patch-prolog` is kept).
        assert_eq!(
            filename(Path::new("/Users/navicore/patch-prolog")),
            "%2FUsers%2Fnavicore%2Fpatch-prolog"
        );
        assert_eq!(filename(Path::new("/a b")), "%2Fa%20b");
    }

    #[test]
    fn filename_is_injective_over_near_neighbours() {
        let p = Path::new("/Users/navicore/git/navicore/patch-prolog");
        let q = Path::new("/Users/navicore/git/navicore/patch-prolog ");
        assert_ne!(filename(p), filename(q));
    }

    #[test]
    fn strip_cwd_tag_removes_only_leading_metadata() {
        // A leading `% cwd:` line is metadata and dropped; the body follows.
        // (A trailing newline isn't preserved by the line-join, which is
        // fine — `Session::source` re-adds one on compile.)
        let raw = "% cwd: /abs/dir\nfoo(1).\nbar(X) :- foo(X).\n";
        assert_eq!(strip_cwd_tag(raw), "foo(1).\nbar(X) :- foo(X).");
    }

    #[test]
    fn strip_cwd_tag_passes_through_untagged_source() {
        assert_eq!(strip_cwd_tag("foo(1).\n"), "foo(1).");
        assert_eq!(strip_cwd_tag(""), "");
    }

    #[test]
    fn save_load_roundtrips_through_the_filesystem() {
        // Drive persist through a throwaway HOME so the test never touches
        // the user's real sessions dir. SAFETY: this is the only test in the
        // crate that touches $HOME, so there's no concurrent reader to race.
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let cwd = Path::new("/fake/project");
        assert!(load(cwd).is_none(), "nothing saved yet");

        save(cwd, "foo(1).\nbar(X) :- foo(X).\n");
        let loaded = load(cwd).expect("saved session loads");
        assert_eq!(loaded, "foo(1).\nbar(X) :- foo(X).");

        // The raw file carries the cwd tag for `cat`-readability.
        let raw = std::fs::read_to_string(path_for(cwd).unwrap()).unwrap();
        assert!(raw.starts_with("% cwd: /fake/project\n"));

        // An empty save deletes the file — this is the `:reset`-sticks path.
        save(cwd, "");
        assert!(!path_for(cwd).unwrap().exists());
        assert!(load(cwd).is_none());
    }
}
