//! The command vocabulary both front ends read: what a `/word` means, and what the
//! product offers when it advertises one.
//!
//! **Why this is here and not in the terminal.** The command half of the registry was pure
//! data and pure text-in/enum-out parsing, and it lived in `epsilon-tui/src/picker.rs` — so
//! `epsilon-desktop` reached across a frontend to read it
//! (`crates/epsilon-desktop/src/commands.rs:79`, `use epsilon_tui::picker::{self, Command};`).
//! That crate's own manifest had already predicted the edge and named the moment it would
//! break: *"The desktop still draws no terminals and depends on no terminal behaviour… If
//! that ever stops being true, the edge is one module."* (`crates/epsilon-desktop/Cargo.toml:47-54`).
//!
//! Contracts is the crate below both front ends and has zero internal dependencies
//! (`AGENTS.md` §2), so this is the one place two front ends can both read without either
//! naming the other. `epsilon-tui` re-exports every item here from `picker`, so
//! `epsilon_tui::picker::parse_command` and `epsilon_tui::picker::Command` keep resolving and
//! **no call site changed** — the same move `QueueMode` made, for the same reason, and it is
//! worth reading `src/queue_mode.rs` as the worked example of the re-export trick.
//!
//! **What this is not.** `ADR-024` records the larger unification — one `REGISTRY` table with
//! per-front-end capability as data, replacing `COMMAND_HEADS`, `ALIAS_HEADS`, `SLASH_ITEMS`,
//! `keybindings::DEFAULTS` and the desktop's `SUPPORTED`/`head_of` — as **designed and not
//! built** (`docs/design/epsilon-decisions.md:1919-1924`). This module is the host that move
//! needs, not the move: the rows are still four hand-maintained tables, they just live in
//! one crate now.
//!
//! **`parse_command` stays a hand-written `match`, and that is a decision, not a leftover.**
//! Rewriting it as a table lookup would delete ~80 lines, and the design pass rejected that
//! as a behaviour change dressed as a refactor: the grammar has four arities (`/model` bare
//! opens the picker, `/model x` sets an id, `/models` is both, and a bare `/login` is valid
//! and means "ask me"), plus `scoped-models`/`scoped_models`. A table would need those spelled
//! out as four rows and would then need the tests to say which row a spelling lands on. The
//! arms move verbatim; `AGENTS.md` §5 — a refactor that also changes behaviour cannot be
//! reviewed as a refactor — is the reason there is no rewrite in this commit.

/// One slash-autocomplete candidate (pi's `AutocompleteItem` + `SlashCommand`):
/// value to insert, display label, and an optional description/argument hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashItem {
    /// Text inserted when the item is chosen (e.g. `/models`).
    pub value: &'static str,
    /// Optional description rendered dim beside the label.
    pub description: &'static str,
}

