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
    if let Err(e) = plgc::compile_files(
        &[src],
        &bin,
        false,
        plgc::OptLevel::O3,
        plgc::Target::Native,
    ) {
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
        /// Compilation target: the host (default), `wasm32-wasi` (a standalone
        /// CLI .wasm module), or `worker` (a Cloudflare Workers / V8 reactor
        /// .wasm). Both wasm targets need plgc built with `--features wasm`.
        #[arg(long)]
        target: Option<String>,
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

/// `compile_files` renders program parse errors as `path:line:col: message`
/// (via the frontend `Span` + `SourceMap`); every other failure it returns
/// (cannot-read-file, codegen, link) lacks that `:line:col:` shape. Detecting
/// it maps parse errors to exit 2 (bad input) vs 3 (environment/internal).
fn is_parse_error(msg: &str) -> bool {
    msg.match_indices(": ").any(|(end, _)| {
        let head = &msg[..end];
        let Some((rest, col)) = head.rsplit_once(':') else {
            return false;
        };
        let Some((_, line)) = rest.rsplit_once(':') else {
            return false;
        };
        !col.is_empty()
            && col.bytes().all(|b| b.is_ascii_digit())
            && !line.is_empty()
            && line.bytes().all(|b| b.is_ascii_digit())
    })
}

/// Map the `--target` string to a [`plgc::Target`]. `None` and the host
/// triple mean native; `wasm32-wasi`/`wasm32-wasip1` select the Tier-1 CLI
/// module; `worker`/`wasm32-unknown-unknown` select the Tier-2 reactor.
fn parse_target(target: Option<&str>) -> Result<plgc::Target, String> {
    match target {
        None | Some("native") => Ok(plgc::Target::Native),
        Some("wasm32-wasi") | Some("wasm32-wasip1") => Ok(plgc::Target::Wasm),
        Some("worker") | Some("wasm32-unknown-unknown") => Ok(plgc::Target::Worker),
        Some(other) => Err(format!(
            "unknown target `{other}` (supported: native, wasm32-wasi, worker)"
        )),
    }
}

/// Emit the reactor deploy scaffolding and report what landed. A glue failure
/// is a warning, not a build failure — the `.wasm` is already produced and the
/// glue is regenerable.
fn emit_worker_glue(output: &std::path::Path) {
    match plgc::worker_glue::emit(output) {
        Ok(written) if written.is_empty() => {
            eprintln!("note: kept existing worker glue (worker.js / wrangler.toml / config.capnp)");
        }
        Ok(written) => {
            eprintln!(
                "note: wrote {} next to {}",
                written.join(", "),
                output.display()
            );
            eprintln!("      serve locally:  just wasm-worker-serve <prog.pl>");
            eprintln!("      deploy:         npx wrangler deploy");
        }
        Err(e) => eprintln!("warning: reactor built, but writing worker glue failed: {e}"),
    }
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
            target,
        } => {
            if inputs.is_empty() {
                eprintln!("error: no input files");
                return ExitCode::from(3);
            }
            let target = match parse_target(target.as_deref()) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(3);
                }
            };
            // Default output stem. Distinct wasm extensions so compiling one
            // source to both wasm targets without an explicit `-o` doesn't
            // silently overwrite (`prog.wasm` vs `prog.worker.wasm`).
            let output = output.unwrap_or_else(|| {
                let stem = PathBuf::from(inputs[0].file_stem().unwrap_or_default());
                match target {
                    plgc::Target::Wasm => stem.with_extension("wasm"),
                    plgc::Target::Worker => stem.with_extension("worker.wasm"),
                    plgc::Target::Native => stem,
                }
            });
            let sources: Vec<&std::path::Path> = inputs.iter().map(|p| p.as_path()).collect();
            if let Err(code) = lint_undefined(&sources, deny_undefined) {
                return ExitCode::from(code);
            }
            let opt = if debug {
                plgc::OptLevel::O0
            } else {
                plgc::OptLevel::O3
            };
            match plgc::compile_files(&sources, &output, keep_ir, opt, target) {
                Ok(()) => {
                    // Drop deploy scaffolding next to a reactor module (D1g):
                    // worker.js + wrangler.toml + config.capnp, written only if
                    // absent so a rebuild never clobbers user edits.
                    if target == plgc::Target::Worker {
                        emit_worker_glue(&output);
                    }
                    ExitCode::SUCCESS
                }
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
            // Dev mode and production mode share one execution path.
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
            if let Err(e) = plgc::compile_files(
                &sources,
                &bin,
                false,
                plgc::OptLevel::O0,
                plgc::Target::Native,
            ) {
                eprintln!("error: {e}");
                // Parse errors carry file:line:col; map them to exit 2.
                let code = if is_parse_error(&e) { 2 } else { 3 };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_maps_known_targets() {
        assert_eq!(parse_target(None).unwrap(), plgc::Target::Native);
        assert_eq!(parse_target(Some("native")).unwrap(), plgc::Target::Native);
        assert_eq!(
            parse_target(Some("wasm32-wasi")).unwrap(),
            plgc::Target::Wasm
        );
        assert_eq!(
            parse_target(Some("wasm32-wasip1")).unwrap(),
            plgc::Target::Wasm
        );
        // Tier 2 reactor: both the friendly name and the triple select Worker.
        assert_eq!(parse_target(Some("worker")).unwrap(), plgc::Target::Worker);
        assert_eq!(
            parse_target(Some("wasm32-unknown-unknown")).unwrap(),
            plgc::Target::Worker
        );
    }

    #[test]
    fn parse_target_rejects_unknown_and_lists_supported() {
        let err = parse_target(Some("wasm64")).unwrap_err();
        assert!(
            err.contains("worker"),
            "supported list must mention worker: {err}"
        );
    }
}
