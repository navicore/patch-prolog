# Build-Time Compilation Pipeline

patch-prolog compiles Prolog rules into the binary at build time. There is no runtime file loading — the knowledge base is embedded via `include_bytes!`.

## Pipeline

```
knowledge/*.pl → build.rs → Parser → Vec<Clause> + StringInterner
                                         ↓
                              CompiledDatabase::new()
                              (interns [] and !, builds predicate index)
                                         ↓
                              bincode::serialize → $OUT_DIR/compiled_db.bin
                                         ↓
                              include_bytes! in main.rs → static COMPILED_DB
```

## Why Build-Time Compilation

1. **Zero runtime I/O** — the binary is self-contained, no file paths to manage
2. **Parse errors at build time** — malformed `.pl` files fail the build, not the runtime
3. **Atom interning** — all atoms from the knowledge base are interned at build time; query atoms are interned at runtime and the predicate index is rebuilt

## Key Implementation Details

- `.pl` files in `knowledge/` are read in sorted order for deterministic builds
- `build.rs` watches the `knowledge/` directory for changes (`cargo:rerun-if-changed`)
- `CompiledDatabase::new()` always interns `[]` and `!` — these atoms must exist for list operations, findall, and cut to work
- Serialization uses bincode (compact binary format, fast deserialization)
- At runtime, the query parser may add new atoms to the interner; the predicate index is rebuilt after query parsing to account for these

## Adding Rules

Drop `.pl` files in `knowledge/`. They will be compiled into the binary on next `cargo build`. The stdlib (`knowledge/stdlib.pl`) provides standard list predicates.

## Atom Interning

The `StringInterner` maps strings to `AtomId` (u32) values. This is critical for performance — unification compares integer IDs, not strings.

The interner is shared between the database and solver. The solver clones it because some predicates (`atom_concat/3`, `atom_chars/2`) create new atoms at runtime. The solver's interner is returned with solutions via `all_solutions_with_interner()` so that result terms can be displayed correctly.