/// All known slash commands with descriptions (pi's command registry shape;
/// descriptions follow pi/opencode conventions).
pub const SLASH_ITEMS: &[SlashItem] = &[
    SlashItem {
        value: "/models",
        description: "switch model (picker)",
    },
    SlashItem {
        value: "/model ",
        description: "set model by id, e.g. /model kimi-k2.6",
    },
    SlashItem {
        value: "/think ",
        description: "thinking effort: off..max (Ctrl+T cycles)",
    },
    SlashItem {
        value: "/compact",
        description: "summarize older context to reclaim token window",
    },
    SlashItem {
        value: "/timeline",
        description: "browse session branches by seq/lane/snippet, Enter jumps",
    },
    SlashItem {
        value: "/tasks",
        description: "subagent tasks: state, turns, ids, cost (Esc stops them with the run)",
    },
    SlashItem {
        // The description names the bare form first because that is the one a user types to
        // find out what is going on; the verbs are one `/goal help` away.
        value: "/goal ",
        description: "one long-running objective: `/goal <what to achieve>`, bare to show it",
    },
    SlashItem {
        value: "/fork",
        description: "copy current session JSONL to a new branch file",
    },
    SlashItem {
        value: "/skills",
        description: "list discovered skills",
    },
    SlashItem {
        // M196: the toggle, and the description says the exact shape because
        // "toggle a skill" alone does not tell a user whether it takes
        // `/skill commit off` or `/skill off commit`.
        value: "/skill ",
        description: "switch a skill on or off — `/skill <name> on|off`",
    },
    SlashItem {
        value: "/search ",
        description: "search this session's history",
    },
    SlashItem {
        value: "/clear",
        description: "clear transcript (totals kept)",
    },
    SlashItem {
        value: "/help",
        description: "command reference",
    },
    SlashItem {
        value: "/login ",
        description: "set API key for a provider (paste inside TUI)",
    },
    SlashItem {
        value: "/connect ",
        description: "connect a provider — alias for /login",
    },
    SlashItem {
        value: "/auth ",
        description: "provider auth — alias for /login",
    },
    SlashItem {
        value: "/logout ",
        description: "remove stored key for a provider",
    },
    SlashItem {
        value: "/quit",
        description: "exit the session",
    },
    // --- pi parity: additional builtins (G1) ---
    SlashItem {
        value: "/new",
        description: "start a new session",
    },
    SlashItem {
        value: "/resume ",
        description: "resume a session file by path (bare /resume opens the browser)",
    },
    SlashItem {
        value: "/sessions",
        description: "browse session files in this workspace and resume one",
    },
    SlashItem {
        value: "/editor",
        description: "compose this draft in $EDITOR (Alt+E)",
    },
    SlashItem {
        value: "/session",
        description: "show session info and stats",
    },
    SlashItem {
        value: "/tree",
        description: "navigate session tree (switch branches)",
    },
    SlashItem {
        value: "/clone",
        description: "duplicate the current session at current position",
    },
    SlashItem {
        value: "/trust ",
        description: "save project trust decision for future sessions",
    },
    SlashItem {
        value: "/reload",
        description: "reload skills, prompts, and config without restarting",
    },
    SlashItem {
        value: "/export ",
        description: "export session (HTML default, or specify path: .html/.json)",
    },
    SlashItem {
        value: "/import ",
        description: "import and resume a session from a JSONL file",
    },
    SlashItem {
        // Window-only: the terminal is bound to its process directory, so it refuses
        // `/open` by name and says where the door lives. Listed here because a head
        // in COMMAND_HEADS that no surface offers is the gap
        // `every_offered_command_is_reachable_from_slash_items` exists to close —
        // and the description says who answers it, so the offer never lies.
        value: "/open ",
        description: "open a workspace directory (desktop window; the terminal refuses by name)",
    },
    SlashItem {
        // The name is the file and the verb is the argument, because "restore" on its own
        // says which history it means. `/restore <path>` states the plan and changes
        // nothing; the description names the second half so a reader who types the first
        // is not left guessing what applies it.
        value: "/restore ",
        description: "put a file back to its checkpoint — preview, then `/restore <path> yes`",
    },
    SlashItem {
        value: "/share",
        description: "share session as a secret GitHub gist",
    },
    SlashItem {
        value: "/copy",
        description: "copy last agent message to clipboard",
    },
    SlashItem {
        value: "/name ",
        description: "set session display name",
    },
    SlashItem {
        value: "/changelog",
        description: "show changelog entries",
    },
    SlashItem {
        value: "/hotkeys",
        description: "show all keyboard shortcuts",
    },
    SlashItem {
        value: "/settings",
        description: "open settings menu",
    },
    SlashItem {
        value: "/scoped-models",
        description: "enable/disable models for Ctrl+P cycling",
    },
    // The six below were parseable and in COMMAND_HEADS, and offered by no surface:
    // `/help` never printed them and the `/` autocomplete never suggested them. They are
    // here because a command a person is never told about is a command they will not use,
    // and because `every_offered_command_is_reachable_from_slash_items` now fails without
    // them. `branch` and `exit` are the *canonical* spellings — ALIAS_HEADS lists `timeline`
    // and `quit`/`q` as their alternatives, so both words are offered deliberately.
    SlashItem {
        value: "/theme ",
        // **The names here are the names the code accepts, and a test enforces it**
        // (`epsilon-tui`, `every_palette_the_description_advertises_is_one_the_product_has`).
        // This line said "dark, light, high-contrast" while `ThemeMode` had two variants, so
        // `/theme high-contrast` printed "unknown palette" — a command offering an option it
        // refuses, which is the failure ADR-024 exists to prevent, in the one place a reader is
        // told what is possible. **M312 fixed it the other way round**: `high-contrast` is now a
        // real palette with every role at 7:1 on black, so the promise is kept rather than
        // withdrawn.
        //
        // Reachable by name and by `--theme`/`EPSILON_THEME`, and **not** by bare `/theme`, which
        // stays a dark↔light toggle: a key a user presses to change their mind about brightness
        // should not sometimes land on a third thing.
        description: "switch palette: dark, light, high-contrast",
    },
    SlashItem {
        value: "/expand",
        description: "toggle tool output in the transcript",
    },
    SlashItem {
        value: "/templates",
        description: "list prompt templates",
    },
    SlashItem {
        value: "/branch",
        description: "browse session branches by seq/lane/snippet, Enter jumps",
    },
    SlashItem {
        value: "/exit",
        description: "end the session and return to the shell",
    },
    SlashItem {
        value: "/scoped_models",
        description: "enable/disable models for Ctrl+P cycling (underscore spelling)",
    },
    SlashItem {
        value: "/preset",
        description: "show or switch the permission preset (ADR-032)",
    },
];

