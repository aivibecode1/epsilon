//! The `/skills` and `/subagent` browser.
//!
//! The transcript listing was complete but it was still a wall of notices: to
//! compare two skills a reader scrolled the conversation, and the reason a skill
//! exists sat in the same line as its name. This panel keeps one selected row,
//! its description, why it is useful, where it comes from, and the exact command
//! that changes its state. It is read-only on purpose: the command remains the
//! one writer, so the panel cannot grow a second settings path.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::overlay::{self, Panel, Width};
use crate::ui::theme::Theme;

use super::App;

/// Which catalogue a panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogKind {
    /// Built-in and discovered skills.
    Skills,
    /// Built-in subagent roles.
    Subagents,
}

impl CatalogKind {
    fn title(self) -> &'static str {
        match self {
            Self::Skills => "skills",
            Self::Subagents => "subagents",
        }
    }
}

/// One row in the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    /// Name used by the toggle command.
    pub name: String,
    /// One-line purpose.
    pub description: String,
    /// Why the entry is worth enabling, shown for the selected row.
    pub why: String,
    /// Upstream credit, when the body is adapted.
    pub credit: Option<String>,
    /// `on`, `off`, or `installed (shadowed)`.
    pub state: String,
    /// Where it comes from: `built-in`, `project`, `user`.
    pub scope: String,
    /// The exact command that flips this row, for the footer.
    pub action: String,
}

/// A browser over skills or subagents. Owned by [`App`] while open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPanel {
    /// Which catalogue is displayed.
    pub kind: CatalogKind,
    /// All rows, in a stable order (built-ins first, then discovered names).
    pub entries: Vec<CatalogEntry>,
    /// Index of the highlighted row.
    pub selected: usize,
}

impl CatalogPanel {
    /// Build the skills browser from the agent's discovered skills and the
    /// bundled registry.
    pub fn skills(
        agent: &epsilon_core::Agent,
        config_dir: Option<&std::path::Path>,
        workspace: &std::path::Path,
    ) -> Self {
        let skills = agent.skills();
        let dir = config_dir;
        let builtin_root = dir.map(epsilon_core::builtin_skills::root_in);
        let disabled = dir
            .map(epsilon_core::config::load_disabled_skills_in)
            .unwrap_or_default();
        let mut entries = Vec::new();

        for builtin in epsilon_core::builtin_skills::all() {
            let installed =
                dir.is_some_and(|d| epsilon_core::builtin_skills::installed(builtin.name, d));
            let active = builtin_root.as_ref().is_some_and(|root| {
                skills
                    .iter()
                    .any(|s| s.name == builtin.name && s.root() == root)
            });
            let state = if active {
                "on".to_string()
            } else if installed {
                "installed (shadowed)".to_string()
            } else {
                "off".to_string()
            };
            let action = if active || installed {
                format!("/skill {} off", builtin.name)
            } else {
                format!("/skill {} on", builtin.name)
            };
            entries.push(CatalogEntry {
                name: builtin.name.to_string(),
                description: builtin.description.to_string(),
                why: builtin.why.to_string(),
                credit: builtin.credit.map(str::to_string),
                state,
                scope: "built-in · opt-in".to_string(),
                action,
            });
        }

        let mut discovered: Vec<epsilon_core::skills::Skill> = skills
            .into_iter()
            .filter(|s| {
                builtin_root
                    .as_ref()
                    .is_none_or(|root| s.root() != root.as_path())
            })
            .collect();
        discovered.sort_by(|a, b| a.name.cmp(&b.name));
        for skill in discovered {
            let is_disabled = disabled.iter().any(|d| d == &skill.name);
            let scope = if skill.root().starts_with(workspace) {
                format!("project · {}", short_path(skill.root()))
            } else {
                format!("user · {}", short_path(skill.root()))
            };
            entries.push(CatalogEntry {
                name: skill.name.clone(),
                description: skill.description.clone(),
                why: if is_disabled {
                    "Disabled for this session; enable it to put it back in the catalog."
                        .to_string()
                } else {
                    "Discovered from the skill roots and offered to the model when relevant."
                        .to_string()
                },
                credit: None,
                state: if is_disabled { "off" } else { "on" }.to_string(),
                scope,
                action: format!(
                    "/skill {} {}",
                    skill.name,
                    if is_disabled { "on" } else { "off" }
                ),
            });
        }

        Self {
            kind: CatalogKind::Skills,
            entries,
            selected: 0,
        }
    }

