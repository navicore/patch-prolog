//! TUI rendering: a scrollback pane, the input line, and a status line.
//!
//! TODO(vim-line): once the `vim-line` editor is wired, surface its mode
//! (NORMAL/INSERT) in the input title and drive the cursor from it.

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::Stylize;
use ratatui::text::Line as TLine;
use ratatui::widgets::{Block, Paragraph, Wrap};

pub fn render(f: &mut Frame, app: &App) {
    let areas = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .split(f.area());

    let log: Vec<TLine> = app.output.iter().map(|l| TLine::raw(l.clone())).collect();
    let scrollback = Paragraph::new(log)
        .block(Block::bordered().title(" plgr — session "))
        .wrap(Wrap { trim: false });
    f.render_widget(scrollback, areas[0]);

    let hint = if app.pending.is_empty() {
        "?- query · clause. · :help"
    } else {
        "continuing clause — end with `.`"
    };
    let title = format!(" input [{}]  ({hint}) ", app.input.status());
    let input = Paragraph::new(app.input.text()).block(Block::bordered().title(title));
    f.render_widget(input, areas[1]);

    // Cursor: +1 for the border, plus the editor's char-column.
    let col = areas[1].x + 1 + app.input.cursor_col() as u16;
    f.set_cursor_position(Position::new(col, areas[1].y + 1));

    let status = format!(
        " {} clause(s){}  ·  :load :list :reset :quit ",
        app.session.clauses.len(),
        if app.session.dirty {
            " · modified (recompiles on next query)"
        } else {
            ""
        },
    );
    f.render_widget(Paragraph::new(status).dim(), areas[2]);
}