/// A parsed editor command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Open the model picker.
    Models,
    /// Set the model id directly (everything after `/model `).
    Model(String),
    /// Set thinking effort (level after `/think `; bare `/think` shows current).
    Think(Option<String>),
    /// Compact the session context (manual trigger of the same compaction loop).
    Compact,
    /// Switch the TUI palette at runtime (`/theme` toggles; `/theme light|dark` sets).
    ///
    /// This existed as a possibility for a long time and as a command for none of it:
    /// `ThemeMode::flipped()` carried the doc comment "for the runtime toggle" and
    /// `Theme::set_palette` was public, but the only production caller applied the palette
    /// once at startup from `EPSILON_THEME` or `[tui] theme`. A user who guessed wrong had to
    /// edit a config file and restart to find out.
    Theme(Option<String>),
    /// List the active branch entries (timeline / branch inspection).
    Timeline,
    /// Fork the session at the current leaf into a new session file.
    Fork,
    /// Show the command reference.
    Help,
    /// Clear the transcript pane.
    Clear,
    /// Expand or collapse tool output (`/expand`; same action as Ctrl+X).
    ///
    /// Added so the palette entry can name a real command. It used to carry a
    /// `\u{1}expand` pseudo-command that `parse_command` cannot read — it requires a
    /// leading `/` — so choosing it closed the palette, cleared the composer, and ran
    /// nothing, with no notice. A menu entry that silently does nothing is worse than
    /// a missing one: it tells the user the feature exists.
    Expand,
    /// List discovered skills.
    Skills,
    /// `/skill <name> off` / `/skill <name> on` — switch one skill off or on.
    ///
    /// A slash command rather than a chord, for the reason `/settings` and
    /// `/templates` are commands: it needs a NAME, and a name does not fit on a
    /// key. The name is checked against what is actually discovered, so a typo
    /// says so rather than writing a setting for a skill that does not exist.
    Skill(String),
    /// List installed prompt templates with their descriptions and argument hints.
    Templates,
    /// Search the session log (`/search <query>`); empty tail opens with guidance.
    Search(String),
    /// Store a provider API key (`/login <provider> [key]`; empty key prompts).
    Login {
        /// Provider id as declared in the catalog (`anthropic`, `openai`, …).
        provider: String,
        /// Raw key text (empty → prompt, masked).
        key: String,
    },
    /// Remove a stored provider key (`/logout <provider>`).
    Logout(String),
    /// Start a new session (clear transcript; session file continues; use /fork for a branch file).
    New,
    /// Resume a different session (path hint; live switch is deferred to restart).
    Resume(String),
    /// Open the draft in `$EDITOR` (M59; `Alt+E` / `Ctrl+O` do the same).
    Editor,
    /// Browse the workspace's session files and resume one (M57).
    Sessions,
    /// Show session info and stats.
    Session,
    /// List delegated subagent tasks (state, turns, ids, cost) — M54, cost at M230.
    Tasks,
    /// Navigate session tree (alias for timeline).
    Tree,
    /// Set, show, or manage the session's one long-running goal.
    ///
    /// `/goal <objective>` creates and arms one; `/goal` alone shows it; the verbs are
    /// `edit`, `pause`, `resume`, `complete`, `blocked`, `clear`. The whole argument text is
    /// handed to the command rather than parsed here, because an objective is a sentence and
    /// splitting it in the parser is how a goal ends up reading "make the doc" — the same
    /// reason `/name` and `/search` take their tails whole.
    Goal(Option<String>),
    /// Duplicate the current session at current position.
    Clone,
    /// Trust decision for project config.
    Trust(Option<String>),
    /// Reload skills, prompts, and config.
    Reload,
    /// Export session (tail may contain format/path hint).
    Export(Option<String>),
    /// Import a session JSONL (path in tail).
    Import(String),
    /// Open a workspace directory in the window (`/open <dir>`; D-046/S-105).
    ///
    /// The tail carries a **directory path**, carried whole like `/resume` and `/import`.
    /// Window-only: the terminal is bound to its process directory, so it refuses this
    /// by name and says where the door lives. The executor validates (exists, is a
    /// directory), asks (`yes` switches, anything else repeats without acting), and
    /// switches through a fresh `resolve_launch` + `IdeSession::start` — the same
    /// composition startup runs, so a switch behaves exactly like launching there.
    Open(String),
    /// Put a file back to the checkpoint recorded before the agent's last write or edit.
    ///
    /// The tail carries a **path**, and carrying it whole is the safe choice here for the
    /// same reason it is for `/goal`: a path is a sentence a human types, and splitting it
    /// in the parser is how `src/my file.rs` becomes two arguments and a restore of
    /// nothing. The executor reads a trailing `yes` as the confirmation and treats every
    /// other spelling as "show me what this would do" — so the first press cannot
    /// overwrite anything.
    ///
    /// It is here because the store behind it (`CheckpointStore::restore`, in
    /// `epsilon-tools`) had no production caller at all: the write half ran on every
    /// `write` and `edit` and the revert half was reachable only from its own tests.
    Restore(String),
    /// Share session as a gist.
    Share,
    /// Copy last agent message to clipboard.
    Copy,
    /// Set session display name.
    Name(Option<String>),
    /// Show changelog.
    Changelog,
    /// Show keyboard shortcuts.
    Hotkeys,
    /// Open settings menu.
    Settings,
    /// Enable/disable models for picker cycling.
    ScopedModels,
    /// Show or switch the permission preset (ADR-032).
    Preset(Option<String>),
    /// Exit the session.
    Quit,
    /// A `/word` we do not know — surfaced as an error, never sent to the model.
    Unknown(String),
}

