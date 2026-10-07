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
//! * **not a command at all** — it does not start with a slash, so it is the reader's words
//!   and is sent. Anything that *does* start with a slash is a command attempt whatever
//!   follows it, and an unrecognised word comes back as [`Command::Unknown`] rather than
//!   as `None`: `/usr/bin/env is broken` is refused as the typo it is, which is a different
//!   answer from the third case below and is worded differently for that reason;
//! * **a command the terminal has and the window does not** — it is **refused, by name**.
//!
//! That third case is the whole point. A window with a handful of commands is a smaller
//! product; a window that quietly forwards the rest to the model is a window that lies.
//!
//! # Why this file holds no list of commands
//!
//! It held two, and the second was the expensive one. [`SUPPORTED`] is thirty-six names and is
//! still here — it is the declaration, and it stays. What went is a 38-arm `head_of` that
//! spelled every [`Command`] variant out by hand, plus a `refusal_head` that existed only to
//! re-derive the refusal set through it. Adding a terminal command meant editing that match
//! — it has no catch-all arm, which was deliberate — **in a frontend the terminal's
//! maintainer does not own**, and a command added at the terminal with a desktop's tree not
//! open was a build failure attributed to the wrong file.
//!
//! And it was wrong where it mattered, quietly. `COMMAND_HEADS` says `branch` and `exit`;
//! the hand-written table said `timeline` and `quit`. So the guard drove `/branch` and
//! `/exit`, and the window printed `/timeline` and `/quit` — and because the guard asserted
//! nothing about the sentence, and the one test that did assert on it typed the alias, **a
//! negative control confirms the old code failed exactly that**: `/branch` was refused as
//! `"/timeline — not in this window; it works in the terminal"`.
//!
//! **Two names, two jobs, neither of them a table.** A *refusal* names the command the reader
//! spelled, because they can see it in their own composer ([`typed_head`], three lines of
//! tokenising). A *capability* answer — is this a command the window runs? — is looked up in
//! the shared table, [`COMMAND_HEADS`] (`crates/epsilon-contracts/src/command.rs:371`),
//! which the parser is held against in both directions by two tests sitting beside it:
//! `every_head_in_command_heads_is_parsed` and
//! `the_parser_does_not_know_a_command_the_list_does_not`, the second of which reads the
//! parser's quoted arms rather than a list of its own.
//!
//! **That table is in `epsilon-contracts`, and this file reads it there directly** (M290). It
//! used to be read through `epsilon_tui::picker` — a native window importing a terminal crate
//! to learn what a `/word` means, in the crate whose manifest says it "draws no terminals and
//! depends on no terminal behaviour". The command half moved down to the crate below both
//! front ends; this edge is now only the *palette* and the *numbers*
//! (`epsilon_tui::ThemeMode`, `ui::theme`, `ui::metrics`), which are colour roles and values
//! rather than behaviour. See `crates/epsilon-contracts/src/command.rs` for the move.
//!
//! So "a new command is a decision someone has to make about this window" now comes from
//! that pair rather than from a hand-written match here: a head the window has no arm for is
//! refused **by name**, which is the honest default and needs no decision, and a name in
//! [`SUPPORTED`] the parser has never heard of fails
//! `the_supported_set_is_a_subset_of_the_heads_the_parser_knows`.
//!
//! **What the desktop still writes down is one declaration, and it is a subset by design.**
//! A command the window can run is one the terminal can run, because it is the same agent
//! loop behind a different draw call. Refusing the rest by name is the feature, not a gap to
//! close — and each name added to [`SUPPORTED`] is a verb this window stops refusing, so
//! growing it is a product decision and not a refactor. ADR-024 records the larger version,
//! one `REGISTRY` with per-frontend capability as data, as designed and **not built**
//! (`docs/design/epsilon-decisions.md:1874`); this file is the step that does not need it.
//!
//! The split is by *where the effect is*, not by how much work it is:
//!
//! * **agent-level** — the effect is on the agent or its session, so both frontends can do it
//!   identically: `/compact`, `/goal`.
//! * **window-level** — the effect is on a particular set of pixels, so it belongs to the
//!   window that has those pixels: `/sessions`, `/tasks`, `/session`, `/clear`, `/theme`,
//!   `/help` — exactly [`SUPPORTED`], no more.
//! * **terminal-only** — everything else, refused by name: a terminal-only surface
//!   (`/timeline`, `/editor`, `/hotkeys`), a session-file action this window has no
//!   equivalent for (`/new`, `/resume`, `/fork`, `/clone`), and the rest.

use std::sync::Arc;

