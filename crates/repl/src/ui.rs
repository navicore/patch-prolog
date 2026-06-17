//! Terminal-style rendering: one borderless transcript that flows
//! top-down, with the live input inline on the last line behind the
//! prompt — like a normal terminal REPL, not a boxed TUI. The vi-mode is
//! shown dimly bottom-right only when *not* in insert mode (so ordinary
//! typing stays clean, but you can never get silently stuck in normal
//! mode). Helper panes (IR, etc.) are a later, on-demand addition.

use crate::app::{App, CONT, PROMPT};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let paging = app.is_paging();

    // The live line: while paging, a `;`/stop hint replaces the input behind
    // a prompt (continuation prompt while a multi-line clause is still open).
    let (live, cursor_col) = if paging {
        (
            "    ;  next solution    ·    any other key  stop".to_string(),
            0,
        )
    } else {
        let prompt = if app.pending.is_empty() { PROMPT } else { CONT };
        (
            format!("{prompt}{}", app.input.text()),
            prompt.chars().count() + app.input.cursor_col(),
        )
    };
    let mut lines: Vec<Line> = app.output.iter().map(|l| Line::raw(l.clone())).collect();
    lines.push(if paging {
        Line::raw(live).dim()
    } else {
        Line::raw(live)
    });

    // Flow from the top; once the transcript is taller than the screen,
    // drop the oldest lines so the newest (and the prompt) stay visible.
    let height = area.height.max(1) as usize;
    let skip = lines.len().saturating_sub(height);
    let visible = lines.split_off(skip);
    let cursor_row = (visible.len() - 1) as u16;
    f.render_widget(Paragraph::new(visible), area);

    // No editor cursor or mode indicator while paging — we're not editing.
    if !paging {
        f.set_cursor_position(Position::new(
            area.x + cursor_col as u16,
            area.y + cursor_row,
        ));
        let mode = app.input.status();
        if !mode.eq_ignore_ascii_case("insert") {
            let label = format!(" -- {} -- ", mode.to_uppercase());
            let w = label.chars().count() as u16;
            if area.width > w {
                let rect = Rect::new(area.right() - w, area.bottom() - 1, w, 1);
                f.render_widget(Paragraph::new(label).dim(), rect);
            }
        }
    }
}
