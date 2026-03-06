use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const STDLIB_PL: &str = include_str!("../knowledge/stdlib.pl");

/// Compile `.pl` files into a standalone native binary.
pub fn compile(input_files: &[PathBuf], output: &Path, release: bool) -> Result<(), String> {
    // 1. Read and concatenate all .pl source (stdlib first)
    let mut all_source = String::from(STDLIB_PL);
    all_source.push('\n');
    for file in input_files {
        let content = fs::read_to_string(file)
            .map_err(|e| format!("Failed to read {}: {}", file.display(), e))?;
        all_source.push_str(&content);
        all_source.push('\n');
    }

    // 2. Parse and validate — catch errors early, before invoking cargo
    let mut interner = patch_prolog_core::StringInterner::new();
    let clauses = patch_prolog_core::parser::Parser::parse_program(&all_source, &mut interner)
        .map_err(|e| format!("Parse error: {}", e))?;

    eprintln!(
        "Parsed {} clauses from {} file(s)",
        clauses.len(),
        input_files.len()
    );

    // 3. Compile and serialize the database
    let db = patch_prolog_core::CompiledDatabase::new(interner, clauses);
    let bytes = db
        .to_bytes()
        .map_err(|e| format!("Serialization error: {}", e))?;

    // 4. Create temp project
    let temp_dir =
        std::env::temp_dir().join(format!("patch-prolog-compile-{}", std::process::id()));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).map_err(|e| format!("Failed to clean temp dir: {}", e))?;
    }
    fs::create_dir_all(temp_dir.join("src"))
        .map_err(|e| format!("Failed to create temp project: {}", e))?;

    // Write compiled_db.bin
    fs::write(temp_dir.join("compiled_db.bin"), &bytes)
        .map_err(|e| format!("Failed to write compiled database: {}", e))?;

    // Write Cargo.toml — pin to same patch-prolog-core version we were built with
    let core_version = env!("CARGO_PKG_VERSION");
    let cargo_toml = format!(
        r#"[package]
name = "compiled-prolog"
version = "0.1.0"
edition = "2021"

[dependencies]
patch-prolog-core = "{core_version}"
clap = {{ version = "4", features = ["derive"] }}
serde_json = "1"
bincode = "1"
"#
    );
    fs::write(temp_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("Failed to write Cargo.toml: {}", e))?;

    // Write build.rs — just copies the pre-serialized database to OUT_DIR
    let build_rs = r#"fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    std::fs::copy("compiled_db.bin", format!("{}/compiled_db.bin", out_dir)).unwrap();
}
"#;
    fs::write(temp_dir.join("build.rs"), build_rs)
        .map_err(|e| format!("Failed to write build.rs: {}", e))?;

    // Write main.rs — query-only CLI (no compile subcommand)
    fs::write(temp_dir.join("src/main.rs"), GENERATED_MAIN_RS)
        .map_err(|e| format!("Failed to write main.rs: {}", e))?;

    // 5. Build
    eprintln!("Building binary...");
    let mut cmd = Command::new("cargo");
    cmd.arg("build").current_dir(&temp_dir);
    if release {
        cmd.arg("--release");
    }

    let status = cmd
        .status()
        .map_err(|e| format!("Failed to run cargo: {}", e))?;

    if !status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err("cargo build failed".to_string());
    }

    // 6. Copy binary to output path
    let profile = if release { "release" } else { "debug" };
    let built_binary = temp_dir
        .join("target")
        .join(profile)
        .join("compiled-prolog");
    fs::copy(&built_binary, output)
        .map_err(|e| format!("Failed to copy binary to {}: {}", output.display(), e))?;

    // 7. Cleanup
    let _ = fs::remove_dir_all(&temp_dir);

    eprintln!("Compiled binary: {}", output.display());
    Ok(())
}

const GENERATED_MAIN_RS: &str = r##"use clap::Parser as ClapParser;
use patch_prolog_core::database::CompiledDatabase;
use patch_prolog_core::parser::Parser;
use patch_prolog_core::solver::{term_to_string, Solution, Solver};
use patch_prolog_core::Term;
use std::process;

static COMPILED_DB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compiled_db.bin"));

#[derive(ClapParser)]
#[command(about = "Prolog query engine")]
struct Cli {
    /// Prolog query to execute
    #[arg(short, long)]
    query: String,