// `ALIAS_HEADS` is deliberately **not** imported here: nothing in the window reads it, so
// naming it here would be an import that only the test module uses — and `-D warnings` is
// right to call that out. The tests reach it through `super::*` plus their own line below.
use epsilon_contracts::command::{COMMAND_HEADS, Command, parse_command};

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
    /// merely starts with a slash, which is a command attempt and is refused as a typo.
    pub fn try_command(&mut self, text: &str) -> Outcome {
        let Some(command) = parse_command(text) else {
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
            // **`/new` starts a session file and switches to it (full-port round 6).**
            // Same act as the terminal's (M221): a file beside the others, handed
            // over through `switch_session` — the path the sidebar's clicks already
            // take. Refuses while a run is in flight, like the terminal's
            // "new: busy".
            Command::New => self.run_new_session(),
            // **`/resume` opens the browser bare, switches by path (full-port round 7).**
            // Bare is `/sessions` by another spelling, exactly as the terminal's
            // bare form opens its browser; a path is `switch_session` — the path
            // the sidebar's clicks already take. Same-path resumes say so rather
            // than rebuilding silently, because a typed line that does nothing
            // and says nothing is the state this UI refuses.
            Command::Resume(path) => self.run_resume_session(path),
            // **`/fork` and `/clone` branch the current file (full-port round 8).**
            // Same act as the terminal's: the store's `fork_branch_at`, not a
            // file copy (a copy duplicates abandoned branches under one
            // identity). The fork is stored and named; switching to it is
            // `/resume`'s job, exactly as the terminal says "open with
            // --session" rather than moving.
            Command::Fork => self.run_fork_session("fork", "forked"),
            Command::Clone => self.run_fork_session("clone", "cloned"),
            // **`/export` writes the session to HTML or JSON (full-port round 15).**
            // Same formats, same default folder, same words as the terminal's:
            // the spelling is shared (`split_export_arg`) and the rendering is
            // shared (`epsilon_session::export`) — this arm is only the window's
            // half (workspace root, notices) around both.
            Command::Export(arg) => self.run_export_session(arg),
            // **`/model` names the current model bare, switches with an id
            // (full-port round 20).** An id runs the same `set_model_id` the
            // terminal's `/model <id>` runs — session record included (M303),
            // and deliberately *only* that: the terminal additionally saves a
            // machine-wide default, which a window action must not rewrite
            // behind the terminal's back. The picker half stays honestly out
            // of reach: there is no model picker in this window yet, and a
            // bare `/model` that opened nothing would be the silent no-op, so
            // bare names what is running instead.
            //
            // Round 21 built the picker, so bare opens it — the terminal's
            // shape, with the running model marked active inside.
            Command::Models => {
                self.open_model_picker();
                Outcome::Ran("models — pick a provider, then a model".to_string())
            }
            Command::Model(id) => self.run_model_argument(Some(id)),
            // **`/think` shows the thinking level bare, sets it with an id
            // (full-port round 30).** Same act as the terminal's: read
            // `agent.thinking_level`, or parse and set through the shared
            // `ThinkingLevel` with the terminal's words. No picker half
            // exists to refuse — the level is one of seven names, and typing
            // one is the whole interface.
            Command::Think(arg) => self.run_think_argument(arg),
            // **`/hotkeys` opens the shortcuts reference (full-port round 40).**
            // The terminal prints its cheat sheet; the window's living
            // equivalent is the reference overlay (same table, effective
            // chords, screen-reader names) — printing a frozen copy beside it
            // would be the second source of truth the reference exists to
            // prevent.
            Command::Hotkeys => {
                self.open_shortcuts();
                Outcome::Ran("shortcuts — every binding with its effective chord".to_string())
            }
            // **`/copy` copies the last assistant message (full-port round
            // 40).** The terminal's act through the platform clipboard
            // (`egui` writes it; no new dependency): same source (the last
            // assistant row), same words. The context comes from the frame
            // (see the `ctx` field) — in a test harness the output is
            // discarded, so no test touches the real clipboard.
            Command::Copy => self.run_copy(),
            // **`/exit` opens the quit guard (full-port round 41).** The
            // terminal exits; the window asks first — a tested product rule
            // says closing is the reader's decision even for an empty
            // window. Same dialog the platform close raises (see `ask_quit`),
            // so the two doors cannot disagree about what quitting costs.
            // `/quit` and `/q` parse here too, and take the same door.
            Command::Quit => {
                self.ask_quit();
                Outcome::Ran("quit — confirm in the dialog".to_string())
            }
            // **`/reload` re-reads skills and keybindings from disk (full-port
            // round 42).** The terminal's `/reload` minus the parts this
            // window does not manage (prompt templates, agent defs — both load
            // at composition and have no window reader): skills re-discover
            // through the shared `discover_with_notices` and land on the
            // shared agent, keybindings re-read so a fixed file applies
            // without a restart. Discovery lines forward verbatim (they name
            // root and reason); the summary names the counts.
            Command::Reload => self.run_reload(),
            // **`/skills` lists discovered skills, `/skill` switches one
            // (full-port round 43).** The terminal's list with its words —
            // name, description, and state per row, plus the toggle hint —
            // and its toggle through the shared disabled-skills store, so a
            // switch in either front end holds in both. The row formatting is
            // duplicated from the terminal's `skill_lines` (ten lines, tested
            // there) rather than shared: it lives in the TUI crate, and
            // moving it is a refactor for a second consumer to force.
            Command::Skills => self.run_skills_list(),
            Command::Skill(arg) => self.run_skill_toggle(arg),
            // **`/changelog` shows the workspace changelog's head (full-port
            // round 44).** The terminal's read (workspace `CHANGELOG.md`,
            // else `docs/CHANGELOG.md`, first ~120 lines) as one answer:
            // header, lines, overflow line — or the honest absence.
            // Read-only, so safe to drive in tests.
            Command::Changelog => self.run_changelog(),
            // **`/restore` previews and applies file checkpoints (full-port
            // round 47).** The terminal's two shared calls with its words:
            // preview first (the sentence that lets a reader decide), apply
            // only on a trailing `yes` — asking for the plan never carries
            // the power to act on it. Same busy guard as `/resume`: a run in
            // flight owns the workspace's files.
            Command::Restore(arg) => self.run_restore_file(arg),
            // **`/name` shows and sets the session display name (full-port
            // round 46).** The terminal's act with its words, over the same
            // `session_name` custom entry: bare reads the persisted tail
            // (the read discovery uses for every listed row), setting
            // persists beside it and says which of the two happened. No
            // in-memory copy — the file is the truth, in both front ends.
            Command::Name(arg) => self.run_name_argument(arg),
            // **`/login` stores a pasted provider key, `/logout` removes one
            // (full-port round 38).** The same shared store the terminal
            // uses (`epsilon_llm::auth`), so a key saved in either front end
            // works in both; the terminal's words throughout. What the window
            // cannot do it refuses by name: subscription (OAuth) ceremonies
            // need the terminal's browser flow, and bare or keyless forms
            // need its picker and masked prompt — the usage names the shape
            // that works here.
            Command::Login { provider, key } => self.run_login(provider, key),
            Command::Logout(provider) => self.run_logout(provider),
            // **`/settings` opens the settings browser (full-port round 33).**
            // Every settable thing with its live value; rows with an action
            // run it through the same helpers the chords and heads run, rows
            // without one report (workspace, session, thinking level).
            Command::Settings => {
                self.open_settings();
                Outcome::Ran("settings — values update live; Enter acts".to_string())
            }
            // **`/share` exports the session and uploads it (full-port round
            // 52).** The export runs on the arm; the `gh` upload runs off
            // the paint thread and reports back on a channel, with an
            // in-between notice so the gap never reads as a hang. The spawn
            // itself is reviewed, not driven (a driven test would upload
            // wherever `gh` happens to exist); the export and the sentence
            // shaping are pinned below.
            Command::Share => {
                let said = self.run_share();
                Outcome::Ran(said)
            }
            // **`/trust` reports and accepts project-config trust (full-port
            // round 53).** The terminal's words over the same two mechanisms:
            // the config hash (per-file integrity, workspace-local) and the
            // remembered path decision (global store). Bare and `no` never
            // write; `yes` writes both, so that arm is reviewed, not driven
            // — a driven test would flip real global trust.
            Command::Trust(arg) => self.run_trust_argument(arg),
            // **`/branch` and `/tree` open the timeline overlay (full-port
            // round 55).** The same view — the label is the only difference
            // that survives, like the terminal. Rows are the store's
            // branch forest, flattened; Enter navigates there and reloads.
            // No chord: the terminal opens its timeline from heads, and
            // heads plus palette carry it here too.
            Command::Timeline => match self.open_timeline("branch") {
                Some(said) => Outcome::Ran(said),
                None => Outcome::Ran("branch — pick an entry, Enter jumps".to_string()),
            },
            Command::Tree => match self.open_timeline("tree") {
                Some(said) => Outcome::Ran(said),
                None => Outcome::Ran("tree — pick an entry, Enter jumps".to_string()),
            },
            // **`/scoped_models` opens the picker on the starred set (full-port
            // round 57).** Same store both front ends read; empty stays a
            // notice naming the starring gesture. The starred rows float to
            // the top through the shared builders, so opening *is* the
            // seeding — no second filter to drift.
            Command::ScopedModels => {
                if self.favorites.is_empty() {
                    Outcome::Ran("scoped-models: no favorites yet — open Ctrl+M and press Ctrl+F on a model to star it, then F2 cycles favorites".to_string())
                } else {
                    self.open_model_picker();
                    Outcome::Ran(format!(
                        "scoped-models: {} favorite(s) — cycling favorites · open Ctrl+M to pick, Ctrl+F to star/unstar",
                        self.favorites.len()
                    ))
                }
            }
            // **`/templates` lists prompt templates (full-port round 56).**
            // The terminal's list with its words — name, description, and
            // argument hint per row — over the same stateless discovery the
            // composer expands through. Descriptions, not just names: a
            // template's whole purpose is that you can forget its body but
            // not want to forget what it does.
            Command::Templates => self.run_templates_list(),
            // **`/expand` shows all tool output — or tails again (full-port
            // round 54).** Rows retain everything (same trade as the
            // terminal's cards); the pane shows the shared 40-line tail with
            // a marker until this flips. No chord: `Ctrl+X` is cut inside
            // text fields, and stealing it would break editing to save
            // typing — the head and the palette carry it, like `/copy`.
            Command::Expand => {
                self.expand_tool_output = !self.expand_tool_output;
                Outcome::Ran(if self.expand_tool_output {
                    "all tool output expanded".to_string()
                } else {
                    "tool output collapsed to tails".to_string()
                })
            }
            // **`/search` finds text in the transcript (full-port round 51).**
            // An overlay over the live rows with the query pre-filled, so
            // refining it never means leaving and retyping; choosing a match
            // scrolls the transcript there through the row offsets. Same
            // overlay manners as the rest (one at a time, Esc closes).
            Command::Search(query) => {
                self.open_search(query);
                Outcome::Ran("search — type, Enter jumps".to_string())
            }
            // **`/import` switches the session file live (full-port round 48).**
            // The terminal's shape — usage on bare (no browser here, unlike
            // `/resume`), same busy and missing guards, same switch through
            // `switch_session` — with its own words ("imported", not
            // "resumed"): the state-changing core is shared, the verb is not.
            Command::Import(path) => self.run_import_session(path),
            // **`/help` opens the command palette.** The terminal's `/help` prints
            // the command reference; the window's equivalent surface is the
            // palette, which lists every file and every command this window
            // answers. Opening it is what "show me what I can do" means here.
            Command::Help => {
                self.open_palette();
                Outcome::Ran("palette — every file and command, type to filter".to_string())
            }
            // **`/tasks` opens the pane the activity bar already draws (M312).** The window has
            // had a subagents pane since it was built and a button beside the files one; the
            // slash command the terminal has went on refusing it by name. This is the arm that
            // costs nothing — the effect is a `set_sidebar_view` the button already performs.
            Command::Tasks => {
                self.set_sidebar_view(crate::app::SidebarView::Tasks);
                Outcome::Ran("subagents — this session's dispatched agents".to_string())
            }
            // **`/session` reports rather than opens (M312).** The terminal's prints the file,
            // the leaf and the entry count; a window has no leaf to speak of, so this says what
            // it can see — the file, the model, the context and the state.
            Command::Session => Outcome::Ran(self.session_summary()),
            // **`/clear` empties the view and keeps the totals (M312)**, exactly as the
            // terminal's does and for the same reason: the rows are a projection of the
            // session file, and the figures come from the session rather than from them.
            Command::Clear => {
                self.transcript.clear();
                Outcome::Ran("transcript cleared — the session file keeps everything".to_string())
            }
            // **`/theme` in the window (M312).** The window had no way to change its own palette:
            // the colour scheme was whatever `IdeTheme::resolve(None)` picked at launch, so a user
            // on a dark desk who wanted the light one had to edit a config file and restart. The
            // terminal's `/theme` was refused here by name, which is the honest default and also
            // the whole gap — and with a third palette it is wider.
            //
            // Same shape as the terminal's, deliberately: apply to the live theme and **do not
            // write config**, because a cosmetic change must not quietly rewrite a file the user
            // may have under version control. Bare flips brightness and leaves high contrast
            // alone (`flipped()` is dark↔light by design); a name selects any of the three.
            Command::Theme(argument) => self.run_theme_argument(argument),

            // ---- everything else is the terminal's, and saying so is the feature ----
            //
            // `other` covers every command outside SUPPORTED, because SUPPORTED and these
            // arms name the same commands and
            // `the_window_runs_exactly_the_heads_it_declares_supported` is what holds them
            // to it. A head the window does not have is refused *by name* rather than
            // forwarded to the model: a window that silently swallows a command the
            // terminal implements tells the reader the command does not exist, which is a
            // different and wrong sentence. The name is the reader's own spelling, so the
            // refusal has no name to get wrong.
            other => Outcome::Refused(refusal_for(&other, typed_head(text))),
        }
    }

    /// The `/goal` argument, which is a sentence rather than a subcommand.
    ///
    /// The tail goes whole, for the reason the terminal's parser documents: splitting an
    /// objective in the parser is how a goal ends up reading "make the doc".
    ///
    /// **The parsing is the terminal's** — `epsilon_runtime::goal_command`, moved there at M267
    /// precisely so this window is not the second implementation. Before that, every verb other
    /// than the four the palette already had was refused *by name*, because the rules lived in a
    /// frontend this crate is not.
    fn run_goal_argument(&mut self, argument: Option<String>) -> Outcome {
        use epsilon_runtime::GoalCommand;

        let parsed = epsilon_runtime::goal_command(argument.as_deref());
        match parsed {
        GoalCommand::Show => match self.goal.as_ref() {
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
            None => Outcome::Ran(
                "no goal is set — `/goal <what to achieve>` starts one, `/goal help` lists the verbs"
                    .into(),
            ),
        },
        GoalCommand::Help(lines) => {
            // One transcript row per line, because this is the window's help for a verb table
            // a keyboard cannot open: `/goal help` is the only way to learn the verbs here.
            for line in lines {
                self.transcript.push_notice(line);
            }
            Outcome::Ran("/goal help".to_string())
        }
        GoalCommand::Invalid(why) => Outcome::Ran(why),
        GoalCommand::Apply(op) => {
            use epsilon_contracts::goal::GoalOp;
            let verb = describe(&op);
            // A created or resumed goal is **armed**, for the same reason the terminal does
            // it: the reader should not have to know that a goal needs arming to continue.
            let starts_work = matches!(op, GoalOp::Create { .. } | GoalOp::Resume);
            let Some(agent) = self.session.as_ref().map(|s| Arc::clone(s.agent())) else {
                return Outcome::Ran("there is no agent attached".to_string());
            };
            match agent.goal_apply(None, op) {
                Ok(Some(view)) => {
                    if starts_work {
                        agent.set_goal_armed(true);
                    }
                    // The band reads the cache, and this changed the thing it caches.
                    self.refresh_goal();
                    Outcome::Ran(format!(
                        "goal {verb} · {} · round {}/{} · {} gate(s)",
                        view.goal.status.as_str(),
                        view.goal.rounds_started,
                        view.goal.max_rounds,
                        view.goal.gates.len(),
                    ))
                }
                Ok(None) => {
                    self.refresh_goal();
                    Outcome::Ran("goal cleared".to_string())
                }
                Err(e) => Outcome::Ran(format!("/goal refused: {e}")),
            }
        }
    }
    }
}