/// Every command head [`parse_command`] understands, one entry per command.
///
/// M171's sweep over all 30 commands kept its own hand-written list, which meant a new
/// command added to [`parse_command`] without updating the test was a silent gap - the
/// exact failure this list exists to remove. Keeping the list here, next to the match it
/// describes, is only half the answer: `every_head_in_command_heads_is_parsed` and
/// `the_parser_does_not_know_a_command_the_list_does_not` together check both directions,
/// so neither an unlisted new command nor a stale entry can survive.
///
/// Aliases (`models` for `model`) are deliberately absent - one entry per command, and the
/// parser resolves the alias to the same variant.
pub const COMMAND_HEADS: &[&str] = &[
    "model",
    "think",
    "compact",
    "theme",
    "branch",
    "tree",
    "goal",
    "fork",
    "clone",
    "new",
    "resume",
    "sessions",
    "editor",
    "session",
    "trust",
    "reload",
    "export",
    "import",
    "open",
    // M277: the rollback half of a checkpoint, and a real command with a real executor.
    //
    // **The reason this line once carried is no longer true, and that matters more than the
    // line.** It used to say that `head_of` in the desktop's `refusal_for` has no catch-all
    // arm, so a new variant would *break that crate's build* rather than be refused by name.
    // M284 replaced `head_of` with a discriminant lookup that returns `Option`: a variant
    // with no head here is simply not found, the window's capability answer is `false`, and
    // the command is **refused by name** — which is the behaviour we want, and which the
    // desktop's own module doc states.
    //
    // So this entry is no longer load-bearing for anyone else's build. It stays because the
    // command is real, and because a command the desktop refuses by name must still be a
    // head this list carries — that is what the reverse drift check below enforces.
    "restore",
    "share",
    "copy",
    "name",
    "changelog",
    "tasks",
    "hotkeys",
    "settings",
    "scoped_models",
    "preset",
    "clear",
    // M194: the palette's "Toggle tool output expand" entry shipped a `\u{1}expand`
    // pseudo-command this parser cannot read, so choosing it ran nothing. Naming a
    // real command is what makes the entry work — and listing it here is what the
    // reverse-drift check compares against the parser.
    "expand",
    "skills",
    // M196: the toggle half. Listed because the reverse-drift check compares
    // SLASH_ITEMS against the parser, and a command the palette does not offer
    // is a command a user has to guess the name of.
    "skill",
    "templates",
    "search",
    "logout",
    "exit",
    // M172: the reverse-drift check found these two on its first run. M167 counted
    // *variants* rather than parser tokens, so it missed seven heads the parser accepts -
    // and so did M171's hand-written list, which is exactly the gap this check exists for.
    "help",
    "login",
];

/// Tokens the parser accepts as spellings of a listed command rather than as commands of
/// their own. Kept beside the list so the reverse drift check has somewhere to put them.
///
/// The sweep drives one head per [`Command`] variant, so an alias is covered through the
/// command it aliases - `/timeline` is swept by `/branch`, which reaches the same arm.
///
/// **`scoped-models` and `?` are here, and the reverse check can now see them.** Both were
/// accepted by [`parse_command`] and named in neither list, so the one check that would have
/// said so could not: its token rule accepted `lowercase`, `_` and `:` only, and those are
/// the parser's only two spellings carrying a `-` and a `?`. A spelling the guard cannot see
/// is a spelling nothing drives — the same gap in a different costume, which is why the
/// filter was widened instead of the two names being left to hide behind it.
pub const ALIAS_HEADS: &[&str] = &[
    "models",   // model
    "timeline", // branch
    "auth",     // login
    "connect",  // login
    "quit",     // exit
    "q",        // exit
    // The dashed spelling of `scoped_models`, which is in COMMAND_HEADS. Offered by no
    // surface and named by no list until now: only `ALIAS_HEADS` says the parser has it.
    "scoped-models", // scoped_models
    "?",             // help
];

