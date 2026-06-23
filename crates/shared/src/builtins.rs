//! The builtin / control-construct vocabulary: the single source of
//! truth for *which* names the language knows, their arities, and a
//! one-line doc each. Reconciled in `docs/design/BUILTIN_VOCAB.md`.
//!
//! This is **data only** — no symbol mapping (that stays compiler-side
//! in `codegen/lower.rs::DET_BUILTINS`) and no dispatch (the runtime
//! keeps its own `match`). Codegen and the runtime are *checked against*
//! this table; the LSP (completion + hover) *reads* it directly.
//!
//! Zero-dependency, like the rest of `plg-shared`. IMPORTANT: the
//! runtime must NOT reference `BUILTINS` outside `#[cfg(test)]` — the
//! `doc` strings would otherwise land in every compiled program binary
//! (see the doc's "doc strings must never reach a compiled program"
//! constraint).

/// Where a name is handled, used only to partition the validation that
/// codegen/runtime stay in sync with this table. Not a dispatch hint.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuiltinKind {
    /// Control construct — handled structurally in codegen/runtime
    /// (`,` `;` `->` `\+` `once` `catch` `throw` `findall` `call`
    /// `between`).
    Control,
    /// Inline goal — its own `LGoal` variant or an op-code table
    /// (`=` `\=` `is` `compare`, arithmetic and term-order comparisons).
    Inline,
    /// Deterministic builtin dispatched to a `plg_rt_b_*` symbol
    /// (the `DET_BUILTINS` set).
    Det,
    /// Reserved arity-0 atom goal (`true` `fail` `false` `!`).
    Atom,
}

/// One vocabulary entry. `arity` is the canonical arity; `call/N` is
/// listed once at its minimum arity (1) and noted variadic in `doc`.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinSpec {
    pub name: &'static str,
    pub arity: u32,
    pub kind: BuiltinKind,
    pub doc: &'static str,
}

impl BuiltinSpec {
    /// Completion-eligible iff the name is a typeable identifier (first
    /// char ASCII-alphabetic). Offers `findall`/`once`/`is`/`compare`/
    /// `nl`, suppresses operators and `!`. Deliberately NOT derived from
    /// `kind`: `is`/`compare` (Inline) complete, `\+` (Control) does not.
    /// If a real counter-example appears, promote this to an explicit
    /// `complete: bool` column on `BuiltinSpec` and overrule per row.
    pub fn completable(&self) -> bool {
        self.name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    }
}

use BuiltinKind::{Atom, Control, Det, Inline};

