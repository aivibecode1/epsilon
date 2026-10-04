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
//! That third case is the whole point. A window with four commands is a smaller product; a
//! window that quietly forwards the other thirty-six to the model is a window that lies.
//!
//! # Why this file holds no list of commands
//!
//! It held two, and the second was the expensive one. [`SUPPORTED`] is three names and is
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
//!   window that has those pixels: `/sessions`, `/new`, `/theme`.
//! * **terminal-only** — the effect is on a terminal that does not exist here: `/timeline`,
//!   `/fork`, `/editor`, `/hotkeys`.

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

            // ---- everything else is the terminal's, and saying so is the feature ----
            //
            // `other` covers every command outside SUPPORTED, because SUPPORTED and these
            // three arms name the same three commands and
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
/// run these three, and `the_window_runs_exactly_the_heads_it_declares_supported` is what keeps
/// those two statements of one fact in step. Everything else about a command — which
/// spellings reach it, what they parse to, what the refusal calls it — is read from the
/// terminal or from the reader's own line.
///
/// A command the window can run is one the terminal can run, so this set is a **subset** by
/// construction and not by luck.
///
/// **Do not grow it to fix a gap in a test.** Each name added is a verb this window stops
/// refusing by name, which is a product decision with an ADR attached — not a way to make an
/// assertion pass.
///
/// `/new` is **not** here: the terminal's starts a new session *file* (M221) and the window
/// has no such action, so it is refused with its name like every other unbuilt verb rather
/// than mapped onto the nearest thing that exists.
pub const SUPPORTED: &[&str] = &["compact", "goal", "sessions"];

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
        assert!(
            refused.len() > 20,
            "the terminal has far more commands than the window: {refused:?}"
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
    /// [`SUPPORTED`] is the declaration and the three arms in [`IdeApp::try_command`] are the
    /// implementation, and this is the only place the two are compared. Spelled out by hand
    /// on purpose: the whole point of this file is that the command list is *read*, so a
    /// check deriving both sides from the same source could not see them disagree.
    ///
    /// It fails when a name is added to `SUPPORTED` with no arm to run it — the drift the
    /// guard test in `tests/window.rs` cannot see, because that test compares two lists and
    /// a name the window refuses while claiming to run it appears in neither.
    #[test]
    fn the_window_runs_exactly_the_heads_it_declares_supported() {
        let implemented: Vec<&'static str> =
            [Command::Compact, Command::Goal(None), Command::Sessions]
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
