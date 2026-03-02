use std::fs;
use std::path::Path;

fn main() {
    let knowledge_dir = Path::new("knowledge");

    // Watch for changes in the knowledge directory
    println!("cargo:rerun-if-changed=knowledge/");

    // Collect all .pl files from the knowledge directory
    let mut all_source = String::new();

    if knowledge_dir.exists() {
        let mut entries: Vec<_> = fs::read_dir(knowledge_dir)
            .expect("Failed to read knowledge/ directory")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "pl").unwrap_or(false))
            .collect();

        // Sort for deterministic builds
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            println!("cargo:rerun-if-changed={}", path.display());
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));
            all_source.push_str(&content);
            all_source.push('\n');
        }
    }

    // Parse and compile the knowledge base
    let mut interner = prolog_core::StringInterner::new();
    let clauses = if all_source.trim().is_empty() {
        Vec::new()
    } else {
        prolog_core::parser::Parser::parse_program(&all_source, &mut interner)
            .unwrap_or_else(|e| panic!("Failed to parse knowledge base: {}", e))
    };

    let db = prolog_core::CompiledDatabase::new(interner, clauses);
    let bytes = db
        .to_bytes()
        .expect("Failed to serialize compiled database");

    // Write the compiled database to OUT_DIR
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_path = Path::new(&out_dir).join("compiled_db.bin");
    fs::write(&out_path, bytes).expect("Failed to write compiled database");
}