/// The full vocabulary (56 rows). Docs for everything except `,` `;`
/// `->` port verbatim from v1's `BUILTIN_DOCS`; those three are new.
/// `rustfmt::skip` keeps it as a one-row-per-line table — the doc
/// strings would otherwise wrap to five lines each.
#[rustfmt::skip]
pub const BUILTINS: &[BuiltinSpec] = &[
    // --- Det: deterministic builtins (DET_BUILTINS mirror) ---
    b(Det, "var", 1, "Type check: succeeds if argument is an unbound variable."),
    b(Det, "nonvar", 1, "Type check: succeeds if argument is bound."),
    b(Det, "atom", 1, "Type check: succeeds if argument is an atom."),
    b(Det, "number", 1, "Type check: succeeds if argument is an integer or float."),
    b(Det, "integer", 1, "Type check: succeeds if argument is an integer."),
    b(Det, "float", 1, "Type check: succeeds if argument is a float."),
    b(Det, "compound", 1, "Type check: succeeds if argument is a compound term."),
    b(Det, "is_list", 1, "Type check: succeeds if argument is a proper list."),
    b(Det, "functor", 3, "`functor(Term, Name, Arity)` — inspect or construct a term's functor."),
    b(Det, "arg", 3, "`arg(N, Term, Arg)` — extract the N-th argument of Term."),
    b(Det, "=..", 2, "Univ: `T =.. L` decomposes T into a list of its functor and args."),
    b(Det, "copy_term", 2, "`copy_term(T, C)` — bind C to a copy of T with fresh variables."),
    b(Det, "atom_length", 2, "`atom_length(A, L)` — bind L to the length of atom A."),
    b(Det, "atom_concat", 3, "`atom_concat(A, B, C)` — concatenate atoms A and B into C."),
    b(Det, "atom_chars", 2, "`atom_chars(A, Chars)` — convert between an atom and a list of single-char atoms."),
    b(Det, "number_chars", 2, "`number_chars(N, Chars)` — convert between a number and a list of single-char atoms."),
    b(Det, "number_codes", 2, "`number_codes(N, Codes)` — convert between a number and a list of character codes."),
    b(Det, "msort", 2, "`msort(L, Sorted)` — sort without removing duplicates."),
    b(Det, "sort", 2, "`sort(L, Sorted)` — sort and remove duplicates."),
    b(Det, "succ", 2, "`succ(X, S)` — Peano successor relation; S = X + 1, both non-negative."),
    b(Det, "plus", 3, "`plus(X, Y, Z)` — addition relation; any one argument may be unbound."),
    b(Det, "unify_with_occurs_check", 2, "Unification with occurs check: rejects `X = f(X)`-style cycles."),
    b(Det, "write", 1, "Write a term to stdout (no newline)."),
    b(Det, "writeq", 1, "Write a term to stdout, quoting atoms so it reads back (no newline)."),
    b(Det, "writeln", 1, "Write a term to stdout followed by a newline."),
    b(Det, "nl", 0, "Write a newline to stdout."),
    // --- Inline: own LGoal variant / op-code table ---
    b(Inline, "=", 2, "Unification: `X = Y` succeeds if X and Y can be made identical."),
    b(Inline, "\\=", 2, "Not-unifiable: succeeds when `=` would fail."),
    b(Inline, "is", 2, "Arithmetic evaluation: `X is Expr` binds X to the value of Expr."),
    b(Inline, "compare", 3, "`compare(Order, T1, T2)` — bind Order to <, =, or > per standard term ordering."),
    b(Inline, "==", 2, "Term identity: structural equality without unification."),
    b(Inline, "\\==", 2, "Term non-identity."),
    b(Inline, "@<", 2, "Standard term ordering: less."),
    b(Inline, "@>", 2, "Standard term ordering: greater."),
    b(Inline, "@=<", 2, "Standard term ordering: less-or-equal."),
    b(Inline, "@>=", 2, "Standard term ordering: greater-or-equal."),
    b(Inline, "<", 2, "Arithmetic less-than."),
    b(Inline, ">", 2, "Arithmetic greater-than."),
    b(Inline, "=<", 2, "Arithmetic less-or-equal (note: `=<`, not `<=`)."),
    b(Inline, ">=", 2, "Arithmetic greater-or-equal."),
    b(Inline, "=:=", 2, "Arithmetic equality."),
    b(Inline, "=\\=", 2, "Arithmetic inequality."),
    // --- Control: structural constructs ---
    b(Control, ",", 2, "`(A, B)` — conjunction: prove A, then B."),
    b(Control, ";", 2, "`(A ; B)` — disjunction: prove A, or B on backtracking. `(C -> T ; E)` reads as if-then-else."),
    b(Control, "->", 2, "`(C -> T)` — if-then: if C succeeds (committing to its first solution), prove T; otherwise fail."),
    b(Control, "\\+", 1, "Negation as failure: succeeds when its argument fails."),
    b(Control, "once", 1, "`once(Goal)` — succeed at most once for Goal."),
    b(Control, "catch", 3, "`catch(Goal, Catcher, Recovery)` — run Goal; on thrown error matching Catcher, run Recovery."),
    b(Control, "throw", 1, "Raise an error term that propagates to the nearest matching `catch/3`."),
    b(Control, "findall", 3, "`findall(Template, Goal, List)` — collect all solutions of Goal."),
    b(Control, "call", 1, "Meta-call: execute its argument as a goal. Variadic — extra args are appended."),
    b(Control, "between", 3, "`between(Low, High, X)` — enumerate or test integers in [Low, High]."),
    // --- Atom: reserved arity-0 goals ---
    b(Atom, "true", 0, "Always succeeds."),
    b(Atom, "fail", 0, "Always fails."),
    b(Atom, "false", 0, "Always fails (alias for `fail`)."),
    b(Atom, "!", 0, "Cut: commit to current choices; remove choice points back to the parent clause."),
];

