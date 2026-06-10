//! plgc — standalone Prolog compiler CLI.
//!
//! Exit codes (compile path): 0 = success, 2 = parse error,
//! 3 = compile/codegen/link error.

use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

fn run_script(source: &str, args: &[String]) -> ExitCode {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot create temp dir: {e}");
            return ExitCode::from(3);
        }
    };
    let bin = dir.path().join("plg-script");
    let src = std::path::Path::new(source);
    if let Err(e) = plgc::compile_files(&[src], &bin, false, plgc::OptLevel::O3) {
        eprintln!("error: {e}");
        return ExitCode::from(3);
    }
    match std::process::Command::new(&bin).args(args).status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(3) as u8),
        Err(e) => {
            eprintln!("error: failed to run compiled script: {e}");
            ExitCode::from(3)
        }
    }
}

#[derive(Parser)]
#[command(
    name = "plgc",
    version,
    about = "Compile ISO-subset Prolog to standalone native binaries"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile .pl source files to a native executable
    Build {
        /// Input .pl files (concatenated in order)
        inputs: Vec<PathBuf>,
        /// Output binary path (default: stem of first input)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Keep the generated .ll LLVM IR file for inspection
        #[arg(long)]
        keep_ir: bool,
        /// Build with -O0 and debug-friendly output
        #[arg(long)]
        debug: bool,
        /// Treat calls to undefined predicates as errors, not warnings
        #[arg(long)]
        deny_undefined: bool,
    },
    /// Compile to a temp binary and run it immediately (never interprets)
    Run {
        /// Input .pl files
        inputs: Vec<PathBuf>,
        /// Goal to solve, e.g. "ancestor(tom, X)"
        #[arg(long)]
        query: String,
        /// Maximum number of solutions to report
        #[arg(long)]
        limit: Option<usize>,
        /// Output format: json or text
        #[arg(long, default_value = "text")]
        format: String,
        /// Treat calls to undefined predicates as errors, not warnings
        #[arg(long)]
        deny_undefined: bool,
    },
    /// Parse and statically check .pl sources without compiling
    Check {
        /// Input .pl files
        inputs: Vec<PathBuf>,
        /// Treat calls to undefined predicates as errors, not warnings
        #[arg(long)]
        deny_undefined: bool,
    },
    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
}

/// Run the undefined-predicate lint and report it. Prints one line per
/// finding — `warning:` by default, `error:` under `--deny-undefined`.
/// Returns `Err(exit_code)` only when `deny` is set and findings exist, so
/// the caller aborts before producing a binary. A parse error is left for
/// the compile/check path to report (avoids double-reporting).
fn lint_undefined(sources: &[&std::path::Path], deny: bool) -> Result<(), u8> {
    let Ok(lints) = plgc::undefined_predicate_lints(sources) else {
        return Ok(());
    };
    let label = if deny { "error" } else { "warning" };
    for m in &lints {
        eprintln!("{label}: {m}");
    }
    if deny && !lints.is_empty() {
        return Err(2);
    }
    Ok(())
}

fn main() -> ExitCode {
    // Script mode (`#!/usr/bin/env plgc`): `plgc prog.pl [binary args…]`
    // compiles to a temp binary and execs it — same path as `plgc run`,
    // never interpretation.
    let raw: Vec<String> = std::env::args().collect();
    if raw.len() >= 2 && raw[1].ends_with(".pl") && std::path::Path::new(&raw[1]).exists() {
        return run_script(&raw[1], &raw[2..]);
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            inputs,
            output,
            keep_ir,
            debug,
            deny_undefined,
        } => {
            if inputs.is_empty() {
                eprintln!("error: no input files");
                return ExitCode::from(3);
            }
            let output =
                output.unwrap_or_else(|| PathBuf::from(inputs[0].file_stem().unwrap_or_default()));
            let sources: Vec<&std::path::Path> = inputs.iter().map(|p| p.as_path()).collect();
            if let Err(code) = lint_undefined(&sources, deny_undefined) {
                return ExitCode::from(code);
            }
            let opt = if debug {
                plgc::OptLevel::O0
            } else {
                plgc::OptLevel::O3
            };
            match plgc::compile_files(&sources, &output, keep_ir, opt) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::from(3)
                }
            }
        }
        Commands::Run {
            inputs,
            query,
            limit,
            format,
            deny_undefined,
        } => {
            // Compile to a temp binary and exec it — NEVER interpret.
            // Dev mode and production mode share one execution path
            // (see docs/design/LESSONS_FROM_V1.md, rule 3).
            if inputs.is_empty() {
                eprintln!("error: no input files");
                return ExitCode::from(3);
            }
            let sources: Vec<&std::path::Path> = inputs.iter().map(|p| p.as_path()).collect();
            if let Err(code) = lint_undefined(&sources, deny_undefined) {
                return ExitCode::from(code);
            }
            let dir = match tempfile::tempdir() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error: cannot create temp dir: {e}");
                    return ExitCode::from(3);
                }
            };
            let bin = dir.path().join("plg-run");
            if let Err(e) = plgc::compile_files(&sources, &bin, false, plgc::OptLevel::O0) {
                eprintln!("error: {e}");
                // Parse errors carry file:line:col; map them to exit 2.
                let code = if e.contains(": expected") || e.contains("at line") {
                    2
                } else {
                    3
                };
                return ExitCode::from(code);
            }
            let mut cmd = std::process::Command::new(&bin);
            cmd.arg("--query").arg(&query).arg("--format").arg(&format);
            if let Some(l) = limit {
                cmd.arg("--limit").arg(l.to_string());
            }
            match cmd.status() {
                Ok(status) => ExitCode::from(status.code().unwrap_or(3) as u8),
                Err(e) => {
                    eprintln!("error: failed to run compiled binary: {e}");
                    ExitCode::from(3)
                }
            }
        }
        Commands::Check {
            inputs,
            deny_undefined,
        } => {
            let sources: Vec<&std::path::Path> = inputs.iter().map(|p| p.as_path()).collect();
            match plgc::check_files(&sources) {
                Ok(()) => match lint_undefined(&sources, deny_undefined) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(code) => ExitCode::from(code),
                },
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
            ExitCode::SUCCESS
        }
    }
}
