//! What the window does with a line the reader typed that the terminal would treat as a
//! command.
//!
//! # Why this file exists
//!
//! Until now the window parsed **nothing**. `send_draft` handed the text to
//! `session.submit`, so a reader who knew the terminal typed `/compact` and the model was
//! asked to compact — as a *prompt*. It answered something plausible, the transcript filled
//! with prose, and nothing anywhere said the window had never heard of a slash command. The
//! same class as every other "it worked, in the pane that says it worked" this repository
//! tracks by name.
//!
//! # The shape of the answer
//!
//! Three outcomes, and the third is the one that matters:
//!
//! * **a command the window can do** — it runs, and the transcript says what happened;
//! * **not a command at all** — it is the reader's words and is sent. `parse_command`
//!   answers `None` for anything it does not recognise, so `/usr/bin/env is broken` is still
//!   a sentence about the environment rather than a swallowed line.
//! * **a command the terminal has and the window does not** — it is **refused, by name**.
//!
//! That third case is the whole point. A window with four commands is a smaller product; a
//! window that quietly forwards the other thirty-six to the model is a window that lies.
//!
//! # Why the match is exhaustive, and why that is not pedantry
//!
//! [`Command`] lives in the terminal crate and the window already depends on it (ADR-020, for
//! the palette). Matching it exhaustively means **adding a command to the terminal breaks this
//! file's build** until someone decides what the window does with it — which is the decision
//! that was being skipped. The alternative, a `_ =>` arm, is exactly how a frontend silently
//! accumulates a decade of "handled by sending it to the model".
//!
//! The split is by *where the effect is*, not by how much work it is:
//!
//! * **agent-level** — the effect is on the agent or its session, so both frontends can do it
//!   identically: `/compact`, `/goal`.
//! * **window-level** — the effect is on a particular set of pixels, so it belongs to the
//!   window that has those pixels: `/sessions`, `/new`, `/theme`.
//! * **terminal-only** — the effect is on a terminal that does not exist here: `/timeline`,
//!   `/fork`, `/editor`, `/hotkeys`.

use epsilon_tui::picker::{self, Command};

use crate::IdeApp;

/// What the window did with a typed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not a command. Send this text as the reader's prompt.
    Send(String),
    /// Handled. This is what the transcript should say about it.
    Ran(String),
    /// A command the terminal has and this window does not. Refused **by name**, so the
    /// reader learns where it lives rather than that "something did not happen".
    Refused(String),
}

impl IdeApp {
    /// Handle `text` if it is a command the window knows, and say what happened.
    ///
    /// Returns [`Outcome::Send`] for anything that is not a command — including a path that
    /// merely starts with a slash.
    pub fn try_command(&mut self, text: &str) -> Outcome {
        let Some(command) = picker::parse_command(text) else {
            return Outcome::Send(text.to_string());
        };
        match command {
            // ---- agent-level: both frontends do the same thing ----
            Command::Compact => {
                self.run_compact();
                Outcome::Ran("compacting context…".to_string())
            }
            Command::Goal(argument) => self.run_goal_argument(argument),

            // ---- window-level: the effect is on this window ----
            Command::Sessions => {
                self.set_sidebar_view(crate::app::SidebarView::Sessions);
                Outcome::Ran("sessions — click one to resume it".to_string())
            }

            // ---- everything else is the terminal's, and saying so is the feature ----
            other => Outcome::Refused(refusal_for(&other)),
        }
    }