/// Past-tense name of an operation, for the notice.
///
/// Not `{:?}`: a reader is shown this.
fn describe(op: &epsilon_contracts::goal::GoalOp) -> String {
    use epsilon_contracts::goal::GoalOp;
    match op {
        GoalOp::Create { .. } => "created".into(),
        GoalOp::Pause => "paused".into(),
        GoalOp::Resume => "resumed".into(),
        GoalOp::Complete => "completed".into(),
        GoalOp::Clear => "cleared".into(),
        GoalOp::Blocked { .. } => "blocked".into(),
        GoalOp::Edit { .. } => "edited".into(),
    }
}

/// Why a command is not available here, naming it and where it is.
///
/// `name` is the command **as the reader spelled it** ([`typed_head`]), not a name this file
/// holds. A refusal that echoes the word the reader can see in their own composer is
/// unambiguous even when the terminal's canonical spelling differs — `/timeline` is refused
/// as `/timeline`, because `timeline` is the command they asked for, and the product's
/// canonical head for it is `branch`.
///
/// One function, so the wording is one wording — and so the refusal a reader sees is about
/// **this** command, not a generic failure that could be anything.
///
/// **Two different refusals, because they are two different facts.** A command the terminal
/// has and this window does not is a gap in this product, and the reader should know where it
/// works. A command **nobody** has is a typo, and telling them to look in the terminal would
/// send them there for something that is not there either.
fn refusal_for(command: &Command, name: &str) -> String {
    match command {
        Command::Unknown(word) => format!("unknown command '/{word}' — nothing here runs it"),
        _ => format!("/{name} — not in this window; it works in the terminal"),
    }
}

/// What `/session` says, in one line a reader can read off the screen (M312).
///
/// **The window's half of the terminal's `/session`, not a second `/session`.** The terminal
/// prints the file, the current leaf and the entry count; a window has no leaf to speak of and
/// no tree to walk, so this prints the file, the model, the context gauge and the state — the
/// four facts this window actually holds, in the terminal's one-line shape.
///
/// Everything is `Option`-guarded because all of it is: a window can be opened with no session,
/// and a session can be running with no durable file (`--faux`, a child agent). **A missing
/// piece is named rather than omitted**, because a `/session` line that silently drops the model
/// reads as "there is no model".
impl IdeApp {
    fn session_summary(&self) -> String {
        let Some(path) = self.session.as_ref().and_then(|s| s.session_path()) else {
            return "session: no durable session file attached to this window".to_string();
        };
        let mut parts = vec![format!("session: {}", path.display())];
        if let Some(m) = self.metrics.as_ref() {
            parts.push(m.model_label.clone());
            parts.push(m.context_gauge());
            // **Words, not arrows** — see [`crate::app::token_figures`]: egui's bundled font has
            // no `U+2191`/`U+2193`, and `/session` is the one command whose whole job is to
            // report numbers a reader is meant to trust.
            parts.push(IdeApp::token_figures(m));
        }
        parts.push(self.run_state().to_string());
        parts.join("  ·  ")
    }