    /// Build the subagent browser. Subagent roles are all built-in; the user's own
    /// agent files are deliberately not listed because they are not this
    /// catalogue and the `task` tool already names them.
    pub fn subagents(config_dir: Option<&std::path::Path>) -> Self {
        let entries = epsilon_core::builtin_subagents::all()
            .iter()
            .map(|h| {
                let installed = config_dir
                    .is_some_and(|d| epsilon_core::builtin_subagents::installed(h.name, d));
                CatalogEntry {
                    name: h.name.to_string(),
                    description: h.description.to_string(),
                    why: h.why.to_string(),
                    credit: None,
                    state: if installed { "on" } else { "off" }.to_string(),
                    scope: "built-in subagent · opt-in".to_string(),
                    action: format!(
                        "/subagent {} {}",
                        h.name,
                        if installed { "off" } else { "on" }
                    ),
                }
            })
            .collect();
        Self {
            kind: CatalogKind::Subagents,
            entries,
            selected: 0,
        }
    }

    fn selected_entry(&self) -> Option<&CatalogEntry> {
        self.entries.get(self.selected)
    }

    fn move_by(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() - 1;
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, last as isize) as usize;
    }

    /// Render the panel's body. `height` is the number of inner rows; the
    /// list takes what is left after the detail and the hint.
    fn lines(&self, theme: &Theme, width: usize, height: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let on = self.entries.iter().filter(|e| e.state == "on").count();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} available", self.entries.len()),
                theme.dim_strong(),
            ),
            Span::styled(" · ", theme.faint()),
            Span::styled(format!("{on} on"), theme.success()),
        ]));

        // Leave room for the rule, the detail block and the hint. The list is
        // the part that scrolls; the detail must stay whole.
        // A blank row at the built-in/discovered boundary. Without it the two
        // populations read as one list even though their detail rows use
        // different scope labels.
        let first_discovered = self
            .entries
            .iter()
            .position(|entry| !entry.scope.starts_with("built-in"));
        let section_gap = first_discovered.is_some_and(|at| at > 0);
        let detail_rows = 3 + usize::from(
            self.selected_entry()
                .is_some_and(|entry| entry.credit.is_some()),
        );
        let list_rows = height
            .saturating_sub(1 + 1 + detail_rows + 1 + usize::from(section_gap))
            .clamp(3, 9);
        let start = self
            .selected
            .saturating_sub(list_rows / 2)
            .min(self.entries.len().saturating_sub(list_rows));
        let list_capacity = list_rows.saturating_sub(usize::from(
            section_gap && first_discovered.is_some_and(|at| at > start),
        ));
        for (offset, entry) in self
            .entries
            .iter()
            .enumerate()
            .skip(start)
            .take(list_capacity)
        {
            if section_gap && Some(offset) == first_discovered && offset > start {
                lines.push(Line::from(""));
            }
            let selected = offset == self.selected;
            let marker = if selected {
                Span::styled("▸ ", theme.accent())
            } else {
                Span::styled("  ", theme.faint())
            };
            let state = if entry.state.starts_with("on") {
                Span::styled(format!("[{}] ", entry.state), theme.success())
            } else {
                Span::styled(format!("[{}] ", entry.state), theme.faint())
            };
            let state_width = crate::ui::layout::display_width(&format!("[{}] ", entry.state));
            let name = format!("{} ", entry.name);
            let name_width = crate::ui::layout::display_width(&name);
            let budget = width
                .saturating_sub(2)
                .saturating_sub(state_width)
                .saturating_sub(name_width);
            let description =
                crate::wrap::fit_width_with(&entry.description, budget, theme.glyphs.ellipsis);
            lines.push(Line::from(vec![
                marker,
                state,
                Span::styled(
                    name,
                    if selected {
                        theme.title()
                    } else {
                        theme.text()
                    },
                ),
                Span::styled(description, theme.dim()),
            ]));
        }
        lines.push(Line::from(""));

        if let Some(entry) = self.selected_entry() {
            lines.push(Line::from(Span::styled(
                crate::wrap::fit_width_with(&entry.description, width, theme.glyphs.ellipsis),
                theme.text(),
            )));
            lines.push(Line::from(Span::styled(
                crate::wrap::fit_width_with(
                    &format!("why: {}", entry.why),
                    width,
                    theme.glyphs.ellipsis,
                ),
                theme.faint(),
            )));
            if let Some(credit) = &entry.credit {
                lines.push(Line::from(Span::styled(
                    crate::wrap::fit_width_with(
                        &format!("credit: {credit}"),
                        width,
                        theme.glyphs.ellipsis,
                    ),
                    theme.faint(),
                )));
            }
            let scope_action = format!("{} · {}", entry.scope, entry.action);
            lines.push(Line::from(Span::styled(
                crate::wrap::fit_width_with(&scope_action, width, theme.glyphs.ellipsis),
                theme.faint(),
            )));
        }
        lines.push(Line::from(Span::styled(
            "up/down · enter command · esc close",
            theme.faint(),
        )));
        lines
    }
}

/// A short path for the detail row, kept to one line.
fn short_path(path: &std::path::Path) -> String {
    crate::ui::format::format_path(path, None, 40, "...")
}