/// Const ctor so the table above reads as one row per line.
const fn b(kind: BuiltinKind, name: &'static str, arity: u32, doc: &'static str) -> BuiltinSpec {
    BuiltinSpec {
        name,
        arity,
        kind,
        doc,
    }
}

/// Exact lookup by name and arity (`call/N` matches only at arity 1).
pub fn lookup(name: &str, arity: u32) -> Option<&'static BuiltinSpec> {
    BUILTINS.iter().find(|s| s.name == name && s.arity == arity)
}

/// First doc for `name` regardless of arity — hover is arity-insensitive
/// (the cursor is on a name, not a resolved call).
pub fn doc(name: &str) -> Option<&'static str> {
    BUILTINS.iter().find(|s| s.name == name).map(|s| s.doc)
}

/// Completion: arity-0 names worth offering.
pub fn atom_names() -> impl Iterator<Item = &'static str> {
    BUILTINS
        .iter()
        .filter(|s| s.arity == 0 && s.completable())
        .map(|s| s.name)
}

/// Completion: (name, arity) for arity->0 names worth offering.
pub fn functor_names() -> impl Iterator<Item = (&'static str, u32)> {
    BUILTINS
        .iter()
        .filter(|s| s.arity > 0 && s.completable())
        .map(|s| (s.name, s.arity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_size_and_partition() {
        assert_eq!(BUILTINS.len(), 56, "roster size changed — update the doc");
        let count = |k: BuiltinKind| BUILTINS.iter().filter(|s| s.kind == k).count();
        assert_eq!(count(Det), 26);
        assert_eq!(count(Inline), 16);
        assert_eq!(count(Control), 10);
        assert_eq!(count(Atom), 4);
    }

    #[test]
    fn no_duplicate_name_arity() {
        for (i, s) in BUILTINS.iter().enumerate() {
            for t in &BUILTINS[i + 1..] {
                assert!(
                    !(s.name == t.name && s.arity == t.arity),
                    "duplicate {}/{}",
                    s.name,
                    s.arity
                );
            }
        }
    }

    #[test]
    fn every_row_has_a_doc() {
        for s in BUILTINS {
            assert!(!s.doc.is_empty(), "{}/{} has no doc", s.name, s.arity);
        }
    }

    /// The user-facing reference must enumerate every builtin — drift guard
    /// so `docs/builtin-reference.md` can't silently fall behind the table.
    #[test]
    fn reference_doc_covers_every_builtin() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/builtin-reference.md");
        let doc = std::fs::read_to_string(&path).expect("docs/builtin-reference.md must exist");
        for s in BUILTINS {
            let entry = format!("{}/{}", s.name, s.arity);
            assert!(
                doc.contains(&entry),
                "{entry} is missing from {}",
                path.display()
            );
        }
    }

    #[test]
    fn completable_tracks_identifier_not_kind() {
        // alphabetic-leading names complete, regardless of kind...
        assert!(lookup("is", 2).unwrap().completable()); // Inline, yes
        assert!(lookup("compare", 3).unwrap().completable()); // Inline, yes
        assert!(lookup("once", 1).unwrap().completable()); // Control, yes
        assert!(lookup("catch", 3).unwrap().completable()); // Control, yes (v1 omitted)
        assert!(lookup("nl", 0).unwrap().completable()); // Det atom-arity, yes
        // ...operators and `!` do not.
        assert!(!lookup("\\+", 1).unwrap().completable()); // Control, no
        assert!(!lookup("=..", 2).unwrap().completable()); // Det operator, no
        assert!(!lookup("!", 0).unwrap().completable()); // Atom, no
        assert!(!lookup(";", 2).unwrap().completable()); // Control, no
    }

    #[test]
    fn accessors_respect_completable() {
        let atoms: Vec<_> = atom_names().collect();
        assert!(atoms.contains(&"true") && atoms.contains(&"nl"));
        assert!(!atoms.contains(&"!"));
        let fns: Vec<_> = functor_names().collect();
        assert!(fns.contains(&("once", 1)) && fns.contains(&("catch", 3)));
        assert!(!fns.contains(&(",", 2)) && !fns.contains(&("=..", 2)));
    }
}