    /// Maximum number of solutions to return
    #[arg(short, long)]
    limit: Option<usize>,

    /// Output format: json or text
    #[arg(short, long, default_value = "json")]
    format: String,
}

/// Exit codes:
/// 0 = no solutions (compliant)
/// 1 = solutions found (violations)
/// 2 = parse error
/// 3 = runtime error
fn main() {
    let cli = Cli::parse();

    let mut db = match CompiledDatabase::from_bytes(COMPILED_DB) {
        Ok(db) => db,
        Err(e) => {
            output_error(&cli.format, &format!("Failed to load database: {}", e));
            process::exit(3);
        }
    };

    let (goals, vars) = match Parser::parse_query_with_vars(&cli.query, &mut db.interner) {
        Ok(result) => result,
        Err(e) => {
            output_error(&cli.format, &format!("Parse error: {}", e));
            process::exit(2);
        }
    };

    db.predicate_index = patch_prolog_core::index::build_index(&db.clauses);

    let mut solver = Solver::new(&db, goals, vars);
    if let Some(limit) = cli.limit {
        solver = solver.with_limit(limit);
    }

    let solutions = match solver.all_solutions() {
        Ok(s) => s,
        Err(e) => {
            output_error(&cli.format, &format!("Runtime error: {}", e));
            process::exit(3);
        }
    };

    let count = solutions.len();
    let exhausted = cli.limit.map_or(true, |l| count < l);

    match cli.format.as_str() {
        "json" => output_json(&solutions, count, exhausted, &db),
        "text" => output_text(&solutions, &db),
        _ => {
            output_error("text", &format!("Unknown format: {}", cli.format));
            process::exit(2);
        }
    }

    if count > 0 {
        process::exit(1);
    } else {
        process::exit(0);
    }
}

fn output_json(solutions: &[Solution], count: usize, exhausted: bool, db: &CompiledDatabase) {
    let solutions_json: Vec<serde_json::Value> = solutions
        .iter()
        .map(|sol| {
            let mut map = serde_json::Map::new();
            for (name, term) in &sol.bindings {
                map.insert(name.clone(), term_to_json(term, &db.interner));
            }
            serde_json::Value::Object(map)
        })
        .collect();

    let output = serde_json::json!({
        "solutions": solutions_json,
        "count": count,
        "exhausted": exhausted,
    });

    println!("{}", serde_json::to_string(&output).unwrap());
}

fn output_text(solutions: &[Solution], db: &CompiledDatabase) {
    if solutions.is_empty() {
        println!("false.");
        return;
    }
    for sol in solutions {
        if sol.bindings.is_empty() {
            println!("true.");
        } else {
            for (name, term) in &sol.bindings {
                println!("{} = {}", name, term_to_string(term, &db.interner));
            }
        }
    }
}

fn output_error(format: &str, message: &str) {
    match format {
        "json" => {
            let output = serde_json::json!({"error": message});
            println!("{}", serde_json::to_string(&output).unwrap());
        }
        _ => eprintln!("Error: {}", message),
    }
}

fn term_to_json(term: &Term, interner: &patch_prolog_core::StringInterner) -> serde_json::Value {
    match term {
        Term::Atom(id) => serde_json::Value::String(interner.resolve(*id).to_string()),
        Term::Integer(n) => serde_json::json!(n),
        Term::Float(f) => serde_json::json!(f),
        Term::Var(id) => serde_json::Value::String(format!("_{}", id)),
        Term::Compound { functor, args } => {
            let name = interner.resolve(*functor);
            let args_json: Vec<serde_json::Value> =
                args.iter().map(|a| term_to_json(a, interner)).collect();
            serde_json::json!({"functor": name, "args": args_json})
        }
        Term::List { head, tail } => {
            let mut elements = vec![term_to_json(head, interner)];
            let mut current = tail.as_ref();
            loop {
                match current {
                    Term::List { head, tail } => {
                        elements.push(term_to_json(head, interner));
                        current = tail;
                    }
                    Term::Atom(id) if interner.resolve(*id) == "[]" => {
                        return serde_json::json!(elements);
                    }
                    _ => {
                        return serde_json::json!({
                            "list": elements,
                            "tail": term_to_json(current, interner)
                        });
                    }
                }
            }
        }
    }
}
"##;