/// Draw the open catalogue panel.
pub(super) fn draw(f: &mut Frame<'_>, app: &App, outer: Rect) {
    let Some(catalog) = app.catalog.as_ref() else {
        return;
    };
    let width = Width::PANEL.resolve(outer);
    // Size to the content, not to a fixed 20 rows: at 80x24 the four-row
    // subagent list left seven empty rows inside the box, which reads as a
    // panel that failed to load rather than one that is complete.
    let list_rows = catalog.entries.len().min(9);
    let section_gap = catalog
        .entries
        .iter()
        .position(|entry| !entry.scope.starts_with("built-in"))
        .is_some_and(|at| at > 0);
    let detail_rows = 3 + usize::from(
        catalog
            .selected_entry()
            .is_some_and(|entry| entry.credit.is_some()),
    );
    let needed = (1 + list_rows + 1 + detail_rows + 1 + 2 + usize::from(section_gap)) as u16;
    let height = needed.min(20).min(outer.height.saturating_sub(2)).max(8);
    let panel = Panel {
        theme: &app.theme,
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: catalog.kind.title(),
        focused: true,
        border: None,
    };
    overlay::clear_band(f, outer, panel.area);
    let inner = panel.open(f);
    let lines = catalog.lines(&app.theme, inner.width as usize, inner.height as usize);
    f.render_widget(Paragraph::new(lines), inner);
}

/// Keys while the browser owns the keyboard. It is a reader: only navigation
/// and close are accepted; every editing key belongs to the composer.
pub(super) fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.code == KeyCode::Esc {
        app.catalog = None;
        return false;
    }
    if key.code == KeyCode::Enter {
        // Enter does not write. It puts the exact command on the prompt, where
        // the user can read it and submit it. A panel that wrote would be a
        // second settings path; a panel that did nothing on Enter would make the
        // most obvious key in the overlay a dead end.
        let action = app
            .catalog
            .as_ref()
            .and_then(CatalogPanel::selected_entry)
            .map(|entry| entry.action.clone());
        if let Some(action) = action {
            app.catalog = None;
            app.cursor_chars = action.chars().count();
            app.input = action;
        }
        return false;
    }
    let Some(catalog) = app.catalog.as_mut() else {
        return false;
    };
    match key.code {
        KeyCode::Up => catalog.move_by(-1),
        KeyCode::Down => catalog.move_by(1),
        KeyCode::PageUp => catalog.move_by(-5),
        KeyCode::PageDown => catalog.move_by(5),
        KeyCode::Home => catalog.selected = 0,
        KeyCode::End => catalog.selected = catalog.entries.len().saturating_sub(1),
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, state: &str) -> CatalogEntry {
        CatalogEntry {
            name: name.to_string(),
            description: format!("{name} description"),
            why: format!("{name} reason"),
            credit: None,
            state: state.to_string(),
            scope: "built-in · opt-in".to_string(),
            action: format!("/skill {name} on"),
        }
    }

    #[test]
    fn the_list_separates_built_in_roles_from_discovered_skills() {
        let theme = Theme::new(
            crate::ui::theme::ColorMode::Ansi16,
            crate::ui::theme::UNICODE,
        );
        let mut discovered = entry("user-skill", "on");
        discovered.scope = "user · ~/.agents/skills".to_string();
        let panel = CatalogPanel {
            kind: CatalogKind::Skills,
            entries: vec![entry("built-in", "off"), discovered],
            selected: 0,
        };
        let lines = panel.lines(&theme, 80, 30);
        let text: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
        assert!(
            text.get(2).is_some_and(|line| line.is_empty()),
            "a blank row must separate the built-in row from the discovered row: {text:?}"
        );
    }

    #[test]
    fn the_selection_stops_at_both_ends_instead_of_wrapping() {
        let mut panel = CatalogPanel {
            kind: CatalogKind::Skills,
            entries: vec![entry("a", "off"), entry("b", "on")],
            selected: 0,
        };
        panel.move_by(-1);
        assert_eq!(panel.selected, 0);
        panel.move_by(5);
        assert_eq!(panel.selected, 1);
    }

    #[test]
    fn the_list_shows_state_why_and_the_writer_command() {
        let theme = Theme::new(
            crate::ui::theme::ColorMode::Ansi16,
            crate::ui::theme::UNICODE,
        );
        let panel = CatalogPanel {
            kind: CatalogKind::Skills,
            entries: vec![entry("ponytail", "off")],
            selected: 0,
        };
        let text = panel
            .lines(&theme, 80, 20)
            .into_iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("[off] ponytail"), "{text}");
        assert!(text.contains("why: ponytail reason"), "{text}");
        assert!(text.contains("/skill ponytail on"), "{text}");
    }
}
