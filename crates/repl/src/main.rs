//! `plgr` — interactive REPL for patch-prolog.
//!
//! The engine compiles whole programs to native binaries; this REPL
//! delivers an interactive feel by *driving the compiler*, never by
//! interpreting clauses at runtime (LESSONS_FROM_V1 rule 3). Clause and
//! `:load` edits recompile the session buffer to a temp binary; `?-`
//! queries re-invoke the *current* binary via `--query`. Full design:
//! `docs/design/REPL.md`.
//!
//! This is the M6 scaffold: a working TUI shell with the real
//! parse/validate + completion paths wired, and two clearly-marked
//! integration seams — the `vim-line` editor (`input.rs`) and the
//! in-process compiler link (`engine.rs`, currently shelling `plgc`).

mod app;
mod completion;
mod engine;
mod input;
mod run;
mod session;
mod ui;

use clap::Parser as ClapParser;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::Duration;

#[derive(ClapParser)]
#[command(name = "plgr", version, about = "Interactive REPL for patch-prolog")]
struct Args {
    /// Prolog source file to load into the session at startup.
    file: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();
    if let Err(e) = run(args.file) {
        eprintln!("plgr: {e}");
        std::process::exit(1);
    }
}

fn run(file: Option<PathBuf>) -> Result<(), String> {
    // Always restore the terminal, even on panic.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        hook(info);
    }));

    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    let mut app = app::App::new();
    if let Some(path) = file {
        app.load_file(&path);
    }

    let result = event_loop(&mut terminal, &mut app);
    app.save_history();

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    result.map_err(|e| e.to_string())
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut app::App,
) -> io::Result<()> {
    while !app.should_quit {
        terminal.draw(|f| ui::render(f, app))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            // With keyboard-enhancement terminals, key *release* events
            // also arrive; act on presses only.
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
        if app.should_edit {
            app.should_edit = false;
            edit_session(terminal, app)?;
        }
    }
    Ok(())
}

/// `:edit` — write the session buffer to a temp `.pl`, hand the terminal to
/// `$EDITOR` (default `vi`), then reload the edited buffer and recompile.
/// `main` owns the `Terminal`, so the suspend/resume lives here.
fn edit_session(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut app::App,
) -> io::Result<()> {
    use std::io::Write;

    let mut file = tempfile::Builder::new()
        .prefix("plgr-session-")
        .suffix(".pl")
        .tempfile()?;
    file.write_all(app.session.source().as_bytes())?;
    file.flush()?;
    let path = file.path().to_path_buf();

    // Suspend the TUI and give the terminal to the editor.
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    // shlex so `EDITOR="code --wait"` and quoted paths work.
    let parts = shlex::split(&editor).filter(|p| !p.is_empty());
    let status = match parts {
        Some(parts) => std::process::Command::new(&parts[0])
            .args(&parts[1..])
            .arg(&path)
            .status(),
        None => std::process::Command::new("vi").arg(&path).status(),
    };

    // Resume the TUI.
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.hide_cursor()?;
    terminal.clear()?;

    match status {
        Ok(_) => {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            app.apply_edit(&content);
        }
        Err(e) => app.note(format!("  :edit could not launch '{editor}': {e}")),
    }
    Ok(())
}

/// Rule-3 guard (LESSONS_FROM_V1 / docs/design/REPL.md checkpoint 6): the
/// REPL drives the compiler and never interprets clauses in-process.
/// Pinned here so it's enforced by CI, not just true by inspection.
#[cfg(test)]
mod rule3_guard {
    use std::fs;
    use std::path::Path;

    /// It must not link the solver substrate (`plg-runtime`); without the
    /// runtime there is no in-process solve path to drift into.
    #[test]
    fn does_not_depend_on_the_solver_runtime() {
        let cargo = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("read Cargo.toml");
        assert!(
            !cargo.contains("plg-runtime"),
            "plg-repl must not depend on plg-runtime — it compiles and execs, \
             never runs an in-process solver (rule 3)"
        );
    }

    /// No source file may *define* a clause-walking `solve` function. Match
    /// `fn` declarations at line start (after any visibility) so this guard
    /// doesn't trip over its own assertion text mentioning "solve".
    #[test]
    fn defines_no_solve_function() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in fs::read_dir(&src).expect("read src/") {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                let body = fs::read_to_string(&path).unwrap();
                for line in body.lines() {
                    let decl = line
                        .trim_start()
                        .trim_start_matches("pub(crate) ")
                        .trim_start_matches("pub ");
                    assert!(
                        !decl.starts_with("fn solve"),
                        "{}: defines a `solve` fn — the REPL must not interpret clauses",
                        path.display()
                    );
                }
            }
        }
    }
}