    /// Start a new session file and switch to it.
    ///
    /// The terminal's `/new` (M221) as a window act: name a file beside the
    /// others with the runtime's own [`epsilon_runtime::fresh_session_file`] —
    /// not a second naming rule — create it under a lease, and hand over through
    /// [`IdeApp::switch_session`], the path the sidebar's clicks already take.
    /// Refuses while a run is in flight: switching under a running turn would
    /// orphan its writes into the fresh file. The terminal refuses the same
    /// case the same way ("new: busy").
    ///
    /// Shared by the `/new` head and the sessions pane's New session button:
    /// one act, two doors. `pub(crate)` for the pane; the head owns the words.
    pub(crate) fn run_new_session(&mut self) -> Outcome {
        let Some(session) = self.session.as_ref() else {
            return Outcome::Ran("new session: no agent attached to this window".to_string());
        };
        if session.running() {
            return Outcome::Ran(
                "new: busy — wait for the current run to finish, then /new again".to_string(),
            );
        }
        // The window's own workspace, not the agent's copy of it: the new file
        // must sit beside the sessions the sidebar lists, and the sidebar
        // lists `self.opts.workspace` — a file named under a differently
        // spelled root lands in a different per-copy directory and the pane
        // never shows it (proven by the test failing exactly that way).
        let workspace = self.workspace().to_path_buf();
        let rel = epsilon_runtime::fresh_session_file(
            epsilon_runtime::now_millis(),
            &epsilon_contracts::new_id("ses"),
            &workspace,
        );
        let path = workspace.join(rel);
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return Outcome::Ran(format!("new session: {}: {e}", parent.display()));
        }
        // Create under a lease, then drop the handle: `switch_session` opens
        // the same file again, and holding both would wait on our own lease
        // (the terminal's `/new` documents the same dance).
        if let Err(e) = epsilon_session::SessionStore::open_in(&path, &workspace).map(|_| ()) {
            return Outcome::Ran(format!("new session: {}: {e}", path.display()));
        }
        match self.switch_session(&path) {
            Ok(_) => Outcome::Ran(format!(
                "new session — {} (the previous one is kept; /sessions lists both)",
                path.display()
            )),
            Err(e) => Outcome::Ran(format!("new session failed: {e}")),
        }
    }

    /// Resume a session: open the browser bare, switch by path.
    ///
    /// The terminal's `/resume` as a window act. Bare opens the sessions
    /// sidebar — the browser this window has, the same one `/sessions` shows —
    /// and needs no agent. A path is resolved against the window's workspace
    /// exactly as the terminal resolves against its own, refused while a run
    /// is in flight ("resume: busy", the terminal's words), named when no file
    /// is there ("no file at ... — check the path", also the terminal's), and
    /// handed over through [`IdeApp::switch_session`] otherwise. Resuming the
    /// session already open says so rather than rebuilding it, following the
    /// click path, which skips it for the same reason.
    fn run_resume_session(&mut self, path: String) -> Outcome {
        let p = path.trim();
        if p.is_empty() {
            self.set_sidebar_view(crate::app::SidebarView::Sessions);
            return Outcome::Ran("sessions — click one to resume it".to_string());
        }
        let Some(session) = self.session.as_ref() else {
            return Outcome::Ran("resume: no agent attached to this window".to_string());
        };
        if session.running() {
            return Outcome::Ran(
                "resume: busy — wait for the current run to finish, then /resume again".to_string(),
            );
        }
        let raw = std::path::PathBuf::from(p);
        let abs = if raw.is_absolute() {
            raw
        } else {
            self.workspace().join(raw)
        };
        if !abs.exists() {
            return Outcome::Ran(format!(
                "resume: no file at '{}' — check the path",
                abs.display()
            ));
        }
        if session.session_path().as_deref() == Some(abs.as_path()) {
            return Outcome::Ran(format!("already in {} — nothing to resume", abs.display()));
        }
        match self.switch_session(&abs) {
            Ok(count) => {
                // Name a recorded-vs-running directory mismatch (round 36),
                // in the same sentence as the resume: a session resumed from
                // elsewhere silently reinterprets relative paths.
                let mut said = format!(
                    "resumed {count} messages from {} (history restored, store swapped)",
                    abs.display()
                );
                if let Some(note) = epsilon_core::Agent::resume_cwd_note(&abs, self.workspace()) {
                    said.push_str("; ");
                    said.push_str(&note);
                }
                Outcome::Ran(said)
            }
            Err(e) => Outcome::Ran(format!("resume failed: {e}")),
        }
    }

    /// Import a session file: switch to it live, no restart.
    ///
    /// The terminal's shape with its words — usage on bare (no browser here,
    /// unlike `/resume`), same busy and missing guards — handed over through
    /// the same [`IdeApp::switch_session`] `/resume` uses. The state-changing
    /// core is shared; only the verb ("imported") differs.
    fn run_import_session(&mut self, path: String) -> Outcome {
        let p = path.trim();
        if p.is_empty() {
            return Outcome::Ran(
                "usage: /import <path> — switches session live, no restart needed (same as /resume)".to_string(),
            );
        }
        let Some(session) = self.session.as_ref() else {
            return Outcome::Ran("import: no agent attached to this window".to_string());
        };
        if session.running() {
            return Outcome::Ran(
                "import: busy — wait for the current run to finish, then /import again".to_string(),
            );
        }
        let raw = std::path::PathBuf::from(p);
        let abs = if raw.is_absolute() {
            raw
        } else {
            self.workspace().join(raw)
        };
        if !abs.exists() {
            return Outcome::Ran(format!(
                "import: no file at '{}' — check the path",
                abs.display()
            ));
        }
        match self.switch_session(&abs) {
            Ok(count) => Outcome::Ran(format!(
                "imported {count} messages from {} (store swapped)",
                abs.display()
            )),
            Err(e) => Outcome::Ran(format!("import failed: {e}")),
        }
    }

    /// Branch the current session file under `verb` (`fork` or `clone`).
    ///
    /// The terminal runs both verbs through the store's `fork_branch_at` and
    /// differs only in the word it reports; this window does the same through
    /// one helper, taking the past tense too because English does not form it
    /// with a rule (`forked`, `cloned` — not `cloneed`). A whole-file copy
    /// would duplicate abandoned branches under one identity, so the branch
    /// keeps the leaf's path under a fresh header — pi's `forkFrom`, which is
    /// also what the RPC surface calls. The fork is stored, not switched to:
    /// moving there is `/resume`'s job.
    /// Shared by the `/fork` head, the `/clone` head and the session chord:
    /// one act, three doors. `pub(crate)` for the pane and the keys; the heads
    /// own the words.
    pub(crate) fn run_fork_session(&mut self, verb: &'static str, past: &'static str) -> Outcome {
        let Some(session) = self.session.as_ref() else {
            return Outcome::Ran(format!("{verb}: no agent attached to this window"));
        };
        let Some(store) = session.agent().session_store() else {
            return Outcome::Ran(format!("{verb}: no session store attached"));
        };
        let (path, leaf) = match store.lock() {
            Ok(st) => (st.path().to_path_buf(), st.leaf_id()),
            Err(_) => return Outcome::Ran(format!("{verb}: session store unavailable")),
        };
        let Some(leaf_id) = leaf else {
            return Outcome::Ran(format!("{verb}: nothing to {verb} — session is empty"));
        };
        let dest_dir = match path.parent() {
            Some(d) => d.to_path_buf(),
            None => {
                return Outcome::Ran(format!("{verb}: the session file has no parent directory"));
            }
        };
        let forked = match store.lock() {
            Ok(st) => st.fork_branch_at(&leaf_id, &dest_dir),
            Err(_) => return Outcome::Ran(format!("{verb}: session store unavailable")),
        };
        match forked {
            Ok(dest) => Outcome::Ran(format!(
                "{past} session → {} (resume it from /sessions)",
                dest.display()
            )),
            Err(e) => Outcome::Ran(format!("{verb} failed: {e}")),
        }
    }

    /// Write the session to HTML or JSON.
    ///
    /// Same formats, same default folder, same words as the terminal's: the
    /// spelling comes from the shared [`epsilon_contracts::command::split_export_arg`]
    /// and the rendering from the shared `epsilon_session::export` — this arm
    /// is only the window's half (workspace root, notices) around both.
    fn run_export_session(&mut self, arg: Option<String>) -> Outcome {
        let Some(session) = self.session.as_ref() else {
            return Outcome::Ran("export: no agent attached to this window".to_string());
        };
        let (fmt, raw_path) =
            epsilon_contracts::command::split_export_arg(arg.as_deref().unwrap_or(""));
        let workspace = self.workspace().to_path_buf();
        let dest = if raw_path.is_empty() {
            let id = epsilon_contracts::new_id("export");
            let ext = if fmt == "html" { "html" } else { "json" };
            workspace
                .join(".epsilon")
                .join("exports")
                .join(format!("session-{id}.{ext}"))
        } else {
            let p = std::path::PathBuf::from(&raw_path);
            if p.is_absolute() {
                p
            } else {
                workspace.join(p)
            }
        };
        let result = (|| -> Result<usize, String> {
            let store = session
                .agent()
                .session_store()
                .ok_or("no session store attached")?;
            let guard = store.lock().map_err(|_| "store unavailable")?;
            if fmt == "html" {
                epsilon_session::export::write_html(&guard, &dest)
            } else {
                epsilon_session::export::write_json(&guard, &dest)
            }
        })();
        match result {
            Ok(n) => Outcome::Ran(format!(
                "exported {} → {} ({} entries)",
                if fmt == "html" { "html" } else { "json" },
                dest.display(),
                n
            )),
            Err(e) => Outcome::Ran(format!("export failed: {e}")),
        }
    }

    /// Name the running model bare, switch it with an id.
    ///
    /// An id runs the terminal's act through the shared [`epsilon_core::Agent::set_model_id`]
    /// — the session record happens inside, for every front end (M303) — and
    /// carries the terminal's words (`model already …`, `model switched to …`)
    /// and its key warning, but not its machine-wide default save: that file
    /// is the terminal's default for *new* sessions, and a window action does
    /// not rewrite it behind the terminal's back.
    ///
    /// Shared by the `/model <id>` head and the model picker: one act, two
    /// doors. `pub(crate)` for the picker; the head owns the words.
    pub(crate) fn run_model_argument(&mut self, arg: Option<String>) -> Outcome {
        let Some(session) = self.session.as_ref() else {
            return Outcome::Ran("model: no agent attached to this window".to_string());
        };
        let agent = session.agent();
        let current = agent.model_id();
        let Some(id) = arg.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            return Outcome::Ran(format!("model {current}"));
        };
        if id == current {
            return Outcome::Ran(format!("model already {id}"));
        }
        agent.set_model_id(id);
        let mut said = format!("model switched to {id}");
        if !epsilon_llm::catalog::credentials_ready(id) {
            said.push_str(" (no key yet)");
        }
        Outcome::Ran(said)
    }

    /// Flip the palette bare, select it by name.
    ///
    /// Extracted from the `/theme` arm unchanged so the settings browser can
    /// offer the same act: one flip, two doors.
    pub(crate) fn run_theme_argument(&mut self, argument: Option<String>) -> Outcome {
        use epsilon_tui::ui::theme::ThemeMode;
        let next = match argument.as_deref().map(str::trim) {
            None | Some("") => self.theme().palette.flipped(),
            Some(name) => match ThemeMode::from_name(name) {
                Some(mode) => mode,
                None => {
                    return Outcome::Ran(format!(
                        "unknown palette '{name}' — use `dark`, `light` or \
                         `high-contrast`, or bare `/theme` to flip"
                    ));
                }
            },
        };
        let was = self.set_palette(next).palette;
        Outcome::Ran(format!("palette: {} → {}", was.name(), next.name()))
    }

    /// Store a pasted provider key; refuse what needs the terminal.
    ///
    /// The terminal's act through the shared [`epsilon_llm::auth::save_credential`]
    /// — same file, same words — minus its overlays: subscription ceremonies
    /// need the browser flow, and bare or keyless forms need its picker and
    /// masked prompt. The usage names the shape that works here. The typed
    /// line is never echoed (commands answer; only prompts echo), so the key
    /// reaches no transcript and no file but the credential store itself.
    fn run_login(&mut self, provider: String, key: String) -> Outcome {
        let provider = provider.trim().to_string();
        if !provider.is_empty() && epsilon_llm::oauth_login::is_oauth_ceremony_arg(&key) {
            return Outcome::Ran(format!(
                "subscription login for '{provider}' needs the terminal — run `epsilon auth login {provider}` there"
            ));
        }
        if provider.is_empty() {
            return Outcome::Ran(
                "usage: /login <provider> <key> — e.g. /login anthropic sk-ant-...".to_string(),
            );
        }
        if key.trim().is_empty() {
            return Outcome::Ran(format!(
                "usage: /login {provider} <key> — paste the key after the provider id"
            ));
        }
        match epsilon_llm::auth::save_credential(&provider, key.trim()) {
            Ok(path) => Outcome::Ran(format!(
                "saved key for '{provider}' → {}  (next request uses it)",
                path.display()
            )),
            Err(e) => Outcome::Ran(format!("not saved for '{provider}': {e}")),
        }
    }

    /// Remove a stored provider key, with the terminal's words.
    ///
    /// Same shared store as the login above: a key removed in either front
    /// end is gone in both.
    fn run_logout(&mut self, provider: String) -> Outcome {
        let provider = provider.trim().to_string();
        if provider.is_empty() {
            return Outcome::Ran("usage: /logout <provider> — e.g. /logout anthropic".to_string());
        }
        match epsilon_llm::auth::remove_credential(&provider) {
            Ok(true) => Outcome::Ran(format!("removed stored key for '{provider}'")),
            Ok(false) => Outcome::Ran(format!("no stored key for '{provider}' (nothing removed)")),
            Err(e) => Outcome::Ran(format!("not removed for '{provider}': {e}")),
        }
    }

    /// Copy the last assistant message to the clipboard.
    ///
    /// The terminal's source (the last assistant row) through the platform
    /// clipboard, with the terminal's words. No clipboard without a painted
    /// frame — the context the frame lent (see the `ctx` field) — and no
    /// assistant message without an answer that says so.
    fn run_copy(&mut self) -> Outcome {
        let Some(text) = self
            .transcript
            .rows()
            .iter()
            .rev()
            .find(|r| r.kind == crate::session::RowKind::Assistant)
            .map(|r| r.text.clone())
        else {
            return Outcome::Ran("copy: no assistant message to copy".to_string());
        };
        let Some(ctx) = self.ctx.clone() else {
            return Outcome::Ran("copy: no window context to copy through".to_string());
        };
        let chars = text.chars().count();
        ctx.copy_text(text);
        Outcome::Ran(format!("copied {chars} chars to the clipboard"))
    }

    /// Re-read skills and keybindings from disk.
    ///
    /// The terminal's `/reload` minus the parts this window does not manage
    /// (prompt templates, agent defs — both load at composition and have no
    /// window reader): skills re-discover through the shared
    /// `discover_with_notices` and land on the shared agent, keybindings
    /// re-read so a fixed file applies without a restart. Discovery lines
    /// forward verbatim (they name root and reason); the summary names the
    /// counts. Read-only throughout — discovery opens directories, never
    /// writes — so this is safe to drive in tests.
    fn run_reload(&mut self) -> Outcome {
        let agent = match self.session.as_ref() {
            Some(session) => session.agent(),
            None => {
                return Outcome::Ran("reload: no agent attached to this window".to_string());
            }
        };
        let mut roots = Vec::new();
        if let Some(dir) = epsilon_core::config::global_dir() {
            roots.push(dir.join("skills"));
        }
        roots.push(self.workspace().join(".agents").join("skills"));
        roots.push(self.workspace().join(".epsilon").join("skills"));
        let found = epsilon_core::skills::discover_with_notices(&roots);
        let n = found.skills.len();
        agent.set_skills(found.skills);
        let mut lines: Vec<String> = found.notices;
        lines.push(format!("reload: {n} skill(s)"));
        self.keys = crate::app::window_keys();
        if let Some(warning) = self.keys.load_warning() {
            lines.push(warning.to_string());
        } else {
            lines.push("keybindings re-read".to_string());
        }
        Outcome::Ran(lines.join("\n"))
    }

    /// List discovered skills with their state.
    ///
    /// The terminal's list with its words: one `name — description` line per
    /// skill (plus `(manual only)` and `[off]` where they apply), then the
    /// enabled count and the toggle hint. Empty stays an invitation, not a
    /// blank.
    fn run_skills_list(&mut self) -> Outcome {
        let skills = match self.session.as_ref() {
            Some(session) => session.agent().skills(),
            None => {
                return Outcome::Ran("skills: no agent attached to this window".to_string());
            }
        };
        if skills.is_empty() {
            return Outcome::Ran(
                "no skills discovered — drop SKILL.md dirs into .epsilon/skills/".to_string(),
            );
        }
        let disabled = epsilon_core::config::load_disabled_skills();
        let mut lines: Vec<String> = skills
            .iter()
            .map(|s| {
                let mut line = format!("{} — {}", s.name, s.description);
                if s.disable_model_invocation {
                    line.push_str(" (manual only)");
                }
                if disabled.iter().any(|d| d == &s.name) {
                    line.push_str(" [off]");
                }
                line
            })
            .collect();
        let on = skills.len().saturating_sub(disabled.len());
        lines.push(format!(
            "{on} of {} enabled · toggle one with `/skill <name> off|on`",
            skills.len()
        ));
        Outcome::Ran(lines.join("\n"))
    }

    /// Switch one skill off or on.
    ///
    /// The terminal's parse and words through the shared disabled-skills
    /// store: unknown names and bad shapes never reach storage. Flipping a
    /// real switch writes the global store, so that arm is reviewed, not
    /// driven — the same reason no test drives the favorites toggle.
    fn run_skill_toggle(&mut self, arg: String) -> Outcome {
        let arg = arg.trim();
        let (name, want_off) = match arg.rsplit_once(char::is_whitespace) {
            Some((n, "off")) | Some((n, "disable")) => (n.trim(), true),
            Some((n, "on")) | Some((n, "enable")) => (n.trim(), false),
            _ => {
                return Outcome::Ran(
                    "usage: /skill <name> on|off — `/skills` lists what is available".to_string(),
                );
            }
        };
        if name.is_empty() {
            return Outcome::Ran("usage: /skill <name> on|off".to_string());
        }
        let Some(session) = self.session.as_ref() else {
            return Outcome::Ran("skill: no agent attached to this window".to_string());
        };
        let known = session.agent().skills().iter().any(|s| s.name == name);
        if !known {
            return Outcome::Ran(format!(
                "no skill named '{name}' — `/skills` lists what is available"
            ));
        }
        match epsilon_core::config::set_skill_disabled(name, want_off) {
            Ok(_) => Outcome::Ran(format!(
                "{name} {} — takes effect on the next turn",
                if want_off { "off" } else { "on" }
            )),
            Err(e) => Outcome::Ran(format!("could not change {name}: {e}")),
        }
    }

    /// Show the workspace changelog's head.
    ///
    /// The terminal's read with its words: workspace `CHANGELOG.md`, else
    /// `docs/CHANGELOG.md`, header line plus the first ~120 lines and an
    /// overflow line — or the honest absence. One answer, joined: the
    /// transcript shows the lines, the file keeps the history.
    fn run_changelog(&mut self) -> Outcome {
        let candidates = [
            self.workspace().join("CHANGELOG.md"),
            self.workspace().join("docs").join("CHANGELOG.md"),
        ];
        let mut found: Option<(std::path::PathBuf, String)> = None;
        for candidate in candidates {
            if let Ok(raw) = std::fs::read_to_string(&candidate)
                && !raw.trim().is_empty()
            {
                found = Some((candidate, raw));
                break;
            }
        }
        let Some((path, raw)) = found else {
            return Outcome::Ran(
                "changelog: no CHANGELOG.md in workspace (see docs/milestones/ for milestone logs)"
                    .to_string(),
            );
        };
        let lines: Vec<&str> = raw.lines().collect();
        let take = lines.len().min(120);
        let mut out = vec![format!(
            "changelog: {} ({} lines, showing {} — see file for full history)",
            path.display(),
            lines.len(),
            take
        )];
        out.extend(lines[..take].iter().map(|s| s.to_string()));
        if lines.len() > take {
            out.push(format!(
                "… {} more lines in {}",
                lines.len() - take,
                path.display()
            ));
        }
        Outcome::Ran(out.join("\n"))
    }

    /// Preview a file checkpoint, or apply it on a trailing `yes`.
    ///
    /// The terminal's two shared calls with its words and its guards: usage
    /// on bare, busy refusal while a run owns the workspace, preview first
    /// (the sentence that lets a reader decide), apply only on confirmation.
    /// The `yes` must be last — a file called `yes` is a file.
    fn run_restore_file(&mut self, argument: String) -> Outcome {
        let tail = argument.trim();
        if tail.is_empty() {
            return Outcome::Ran(
                "usage: /restore <path> — preview the checkpoint for a file, then \
                 `/restore <path> yes` to put it back"
                    .to_string(),
            );
        }
        if let Some(session) = self.session.as_ref()
            && session.running()
        {
            return Outcome::Ran(
                "restore: busy — wait for the current run to finish, then /restore again"
                    .to_string(),
            );
        }
        let (path, confirmed) = match tail.rsplit_once(char::is_whitespace) {
            Some((p, word)) if word.eq_ignore_ascii_case("yes") => (p.trim(), true),
            _ => (tail, false),
        };
        let answer = if confirmed {
            epsilon_runtime::restore_checkpoint(self.workspace(), path)
        } else {
            epsilon_runtime::describe_checkpoint(self.workspace(), path)
        };
        match answer {
            Ok(sentence) => Outcome::Ran(format!("restore: {sentence}")),
            Err(why) => Outcome::Ran(format!("restore: {why}")),
        }
    }

    /// List discovered prompt templates with their descriptions.
    ///
    /// The terminal's list with its words: one `/name — description` line
    /// per template (plus argument hints where they apply), or the honest
    /// absence. Read-only throughout — discovery opens files, never writes
    /// — so this is safe to drive in tests.
    fn run_templates_list(&mut self) -> Outcome {
        let templates = match self.session.as_ref() {
            Some(_) => self.discover_templates(),
            None => {
                return Outcome::Ran("templates: no agent attached to this window".to_string());
            }
        };
        if templates.is_empty() {
            return Outcome::Ran(
                "no prompt templates — drop .md files into .epsilon/prompts/ or .agents/prompts/"
                    .to_string(),
            );
        }
        let mut out: Vec<String> = Vec::new();
        for t in &templates {
            let mut line = format!("/{} — {}", t.name, t.description);
            if let Some(hint) = &t.argument_hint {
                line.push_str(&format!(" (takes {hint})"));
            }
            out.push(line);
        }
        Outcome::Ran(format!("prompt templates: {}", out.join(" · ")))
    }

    /// Report project-config trust bare, accept it on `yes`.
    ///
    /// The terminal's words over the same two mechanisms: the config hash
    /// (workspace-local integrity) and the remembered path decision (same
    /// global store both front ends read — `epsilon_tui::trust_store` is
    /// shared code that happens to live in the TUI crate, and moving it to
    /// the core is a refactor for its third consumer to force). Bare and
    /// `no` never write; `yes` writes both, so that arm is reviewed, not
    /// driven. The window holds no pending-review flag to clear.
    fn run_trust_argument(&mut self, arg: Option<String>) -> Outcome {
        if self.session.is_none() {
            return Outcome::Ran("trust: no agent attached to this window".to_string());
        }
        let answer = arg.as_deref().unwrap_or("").trim().to_ascii_lowercase();
        if answer.is_empty() {
            return match epsilon_core::config::current_project_config_hash_in(self.workspace()) {
                None => Outcome::Ran(
                    "trust: no project config (.epsilon/config.toml not found) — nothing to trust"
                        .to_string(),
                ),
                Some(hash) => {
                    let trust_path = self.workspace().join(".epsilon").join("trust.json");
                    match std::fs::read_to_string(&trust_path) {
                        Ok(raw) => {
                            let prev = serde_json::from_str::<serde_json::Value>(&raw)
                                .ok()
                                .and_then(|v| {
                                    v.get("config_sha256")
                                        .and_then(|h| h.as_str())
                                        .map(|s| s.to_string())
                                });
                            match prev {
                                Some(prev) if prev == hash => Outcome::Ran(format!(
                                    "trust: config is trusted (hash {}…)",
                                    &hash[..8.min(hash.len())]
                                )),
                                Some(prev) => Outcome::Ran(format!(
                                    "trust: config changed (prev {}… current {}…) — /trust yes to accept, /trust no to keep untrusted",
                                    &prev[..8.min(prev.len())],
                                    &hash[..8.min(hash.len())]
                                )),
                                None => Outcome::Ran(
                                    "trust: trust file unreadable — /trust yes to accept"
                                        .to_string(),
                                ),
                            }
                        }
                        Err(_) => Outcome::Ran(format!(
                            "trust: config present (hash {}…) but not yet trusted — /trust yes to accept",
                            &hash[..8.min(hash.len())]
                        )),
                    }
                }
            };
        }
        if answer == "yes" || answer == "y" {
            // Both halves report, like the terminal's two notices: a store
            // sentence that failed must not swallow the hash answer. No
            // owned copy: the workspace is only ever borrowed here, and a
            // `.clone()` on its reference would clone the reference.
            let mut said = Vec::new();
            if let Some(store) = epsilon_tui::trust_store::ProjectTrustStore::open() {
                match store.set(self.workspace(), Some(true)) {
                    Ok(()) => {
                        said.push("project path trusted (remembered for future runs)".to_string())
                    }
                    Err(e) => said.push(format!("trust store note: {e}")),
                }
            }
            match epsilon_core::config::current_project_config_hash_in(self.workspace()) {
                Some(hash) => {
                    match epsilon_core::config::accept_project_config_hash_in(
                        self.workspace(),
                        &hash,
                    ) {
                        Ok(path) => said.push(format!(
                            "trust accepted — hash {}… saved to {}",
                            &hash[..8.min(hash.len())],
                            path.display()
                        )),
                        Err(e) => said.push(format!("trust accept failed: {e}")),
                    }
                }
                None => said.push("trust: no config file to accept".to_string()),
            }
            return Outcome::Ran(said.join("\n"));
        }
        if answer == "no" || answer == "n" {
            return Outcome::Ran(
                "trust: not accepted — config stays untrusted (edits still require /trust yes)"
                    .to_string(),
            );
        }
        Outcome::Ran("usage: /trust [yes|no]".to_string())
    }

    /// Show the session display name bare, set it with an id.
    ///
    /// The terminal's act with its words over the same `session_name` custom
    /// entry: bare reads the persisted tail (the read discovery uses for
    /// every listed row), setting persists beside it. No in-memory copy —
    /// the file is the truth, so a name set in either front end reads in
    /// both, and the notice says which of the two happened (set, or named
    /// for this run only).
    fn run_name_argument(&mut self, arg: Option<String>) -> Outcome {
        let Some(session) = self.session.as_ref() else {
            return Outcome::Ran("name: no agent attached to this window".to_string());
        };
        let name = arg.as_deref().unwrap_or("").trim();
        if name.is_empty() {
            let shown = session
                .session_path()
                .and_then(|p| epsilon_session::discover::display_name(&p));
            return match shown {
                Some(n) => Outcome::Ran(format!("session name: '{n}'")),
                None => Outcome::Ran(
                    "usage: /name <display name> — sets session display name".to_string(),
                ),
            };
        }
        let agent = session.agent();
        let persisted = (|| -> Result<(), String> {
            let store = agent.session_store().ok_or("no session store attached")?;
            let mut st = store.lock().map_err(|_| "store lock".to_string())?;
            st.append(epsilon_contracts::session_entry::EntryKind::Custom {
                custom_type: "session_name".into(),
                data: Some(serde_json::json!({"name": name})),
            })
            .map_err(|e| format!("{e}"))?;
            Ok(())
        })();
        match persisted {
            Ok(()) => Outcome::Ran(format!("session name set to '{name}'")),
            Err(why) => Outcome::Ran(format!(
                "session named '{name}' for this run, but not saved: {why}"
            )),
        }
    }

    /// Show the thinking level bare, set it with an id.
    ///
    /// Mirrors the terminal's words (`thinking level: …`, `unknown level …`)
    /// over the same shared [`epsilon_contracts::ThinkingLevel`] parse. No
    /// picker half exists to refuse: the level is one of seven names, and
    /// typing one is the whole interface.
    fn run_think_argument(&mut self, arg: Option<String>) -> Outcome {
        let Some(session) = self.session.as_ref() else {
            return Outcome::Ran("think: no agent attached to this window".to_string());
        };
        let agent = session.agent();
        match arg.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => Outcome::Ran(format!(
                "thinking level: {}",
                agent.thinking_level().as_str()
            )),
            Some(word) => match epsilon_contracts::ThinkingLevel::parse(word) {
                Some(level) => {
                    agent.set_thinking_level(level);
                    Outcome::Ran(format!("thinking level: {}", level.as_str()))
                }
                None => {
                    let levels = epsilon_contracts::ThinkingLevel::ALL
                        .iter()
                        .map(|l| l.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    Outcome::Ran(format!("unknown level '{word}' - use: {levels}"))
                }
            },
        }
    }
}

