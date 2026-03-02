use clap::Parser as ClapParser;
use prolog_core::database::CompiledDatabase;
use prolog_core::parser::Parser;
use prolog_core::solver::{term_to_string, Solution, Solver};
use prolog_core::Term;
use std::process;

static COMPILED_DB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compiled_db.bin"));

#[derive(ClapParser)]
#[command(
    name = "patch-prolog",
    about = "Prolog engine for linting generative AI output"
)]
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

    // Deserialize the compiled database
    let mut db = match CompiledDatabase::from_bytes(COMPILED_DB) {
        Ok(db) => db,
        Err(e) => {
            output_error(&cli.format, &format!("Failed to load database: {}", e));
            process::exit(3);
        }
    };

    // Parse the query using the database's interner
    let (goals, vars) = match Parser::parse_query_with_vars(&cli.query, &mut db.interner) {
        Ok(result) => result,
        Err(e) => {
            output_error(&cli.format, &format!("Parse error: {}", e));
            process::exit(2);
        }
    };

    // Rebuild index since the interner may have grown with query atoms
    db.predicate_index = prolog_core::index::build_index(&db.clauses);

    // Solve
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

fn term_to_json(term: &Term, interner: &prolog_core::StringInterner) -> serde_json::Value {
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