/// Parse submitted input that starts with `/`; `None` means ordinary prompt text.
///
/// Slash input is ALWAYS a command attempt (pi/opencode convention): unknown words
/// come back as [`Command::Unknown`] so the UI can correct the user instead of
/// silently prompting the model with literal `/typo`.
pub fn parse_command(input: &str) -> Option<Command> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let (head, tail) = match rest.split_once(' ') {
        Some((h, t)) => (h, t.trim()),
        None => (rest, ""),
    };
    match head.to_ascii_lowercase().as_str() {
        "models" | "model" if tail.is_empty() => Some(Command::Models),
        // **Both spellings take the argument, and the reason is the alternative one used
        // not to.** `models` is in [`ALIAS_HEADS`] as a spelling of `model` and in
        // [`SLASH_ITEMS`] as the picker a reader is offered, so `/models gpt` is the obvious
        // next thing to type — and it fell past this arm to `Unknown`, which is reported as
        // an unknown command. That sentence is false: the command exists, this file lists
        // it, and `/help` prints it. `/model` below is the house shape for a head with an
        // argument, so the alias joins the arm it aliases rather than getting a second
        // grammar of its own — and bare `/models` still opens the picker, which is the one
        // thing the plural spelling is for.
        "models" | "model" => Some(Command::Model(tail.to_string())),
        "think" => Some(Command::Think(if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        })),
        "compact" => Some(Command::Compact),
        "theme" => Some(Command::Theme(if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        })),
        "timeline" | "branch" => Some(Command::Timeline),
        "tree" => Some(Command::Tree),
        "goal" => Some(Command::Goal(if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        })),
        "fork" => Some(Command::Fork),
        "clone" => Some(Command::Clone),
        "new" => Some(Command::New),
        "resume" => Some(Command::Resume(tail.to_string())),
        "sessions" => Some(Command::Sessions),
        "editor" => Some(Command::Editor),
        "session" => Some(Command::Session),
        "trust" => Some(Command::Trust(if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        })),
        "reload" => Some(Command::Reload),
        "export" => Some(Command::Export(if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        })),
        "import" => Some(Command::Import(tail.to_string())),
        "open" => Some(Command::Open(tail.to_string())),
        "restore" => Some(Command::Restore(tail.to_string())),
        "share" => Some(Command::Share),
        "copy" => Some(Command::Copy),
        "name" => Some(Command::Name(if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        })),
        "changelog" => Some(Command::Changelog),
        "tasks" => Some(Command::Tasks),
        "hotkeys" => Some(Command::Hotkeys),
        "settings" => Some(Command::Settings),
        "scoped-models" | "scoped_models" => Some(Command::ScopedModels),
        "preset" => Some(Command::Preset(if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        })),
        "help" | "?" => Some(Command::Help),
        "clear" => Some(Command::Clear),
        "expand" => Some(Command::Expand),
        "skills" => Some(Command::Skills),
        "skill" => Some(Command::Skill(tail.to_string())),
        "templates" => Some(Command::Templates),
        "search" => Some(Command::Search(tail.to_string())),
        "login" | "auth" | "connect" => {
            // `/login <provider> [key]` — key may be pasted; when omitted the
            // TUI prompts with a masked overlay (opencode DialogPrompt parity).
            let tail = tail.trim();
            if tail.is_empty() {
                // Bare `/login` without a provider is recoverable: let the UI
                // guide the user rather than treating it as Unknown.
                Some(Command::Login {
                    provider: String::new(),
                    key: String::new(),
                })
            } else if let Some((prov, key)) = tail.split_once(' ') {
                Some(Command::Login {
                    provider: prov.trim().to_string(),
                    key: key.trim().to_string(),
                })
            } else {
                Some(Command::Login {
                    provider: tail.to_string(),
                    key: String::new(),
                })
            }
        }
        "logout" => Some(Command::Logout(tail.to_string())),
        "quit" | "q" | "exit" => Some(Command::Quit),
        other => Some(Command::Unknown(other.to_string())),
    }
}