/// The head the reader typed, for the refusal.
///
/// **Three lines of tokenising, and no table.** This is the only place the window needs a
/// name at all, and the reader has already written it; the parse that decided this is a
/// command the window lacks has already happened, and
/// [`parse_command`](epsilon_contracts::command::parse_command) splits the head the same way
/// (`crates/epsilon-contracts/src/command.rs:464`). Mirroring a tokeniser is not a second command
/// registry — a registry is the list of commands, and that list now lives in `epsilon-contracts`,`r`n/// the one crate both front ends read.
///
/// The case that cannot arise: `parse_command` returned `None`, so there was no `/` to strip
/// and no head to echo. The `unwrap_or` keeps that a total function rather than a panic on
/// the one path that must never panic.
fn typed_head(line: &str) -> &str {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix('/').unwrap_or(trimmed);
    rest.split_whitespace().next().unwrap_or(rest)
}

/// The command's canonical head in the shared table, **looked up rather than written
/// down**.
///
/// This used to be a 38-arm `match`, one line per [`Command`] variant: a third copy of the
/// variant list, next to `enum Command` and the parser's own arms. Adding a command to the
/// terminal broke this file's build, which is a good property — but it was bought by
/// restating the table rather than by reading it.
///
/// So it reads the table. [`COMMAND_HEADS`] is the terminal's own list of heads
/// (`crates/epsilon-contracts/src/command.rs:371`), and the variant a head parses to *is* the
/// variant that head names. [`std::mem::discriminant`] compares them, so an argument makes no
/// difference to the name: `/model` and `/model gpt` are both `model`.
///
/// **Both spellings of a head are tried, and that is load-bearing.** The parser returns
/// `Command::Models` for a bare `/model` and `Command::Model(..)` for `/model gpt`
/// (`command.rs:472`), so a lookup that asked only for the bare form would fail to answer for
/// the argument-taking half of any command written that way.
///
/// `None` means no head in the shared table names this command — which is
/// [`Command::Unknown`] and nothing else the parser can produce, and which
/// [`window_runs`] reads as "not supported".
fn head_of(command: &Command) -> Option<&'static str> {
    let wanted = std::mem::discriminant(command);
    COMMAND_HEADS.iter().copied().find(|head| {
        [format!("/{head}"), format!("/{head} x")]
            .iter()
            .filter_map(|spelling| parse_command(spelling))
            .any(|parsed| std::mem::discriminant(&parsed) == wanted)
    })
}

