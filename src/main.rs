mod compiler;

use clap::Parser as ClapParser;
use compiler::CompileError;
use std::path::PathBuf;
use std::process;

#[derive(ClapParser)]
#[command(
    name = "patch-prolog",
    about = "Prolog compiler — compile .pl rules into standalone native binaries"
)]
struct Cli {
    /// Input .pl files
    #[arg(required = true)]
    files: Vec<PathBuf>,

    /// Output binary path
    #[arg(short, long, default_value = "a.out")]
    output: PathBuf,

    /// Build in debug mode (faster compile, slower runtime)
    #[arg(long)]
    debug: bool,
}

/// Exit codes:
/// 0 = compile succeeded
/// 2 = parse error in .pl files
/// 3 = compile error
fn main() {
    let cli = Cli::parse();

    if let Err(e) = compiler::compile(&cli.files, &cli.output, !cli.debug) {
        eprintln!("Error: {}", e);
        match e {
            CompileError::Parse(_) => process::exit(2),
            CompileError::Build(_) => process::exit(3),
        }
    }
}