/// Split an `/export` tail into `(format, path)`.
///
/// Bare means HTML with no path (the caller picks the default file);
/// otherwise the first word names a format (`html`/`json`/`jsonl`) with the
/// rest as the path, or the path alone with the format inferred from its
/// extension (HTML unless it ends `.json`/`.jsonl`). Shared by both front
/// ends so `/export` spells the same thing in the terminal and in the window.
pub fn split_export_arg(tail: &str) -> (String, String) {
    let tail = tail.trim();
    if tail.is_empty() {
        return ("html".to_string(), String::new());
    }
    let mut parts = tail.split_whitespace();
    let first = parts.next().unwrap_or("");
    let rest = parts.next();
    if first.eq_ignore_ascii_case("html")
        || first.eq_ignore_ascii_case("json")
        || first.eq_ignore_ascii_case("jsonl")
    {
        (first.to_ascii_lowercase(), rest.unwrap_or("").to_string())
    } else {
        let inferred = if tail.ends_with(".json") {
            "json"
        } else if tail.ends_with(".jsonl") {
            "jsonl"
        } else {
            "html"
        };
        (inferred.to_string(), tail.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_head_in_command_heads_is_parsed() {
        for head in COMMAND_HEADS {
            assert!(
                parse_command(&format!("/{head}")).is_some(),
                "`/{head}` is listed in COMMAND_HEADS but parse_command returned None"
            );
        }
    }

    /// **A command that parses but is never offered is a command the user is not told exists.**
    ///
    /// The two checks above cover `COMMAND_HEADS` ↔ `parse_command`. Neither of them mentions
    /// [`SLASH_ITEMS`], and that is where the gap was: `SLASH_ITEMS` is what `/help` prints and
    /// what the `/` autocomplete suggests, so a head missing from it is a command that works when
    /// typed and is invisible everywhere the product volunteers it. `theme` and `templates` were
    /// both in that state — parseable, in `COMMAND_HEADS`, and offered by no surface.
    ///
    /// The match allows a trailing space because that is how an argument-taking command is
    /// spelled (`"/model "`). Anything else would make this test fail on correct data.
    #[test]
    fn every_offered_command_is_reachable_from_slash_items() {
        for head in COMMAND_HEADS {
            let needle = format!("/{head}");
            let offered = SLASH_ITEMS
                .iter()
                .any(|item| item.value == needle || item.value == format!("{needle} "));
            assert!(
                offered,
                "`{needle}` parses and is in COMMAND_HEADS but is absent from SLASH_ITEMS, so \
                 /help never lists it and the `/` autocomplete never suggests it"
            );
        }
    }

    /// Reverse drift: the parser must not accept a command the list does not drive.
    ///
    /// This is the direction a hand-written list cannot catch, and the one that matters: a
    /// command added to the `match` and forgotten in the sweep would be executed by no test,
    /// which is precisely how the M168 defect survived sixteen commands' worth of coverage
    /// census. Reading the parser's own arms means the check cannot lag behind the code.
    ///
    /// This is a source-shaped assertion and honestly labelled as one - it reads the quoted
    /// tokens of `parse_command`'s body out of this file. It is the price of `Command` not
    /// being `Debug` and therefore not comparable. The alternative is a second hand-written
    /// list, which is the bug this milestone removes.
    #[test]
    fn the_parser_does_not_know_a_command_the_list_does_not() {
        // Line-based, not byte-based. This repository is **LF** on disk today
        // (`.gitattributes`: `* text=auto eol=lf`, and `scripts/check-line-endings.py` is a
        // gate) — it was CRLF when this check was written, and a byte pattern for `"\n}\n"`
        // still finds nothing on a checkout with CRLF, failing in a way that looks like a
        // parser bug. `lines()` strips the terminator and is immune to either ending, which is
        // the reason it is used rather than a claim about today's bytes.
        //
        // The anchor was repointed with the function. It read `picker.rs` when the parser
        // lived in the terminal; it reads itself now, and `include_str!` resolves relative to
        // the file holding the call, which is the file the parser moved into. A wrong path
        // here fails loudly (`.expect` on both anchors), never silently green.
        let lines: Vec<&str> = include_str!("command.rs").lines().collect();
        let start = lines
            .iter()
            .position(|l| l.starts_with("pub fn parse_command("))
            .expect("parse_command is defined in this file");
        let end = lines[start..]
            .iter()
            .position(|l| l.trim_end() == "}")
            .map(|i| start + i)
            .expect("parse_command has a closing brace on a line of its own");

        let mut found: Vec<&str> = Vec::new();
        for line in &lines[start..end] {
            let line = line.trim_start();
            if !line.starts_with('"') {
                continue;
            }
            for piece in line.split('"').skip(1).step_by(2) {
                // **The character class is the rule that decides what this check can see, so it
                // names every character the parser actually uses in a head.** It used to be
                // `lowercase`, `_` and `:`, and the two spellings that did not fit —
                // `scoped-models` and `?` — were therefore invisible to it: a check that cannot
                // see a spelling cannot report it, and `ALIAS_HEADS` was allowed to leave them
                // out. `-` and `?` are here for exactly those two arms, and nothing else widens
                // the class: the "line starts with a quote" guard above is what keeps comments
                // and format strings out, and it stays the real filter.
                let is_head = !piece.is_empty()
                    && piece.chars().all(|c| {
                        c.is_ascii_lowercase() || c == '_' || c == ':' || c == '-' || c == '?'
                    });
                if is_head && found.last() != Some(&piece) {
                    found.push(piece);
                }
            }
        }

        let unlisted: Vec<&&str> = found
            .iter()
            .filter(|h| !COMMAND_HEADS.contains(*h) && !ALIAS_HEADS.contains(*h))
            .collect();
        assert!(
            unlisted.is_empty(),
            "the parser accepts {unlisted:?} but no test drives them; add each to COMMAND_HEADS \
             (or to ALIAS_HEADS if it is only another spelling of a listed command) so the \
             all-commands sweep reaches it"
        );
    }

    #[test]
    fn commands_parse_with_optional_args_and_case() {
        assert_eq!(parse_command("/models"), Some(Command::Models));
        assert_eq!(
            parse_command("/model"),
            Some(Command::Models),
            "bare /model opens the picker"
        );
        assert_eq!(parse_command(" /MODELS "), Some(Command::Models));
        assert_eq!(
            parse_command("/model deepseek-chat"),
            Some(Command::Model("deepseek-chat".into()))
        );
        assert_eq!(
            parse_command("/MODEL kimi-k2"),
            Some(Command::Model("kimi-k2".into()))
        );
        assert_eq!(parse_command("/help"), Some(Command::Help));
        assert_eq!(parse_command("/?"), Some(Command::Help));
        assert_eq!(parse_command("/clear"), Some(Command::Clear));
        assert_eq!(parse_command("/skills"), Some(Command::Skills));
        assert_eq!(parse_command("/quit"), Some(Command::Quit));
        assert_eq!(parse_command("/q"), Some(Command::Quit));
        // pi parity builtins (G1): every builtin must parse, not fall to Unknown
        assert_eq!(parse_command("/new"), Some(Command::New));
        assert_eq!(
            parse_command("/resume /tmp/s.json"),
            Some(Command::Resume("/tmp/s.json".into()))
        );
        assert_eq!(parse_command("/session"), Some(Command::Session));
        assert_eq!(parse_command("/tree"), Some(Command::Tree));
        assert_eq!(parse_command("/clone"), Some(Command::Clone));
        assert_eq!(
            parse_command("/trust yes"),
            Some(Command::Trust(Some("yes".into())))
        );
        assert_eq!(parse_command("/trust"), Some(Command::Trust(None)));
        assert_eq!(parse_command("/reload"), Some(Command::Reload));
        assert_eq!(
            parse_command("/export ./out.html"),
            Some(Command::Export(Some("./out.html".into())))
        );
        assert_eq!(
            parse_command("/import ./in.jsonl"),
            Some(Command::Import("./in.jsonl".into()))
        );
        // D-046/S-105: the window's workspace door. The tail is a directory path,
        // carried whole like `/resume` and `/import` — splitting a path is how
        // `src/my dir` becomes two arguments and a switch to nowhere.
        assert_eq!(
            parse_command("/open ../other"),
            Some(Command::Open("../other".into()))
        );
        assert_eq!(parse_command("/share"), Some(Command::Share));
        assert_eq!(parse_command("/copy"), Some(Command::Copy));
        assert_eq!(
            parse_command("/name my-session"),
            Some(Command::Name(Some("my-session".into())))
        );
        assert_eq!(parse_command("/changelog"), Some(Command::Changelog));
        assert_eq!(parse_command("/hotkeys"), Some(Command::Hotkeys));
        assert_eq!(parse_command("/settings"), Some(Command::Settings));
        assert_eq!(parse_command("/scoped-models"), Some(Command::ScopedModels));
        assert_eq!(
            parse_command("/bogus"),
            Some(Command::Unknown("bogus".into())),
            "unknown slash words error out — never sent to the model"
        );
        assert_eq!(parse_command("fix the bug"), None);
    }

    /// `/export` tails split the same way for both front ends: bare means HTML
    /// with no path, a leading format word wins, otherwise the extension
    /// decides (HTML unless `.json`/`.jsonl`).
    #[test]
    fn split_export_arg_spells_one_grammar() {
        assert_eq!(split_export_arg(""), ("html".into(), "".into()));
        assert_eq!(split_export_arg("   "), ("html".into(), "".into()));
        assert_eq!(
            split_export_arg("json out.json"),
            ("json".into(), "out.json".into())
        );
        assert_eq!(
            split_export_arg("HTML OUT.HTML"),
            ("html".into(), "OUT.HTML".into())
        );
        assert_eq!(
            split_export_arg("jsonl out.jsonl"),
            ("jsonl".into(), "out.jsonl".into())
        );
        assert_eq!(
            split_export_arg("out.json"),
            ("json".into(), "out.json".into())
        );
        assert_eq!(
            split_export_arg("notes.txt"),
            ("html".into(), "notes.txt".into())
        );
        // A format word with no path that follows: empty path, not the word.
        assert_eq!(split_export_arg("json"), ("json".into(), "".into()));
    }

    /// `/skill <name> on|off` parses, and a bare `/skill` does not guess.
    ///
    /// M196: a setting with no writer is the repo's own documented defect class
    /// (`EntryKind::Label` had one for most of a milestone). The parse test is
    /// here rather than at the handler because the failure mode being avoided is
    /// SILENT: a `/skill commit` that parsed as "disable" would switch a skill
    /// off because the user forgot a word.
    #[test]
    fn a_skill_toggle_takes_a_name_and_an_explicit_on_or_off() {
        assert_eq!(
            parse_command("/skill commit off"),
            Some(Command::Skill("commit off".to_string())),
            "the whole tail is carried, so the handler can split it"
        );
        assert_eq!(
            parse_command("/skill commit on"),
            Some(Command::Skill("commit on".to_string()))
        );
        // A bare `/skill` is NOT a toggle. It carries an empty tail, and the
        // handler prints usage -- it does not default to "off".
        match parse_command("/skill") {
            Some(Command::Skill(tail)) => assert!(
                tail.trim().is_empty(),
                "a bare /skill carries nothing, so nothing can be assumed: {tail:?}"
            ),
            other => panic!("expected a Skill command, got {other:?}"),
        }
        // `/skills` (plural, the listing) is a DIFFERENT command and stays one.
        assert_eq!(
            parse_command("/skills"),
            Some(Command::Skills),
            "the listing is not the toggle"
        );
    }

    /// **`/models <id>` sets the model, exactly as `/model <id>` does.**
    ///
    /// This is the user-facing half of a parser fact, and the sentence the user got was false.
    /// `models` is an alternative spelling of `model` ([`ALIAS_HEADS`]) and is offered by
    /// [`SLASH_ITEMS`] as the picker, so `/models gpt` is the obvious next thing to type after
    /// seeing it. The arm read `"models" | "model" if tail.is_empty()`, which meant the plural
    /// spelling took no argument: `/models gpt` reached no arm and came back as
    /// [`Command::Unknown`], and the UI says *unknown command '/models'* — a command this file
    /// lists, this product advertises, and this parse refuses to recognise. A reader who typed
    /// something reasonable and was told it does not exist has learned something untrue.
    ///
    /// So the alias joins the arm it aliases. Bare `/models` still opens the picker — that is
    /// the one thing the plural spelling is *for* — and every property the parser already
    /// promises for `/model` (case folding, surrounding whitespace, a trimmed tail) now holds for
    /// `/models` too, because there is one grammar and not two.
    #[test]
    fn the_plural_model_spelling_takes_the_argument_the_singular_one_does() {
        // What a reader types, and what they get back.
        assert_eq!(
            parse_command("/models gpt"),
            Some(Command::Model("gpt".to_string())),
            "`/models gpt` must set the model id, not be refused as a command nobody has"
        );
        assert_eq!(
            parse_command("/model gpt"),
            Some(Command::Model("gpt".to_string())),
            "and it must be indistinguishable from the singular spelling"
        );
        assert_eq!(
            parse_command("/models gpt-5"),
            parse_command("/model gpt-5"),
            "the two spellings are one grammar: any id must give the same command"
        );

        // The part that is still the plural spelling's job.
        assert_eq!(
            parse_command("/models"),
            Some(Command::Models),
            "a bare `/models` opens the picker — that is what the plural is for"
        );
        assert_eq!(
            parse_command("/models "),
            Some(Command::Models),
            "and a trailing space is an empty tail, not an argument"
        );

        // The promises the parser already makes for every other head, now held for this one.
        assert_eq!(
            parse_command(" /MODELS gpt "),
            Some(Command::Model("gpt".to_string())),
            "case folding and surrounding whitespace are the parser's, not the arm's"
        );
        assert_eq!(
            parse_command("/MODEL gpt"),
            parse_command("/models gpt"),
            "and upper-case must not be the one spelling that behaves differently"
        );

        // The failure this test exists for, stated as a failure rather than left implicit.
        assert!(
            !matches!(parse_command("/models gpt"), Some(Command::Unknown(_))),
            "`/models gpt` came back as Unknown, which the UI reports as an unknown command — \
             a false sentence about a command the product lists and offers"
        );
    }
}