    /// The `/goal` argument, which is a sentence rather than a subcommand.
    ///
    /// The tail goes whole, for the reason the terminal's parser documents: splitting an
    /// objective in the parser is how a goal ends up reading "make the doc".
    fn run_goal_argument(&mut self, argument: Option<String>) -> Outcome {
        let Some(argument) = argument else {
            return match self.goal.as_ref() {
                Some(view) => Outcome::Ran(format!(
                    "goal · {} · round {}/{} · {} gate(s) · {}",
                    view.goal.status.as_str(),
                    view.goal.rounds_started,
                    view.goal.max_rounds,
                    view.goal.gates.len(),
                    if view.armed {
                        "continuing automatically"
                    } else {
                        "not armed"
                    }
                )),
                None => Outcome::Ran("no goal is set — /goal <what you want> creates one".into()),
            };
        };
        let (verb, rest) = match argument.split_once(char::is_whitespace) {
            Some((v, r)) => (v.to_ascii_lowercase(), r.trim().to_string()),
            None => (argument.to_ascii_lowercase(), String::new()),
        };
        let command = match verb.as_str() {
            "pause" => crate::app::GoalCommand::Pause,
            "resume" => crate::app::GoalCommand::Resume,
            "clear" => crate::app::GoalCommand::Clear,
            // `edit`, `complete` and `blocked` exist in the terminal and their verbs are real
            // (`GoalOp` carries all of them). Saying "not here" is honest; guessing at them
            // would be a second implementation of the goal rule, which is the thing ADR-021
            // exists to prevent.
            "edit" | "complete" | "blocked" => {
                return Outcome::Refused(format!(
                    "/goal {verb} — the terminal has it; the window has not built it yet"
                ));
            }
            _ => {
                // An objective, not a verb: create one from the whole sentence.
                self.run_goal_command(crate::app::GoalCommand::Create, &argument);
                return Outcome::Ran(format!("goal created · {}", argument.trim()));
            }
        };
        self.run_goal_command(command, &rest);
        Outcome::Ran(format!("/goal {verb}"))
    }
}

/// Why a command is not available here, naming it and where it is.
///
/// One function, so the wording is one wording — and so the refusal a reader sees is about
/// **this** command, not a generic failure that could be anything.
///
/// **Two different refusals, because they are two different facts.** A command the terminal
/// has and this window does not is a gap in this product, and the reader should know where it
/// works. A command **nobody** has is a typo, and telling them to look in the terminal would
/// send them there for something that is not there either.
fn refusal_for(command: &Command) -> String {
    match command {
        Command::Unknown(word) => format!("unknown command '/{word}' — nothing here runs it"),
        other => format!(
            "/{} — not in this window; it works in the terminal",
            head_of(other)
        ),
    }
}

/// The command's own name, for the refusal.
///
/// Deliberately not `{:?}`: a debug rendering of an enum variant is not something to show a
/// reader, and `Command::Model("x")` would print its argument.
fn head_of(command: &Command) -> &'static str {
    match command {
        Command::Models | Command::Model(_) => "model",
        Command::Think(_) => "think",
        Command::Compact => "compact",
        Command::Theme(_) => "theme",
        Command::Timeline => "timeline",
        Command::Tree => "tree",
        Command::Goal(_) => "goal",
        Command::Fork => "fork",
        Command::Clone => "clone",
        Command::New => "new",
        Command::Resume(_) => "resume",
        Command::Sessions => "sessions",
        Command::Editor => "editor",
        Command::Session => "session",
        Command::Trust(_) => "trust",
        Command::Reload => "reload",
        Command::Export(_) => "export",
        Command::Import(_) => "import",
        Command::Share => "share",
        Command::Copy => "copy",
        Command::Name(_) => "name",
        Command::Changelog => "changelog",
        Command::Help => "help",
        Command::Quit => "quit",
        Command::Tasks => "tasks",
        Command::Hotkeys => "hotkeys",
        Command::Settings => "settings",
        Command::ScopedModels => "scoped_models",
        Command::Clear => "clear",
        Command::Expand => "expand",
        Command::Skills => "skills",
        Command::Skill(_) => "skill",
        Command::Templates => "templates",
        Command::Search(_) => "search",
        Command::Login { .. } => "login",
        Command::Logout(_) => "logout",
        Command::Unknown(_) => "?",
    }
}

/// Every command head the window **refuses**, for the test that the refusal table and the
/// parser have not drifted apart.
pub fn refused_heads() -> Vec<&'static str> {
    picker::COMMAND_HEADS
        .iter()
        .copied()
        .filter(|head| {
            matches!(
                picker::parse_command(&format!("/{head} x")).map(|c| refusal_head(&c)),
                Some(h) if !SUPPORTED.contains(&h)
            )
        })
        .collect()
}

/// The command a head resolves to, for the drift check.
fn refusal_head(command: &Command) -> &'static str {
    head_of(command)
}

/// Heads this window answers itself. Everything else in [`picker::COMMAND_HEADS`] is refused
/// by name, which is the honest default.
///
/// `/new` is **not** here: the terminal's starts a new session *file* (M221) and the window
/// has no such action, so it is refused with its name like every other unbuilt verb rather
/// than mapped onto the nearest thing that exists.
pub const SUPPORTED: &[&str] = &["compact", "goal", "sessions"];