/// Whether **this window** runs `command` itself — the capability answer, and the only place
/// [`SUPPORTED`] is read.
///
/// **Stated plainly, because it is the part of this file a reviewer should check:** the
/// window's own run/refuse split is the three arms in [`IdeApp::try_command`], not this
/// predicate. What this predicate owns is the *exported* answer — the refusal set the guard
/// test drives — and the two are held together by
/// `the_window_runs_exactly_the_heads_it_declares_supported`, which fails if a name is added
/// to `SUPPORTED` with no arm to run it. Routing the split through the data instead is the
/// registry ADR-024 defers; until then the declaration and the implementation are two
/// statements of one fact and one test says so.
fn window_runs(command: &Command) -> bool {
    head_of(command).is_some_and(|head| SUPPORTED.contains(&head))
}

/// Every command head the window **refuses**, for the test that the refusal table and the
/// parser have not drifted apart.
///
/// **Derived, in the direction the reader meets it:** every head the parser knows, minus the
/// heads this window runs. There is no list here to keep in step — the heads come from the
/// shared table and the subtraction comes from [`window_runs`], which reads
/// [`SUPPORTED`].
///
/// The heads are the canonical spellings from [`COMMAND_HEADS`] — the names the
/// product advertises, which is what a guard wants to drive. An alias (`/timeline` for
/// `/branch`, `/q` for `/exit`) parses to the same command and is refused with a sentence
/// naming the alias the reader typed, so listing them here would drive the same refusal a
/// second time under a different spelling.
pub fn refused_heads() -> Vec<&'static str> {
    COMMAND_HEADS
        .iter()
        .copied()
        .filter(|head| {
            parse_command(&format!("/{head}")).is_some_and(|command| !window_runs(&command))
        })
        .collect()
}

