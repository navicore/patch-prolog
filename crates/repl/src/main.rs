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
    }
    Ok(())
}