/// Heads this window answers itself. Everything else in [`COMMAND_HEADS`] is refused
/// by name, which is the honest default.
///
/// **This is the only hand-written statement of the window's capability in the tree**, and it
/// is a declaration rather than an implementation: [`IdeApp::try_command`] holds the arms that
/// run exactly these heads, and `the_window_runs_exactly_the_heads_it_declares_supported` and
/// `every_head_the_parser_knows_is_answered_or_refused_by_name` are what keep those two
/// statements of one fact in step. Everything else about a command — which spellings reach it,
/// what they parse to, what the refusal calls it — is read from the terminal or from the
/// reader's own line.
///
/// A command the window can run is one the terminal can run, so this set is a **subset** by
/// construction and not by luck.
///
/// **Do not grow it to fix a gap in a test.** Each name added is a verb this window stops
/// refusing by name, which is a product decision with an ADR attached — not a way to make an
/// assertion pass.
///
/// **A macro that emits both this and the arms was tried and does not work (M312).** The idea is
/// the right one — one list, no drift — and `macro_rules!` cannot express it: it will not expand
/// inside a match's arm-list, so it would have to write the whole function from **item** position,
/// where its arguments are parsed as *items* and `self.run_compact()` is not an item. The
/// measured failure is `E0424: expected value, found module self` at every arm. The two tests
/// named above are what hold the pair together instead, and the drift this would have prevented
/// is a **red test**, not a shipped lie.
///
/// `/new` is **not** here: the terminal's starts a new session *file* (M221) and the window
/// has no such action, so it is refused with its name like every other unbuilt verb rather
/// than mapped onto the nearest thing that exists.
pub const SUPPORTED: &[&str] = &[
    "branch",
    "changelog",
    "clear",
    "clone",
    "compact",
    "copy",
    "exit",
    "expand",
    "export",
    "fork",
    "goal",
    "help",
    "hotkeys",
    "import",
    "login",
    "logout",
    "model",
    "name",
    "new",
    "reload",
    "restore",
    "resume",
    "scoped_models",
    "search",
    "session",
    "sessions",
    "settings",
    "share",
    "skill",
    "skills",
    "tasks",
    "templates",
    "theme",
    "think",
    "tree",
    "trust",
];

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::command::ALIAS_HEADS;

    /// The two ways a head is typed. Both are driven everywhere below, because the parser
    /// hands back **different variants** for `/model` and `/model gpt`.
    fn both_spellings(head: &str) -> [String; 2] {
        [format!("/{head}"), format!("/{head} x")]
    }

    /// **Every refused head is refused by name.**
    ///
    /// This is the property the file exists for: a window that quietly forwards a command it
    /// cannot run to the model is a window that lies. Asserted on the sentence the reader is
    /// shown, through the same `refusal_for` the window calls, for every head the terminal has
    /// and this window does not — and for both spellings of each.
    #[test]
    fn every_refused_head_is_refused_by_name() {
        let refused = refused_heads();
        // Non-empty, not relative: the window answered its twentieth head in
        // round 40, so "the terminal has more" inverted and the relative
        // bound failed loudly as designed. What must hold is that the window
        // does not answer *everything* — the day it does, the refusal
        // machinery (and this test) retires. The loop below still checks
        // every refused head is refused by name.
        assert!(
            !refused.is_empty(),
            "the window answers every head; retire the refusal path"
        );
        for head in refused {
            for typed in both_spellings(head) {
                let command = parse_command(&typed)
                    .unwrap_or_else(|| panic!("/{head} is listed but `{typed}` does not parse"));
                assert!(
                    !window_runs(&command),
                    "/{head} is in the refusal set and SUPPORTED says the window runs it"
                );
                let sentence = refusal_for(&command, typed_head(&typed));
                // The trailing space is what separates `/skill` from `/skills` and `/model`
                // from `/models`: a substring check without it would pass a refusal that named
                // the wrong command.
                assert!(
                    sentence.contains(&format!("/{head} ")),
                    "`{typed}` is refused as {sentence:?}, which does not name it; a refusal \
                     that names no command is a generic failure that could be anything"
                );
                assert!(
                    sentence.contains("terminal"),
                    "`{typed}` is refused as {sentence:?}, which does not say where it works"
                );
            }
        }
    }

    /// **The supported set is a subset of what the parser knows.**
    ///
    /// A name in [`SUPPORTED`] the parser has never heard of is a permanent, invisible
    /// refusal: `refused_heads` produces no command to drop, the guard test sees no
    /// contradiction because both of its sets agree, and the window has declared a command it
    /// does not have. Checked in both directions — the name is a head the parser knows, and
    /// the head parses to a real command that [`window_runs`] agrees about.
    #[test]
    fn the_supported_set_is_a_subset_of_the_heads_the_parser_knows() {
        assert!(
            !SUPPORTED.is_empty(),
            "a window that runs no command at all is a defect, not a policy"
        );
        for head in SUPPORTED {
            assert!(
                COMMAND_HEADS.contains(head),
                "/{head} is declared supported but `parse_command` does not know it, so the \
                 window would refuse a command it has declared"
            );
            for typed in both_spellings(head) {
                let command = parse_command(&typed).expect("a supported head parses");
                assert!(
                    window_runs(&command),
                    "/{head} parses to {command:?}, whose head is not in SUPPORTED, so \
                     `try_command` would refuse a command SUPPORTED claims the window runs"
                );
                assert_eq!(
                    head_of(&command),
                    Some(*head),
                    "/{head} parses to {command:?}, and the head a command parses from must be \
                     the head it is refused by"
                );
            }
        }
    }

    /// **The refusal set is the parser's heads minus the window's, and nothing else.**
    ///
    /// Computed here by plain subtraction and deliberately *not* by calling `refused_heads`,
    /// so the derived set is checked against the sentence it claims to describe rather than
    /// against itself. The second assertion is the direction that matters: a command added to
    /// the terminal lands in the refusal set with nothing here edited, and a head cannot be
    /// both refused and run.
    #[test]
    fn the_refusal_set_is_the_parser_heads_the_window_lacks() {
        let mut expected: Vec<&'static str> = COMMAND_HEADS
            .iter()
            .copied()
            .filter(|head| !SUPPORTED.contains(head))
            .collect();
        let mut refused = refused_heads();
        refused.sort_unstable();
        expected.sort_unstable();
        assert_eq!(
            refused, expected,
            "refused_heads() is not COMMAND_HEADS minus SUPPORTED, so the guard test is \
             describing a set the window does not actually refuse"
        );
        assert_eq!(
            refused.len() + SUPPORTED.len(),
            COMMAND_HEADS.len(),
            "a head is neither refused nor run, or is counted in both"
        );
    }

    /// **Every spelling the parser accepts resolves to a head in the terminal's table.**
    ///
    /// The canary for [`head_of`]. A command that reached the parser without reaching
    /// `COMMAND_HEADS` would have no head to resolve to, so [`window_runs`] would answer
    /// "the window does not run this" — for a command it should know — and the head would be
    /// missing from [`refused_heads`] as well. Nothing else in this crate would say so.
    ///
    /// **Canonical heads are driven both ways** — bare and with an argument — because the
    /// capability answer has to hold for the argument-taking half too: `/model gpt` is a
    /// different variant from `/model`, and a lookup that only asked for the bare form would
    /// answer "not supported" for the same command either way. **An alias is driven the same
    /// way**, because as of M286 an alias is a spelling and not a reduced grammar: `models`
    /// opens the picker bare and sets a model with an argument, exactly as `model` does
    /// (`command.rs:482`, the argument arm — `:472` is the bare one). It used to be bare-only,
    /// and the test below recorded that — pinning
    /// `/models gpt` coming back `Unknown`, which is the behaviour that was wrong.
    #[test]
    fn every_spelling_the_parser_accepts_resolves_to_a_head_this_window_can_name() {
        for head in COMMAND_HEADS {
            for typed in both_spellings(head) {
                let command = parse_command(&typed).expect("a listed head parses");
                assert!(
                    !matches!(command, Command::Unknown(_)),
                    "/{head} is in COMMAND_HEADS but `{typed}` parses as Unknown, so this \
                     window would refuse a command the product advertises"
                );
                let resolved = head_of(&command).unwrap_or_else(|| {
                    panic!("`{typed}` parses to {command:?}, which no head in COMMAND_HEADS names")
                });
                assert!(
                    COMMAND_HEADS.contains(&resolved),
                    "`{typed}` resolved to /{resolved}, which is not a head COMMAND_HEADS lists"
                );
            }
        }
        for alias in ALIAS_HEADS {
            for typed in both_spellings(alias) {
                let Some(command) = parse_command(&typed) else {
                    continue;
                };
                if matches!(command, Command::Unknown(_)) {
                    // An alias the parser does not accept in this form is a typo, and the
                    // window words it as one. Pinned by the next test.
                    continue;
                }
                let resolved = head_of(&command).unwrap_or_else(|| {
                    panic!("`{typed}` parses to {command:?}, which no head in COMMAND_HEADS names")
                });
                assert!(
                    COMMAND_HEADS.contains(&resolved),
                    "the alias /{alias} resolved to /{resolved}, which is not a head \
                     COMMAND_HEADS lists, so refused_heads() cannot describe the refusal this \
                     spelling gets"
                );
            }
        }
    }

    /// **The refusal names the command the reader spelled.**
    ///
    /// The window prints what the reader typed, and that is a decision rather than an
    /// accident: `/timeline` is refused as `/timeline` even though the terminal's canonical
    /// head for it is `branch`, because the reader can only check a name against the word in
    /// their own composer. An alias is therefore refused under the spelling they used, and
    /// that spelling is still refused for the reason it deserves — named, and pointing at the
    /// terminal.
    ///
    /// Driven over [`ALIAS_HEADS`], because a canonical head has nothing to get
    /// wrong here and an alias is the case.
    #[test]
    fn the_refusal_names_the_command_the_reader_spelled() {
        for alias in ALIAS_HEADS {
            let typed = format!("/{alias}");
            let command = parse_command(&typed).expect("slash input is a command");
            if matches!(command, Command::Unknown(_)) {
                continue;
            }
            let sentence = refusal_for(&command, typed_head(&typed));
            assert!(
                sentence.starts_with(&format!("/{alias} —")),
                "`{typed}` parses to {command:?} and must be refused under the name the reader \
                 typed, got {sentence:?}"
            );
            assert!(
                sentence.contains("terminal"),
                "`{typed}` is refused as {sentence:?}, which does not say where it works"
            );
        }
    }

    /// **A spelling the parser rejects is refused as a typo, whatever any table lists.**
    ///
    /// The obligation is the narrow one: a reader who mistypes must be told the command does
    /// not exist, **not** pointed at the terminal as though it worked there. That is the whole
    /// value of the distinction — "works in the terminal" is only true for a real command, and
    /// sending a reader to look for one is a worse answer than "unknown command".
    ///
    /// **The subject changed in M286, and the old one is worth recording.** This test used
    /// `/models gpt`, on the grounds that `models` was bare-only and an argument therefore
    /// reached no arm. That was a real parser fact and it was pinned here so the refusal could
    /// be worded against it. It was also a **defect**: `models` is declared an alias of `model`,
    /// so `/models gpt` was the obvious thing to type and the answer was "unknown command".
    /// Fixing it (`command.rs:472`) removed the only listed spelling that rejected an argument,
    /// which left this test with nothing to stand on. The property is worth more than the
    /// example, so it now drives a spelling that is a genuine typo and appears in no table.
    #[test]
    fn a_spelling_the_parser_rejects_is_refused_as_a_typo() {
        let typed = "/mode gpt"; // a typo of `model`, in no head list
        let command = parse_command(typed).expect("slash input is a command");
        assert!(
            matches!(command, Command::Unknown(_)),
            "this pins a parser fact the refusal depends on: no table lists `mode`, so it \
             reaches no arm — got {command:?}"
        );
        let sentence = refusal_for(&command, typed_head(typed));
        assert!(sentence.starts_with("unknown command"), "got {sentence:?}");
        assert!(
            !sentence.contains("works in the terminal"),
            "a typo must not be pointed at the terminal, which has no such command: \
             got {sentence:?}"
        );
    }

    /// **The window runs exactly the heads it declares it runs.**
    ///
    /// [`SUPPORTED`] is the declaration and the arms of [`IdeApp::try_command`] are the
    /// implementation, and this is the only place the two are compared. Spelled out by hand
    /// on purpose: the whole point of this file is that the command list is *read*, so a
    /// check deriving both sides from the same source could not see them disagree.
    ///
    /// It fails when a name is added to `SUPPORTED` with no arm to run it — the drift the
    /// guard test in `tests/window.rs` cannot see, because that test compares two lists and
    /// a name the window refuses while claiming to run it appears in neither. **It did exactly
    /// that at M312**, when `/tasks`, `/session` and `/clear` were declared before this list
    /// knew them.
    ///
    /// **This is a third hand-written list, and it is the price of the two others.** A macro
    /// that emitted all three would need to expand inside a match's arm-list, which
    /// `macro_rules!` cannot do — see [`SUPPORTED`] for the measured failure. Two tests hold
    /// the pair together instead, so the drift this would have prevented is a red test rather
    /// than a shipped lie.
    #[test]
    fn the_window_runs_exactly_the_heads_it_declares_supported() {
        let implemented: Vec<&'static str> = [
            Command::Timeline,
            Command::Changelog,
            Command::Clear,
            Command::Clone,
            Command::Compact,
            Command::Copy,
            Command::Quit,
            Command::Expand,
            Command::Export(None),
            Command::Fork,
            Command::Goal(None),
            Command::Help,
            Command::Hotkeys,
            Command::Import(String::new()),
            Command::Login {
                provider: String::new(),
                key: String::new(),
            },
            Command::Logout(String::new()),
            Command::Models,
            Command::Name(None),
            Command::New,
            Command::Reload,
            Command::Restore(String::new()),
            Command::Resume(String::new()),
            Command::ScopedModels,
            Command::Search(String::new()),
            Command::Session,
            Command::Sessions,
            Command::Settings,
            Command::Share,
            Command::Skill(String::new()),
            Command::Skills,
            Command::Tasks,
            Command::Templates,
            Command::Theme(None),
            Command::Think(None),
            Command::Tree,
            Command::Trust(None),
        ]
        .iter()
        .filter_map(|command| head_of(command))
        .collect();
        assert_eq!(
            implemented,
            SUPPORTED.to_vec(),
            "`try_command` runs {implemented:?} and SUPPORTED declares {SUPPORTED:?}; a head \
             declared but not run is refused by the reader's own hand while the product says \
             the window has it"
        );
    }

    /// **A command nobody has is a typo, and is worded as one.**
    ///
    /// The other half of `refusal_for`'s two facts. Anything starting with a slash reaches
    /// the parser as a command attempt, so `/usr/bin/env is broken` comes back as
    /// `Command::Unknown` — and the sentence must not send a reader to the terminal for
    /// something that is not there either.
    #[test]
    fn a_command_nobody_has_is_refused_as_a_typo_not_as_a_gap() {
        let command = parse_command("/usr/bin/env is broken")
            .expect("slash input is always a command attempt");
        assert!(
            matches!(command, Command::Unknown(_)),
            "a word no command claims must come back as Unknown, got {command:?}"
        );
        let sentence = refusal_for(&command, typed_head("/usr/bin/env is broken"));
        assert!(sentence.starts_with("unknown command"), "got {sentence:?}");
        assert!(
            !sentence.contains("terminal"),
            "a typo must not be pointed at the terminal, which does not have it either: \
             got {sentence:?}"
        );
    }
}
