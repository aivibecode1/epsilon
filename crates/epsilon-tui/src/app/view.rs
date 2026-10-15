//! What the frame shows, and where the user is inside it.
//!
//! ## Why this file exists
//!
//! The second of `app.rs`'s two `impl App` blocks: 22 methods that read the conversation's
//! position and shape rather than change it. `run_state`, `metrics`, `cost_summary` and
//! `hints` are what the header and footer report; the rest decide where the user is - which
//! card is expanded, which overlay owns the keyboard, how tall the transcript window is,
//! whether it is empty, where `/search` is and which match it is on, and how much of a
//! streaming tool's output still fits on screen.
//!
//! They are one file because the draw path, the panel renderer and `/search` all read them
//! together, and because a change to any of them is a change to *what the frame can show*.
//! They were interleaved with the methods that mutate state, which change for entirely
//! unrelated reasons: a provider reports usage, a key is pressed, a tool streams a line.
//!
//! ## The rule for this move
//!
//! The whole block was sliced by line range and is byte-identical to `app.rs` at the parent
//! commit. The only edits were the `pub(super)` markers the move required, added where the
//! compiler asked for them - a file's contents do not get to decide their own visibility.
use super::*;

/// Adapters from live application state to the plain values the presentation
/// layer draws with. Keeping them here — rather than letting widgets read the
/// app — is what lets every screen be rendered in a test without an agent.
impl App {
    /// Whether the header should say this process is offline (M233).
    ///
    /// The CLI's decision **or** a selected faux model: a caller that sets `model_id` directly (the
    /// TUI's own tests, `/model faux-1`) has no CLI to tell it, and the old rule — `model_id` starts
    /// with `faux` — is what made the header name DeepSeek over a faux turn. Both facts, either one
    /// is enough.
    pub(crate) fn is_offline(&self) -> bool {
        self.offline || self.model_id.starts_with("faux")
    }
    /// The look this session was started with.
    ///
    /// Public so a caller outside the draw path (a future integration test, a
    /// screenshot harness) can render with the same palette the binary uses.
    #[allow(dead_code, reason = "the draw path reads the field directly")]
    pub fn theme(&self) -> &crate::ui::theme::Theme {
        &self.theme
    }

    // ---- M126: per-card expansion -------------------------------------------

    /// Whether one card renders its whole body.
    ///
    /// The user's own decision if they made one, the global toggle otherwise. The
    /// global toggle has to stay authoritative for cards nobody has touched, or
    /// "expand all" would only ever work on the cards that were already open.
    ///
    /// **This defers to [`crate::render_cache::RenderState::is_open`] rather than
    /// restating it.** The two answer the same question about the same card — one
    /// decides what is *drawn*, the other what a keypress *records* — and the
    /// whole per-card convention lives in their agreement: press `ctrl+x` twice
    /// and the card the first press opened must still be open after the second,
    /// which is only coherent while both halves read one rule. A second copy that
    /// agreed today is exactly the copy that would stop agreeing on the day
    /// somebody adds a third source of truth to one of them.
    pub(super) fn card_is_open(&self, id: u64, has_body: bool) -> bool {
        crate::render_cache::RenderState {
            expand_all: self.prefs.expand_outputs,
            // Unread by `is_open`: this decides card openness, not row
            // visibility.
            hide_thinking: false,
            // Likewise unread: the filter selects rows, and this decides one
            // card's body.
            filter: crate::state::TranscriptFilter::default(),
            expanded: &self.card_open,
            // The key text is the renderer's business, not the rule's: it is
            // `None` here because nothing in this decision reads it, and a
            // resolved chord would only be a second thing that could disagree.
            more_key: None,
        }
        .is_open(id, has_body)
    }

    /// The row `Ctrl+X` acts on when it is not the global toggle's turn.
    ///
    /// The **newest** expandable row: the reader is at the bottom of a live
    /// transcript, so the most recent expansion is the one they just watched
    /// happen. `None` when no expandable row exists, and then the key is purely
    /// the global toggle it always was.
    ///
    /// Two shapes of expandable, one rule: a tool card with a body, and an
    /// expandable **event** row (the compaction digest —
    /// [`crate::state::TranscriptLine::is_expandable_event_row`]). Both nest
    /// their body under the continuation gutter and both name the same chord, so
    /// the chord picks the newest of either rather than owning a second rule.
    pub(super) fn selected_card_id(&self) -> Option<u64> {
        self.ui
            .lines()
            .iter()
            .rev()
            .find(|l| l.is_expandable_event_row() || l.card.as_ref().is_some_and(|c| c.has_body()))
            .map(|l| l.id)
    }

    /// Flip one card's own expansion state, remembering the decision.
    ///
    /// Marks the card's rows stale so exactly that card is rebuilt: the opened
    /// body is measured, wrapped and laid out against the *current* width, which a
    /// cached row from a narrower frame could not be.
    pub(super) fn set_card_open(&mut self, id: u64, open: bool) {
        match self
            .card_open
            .iter_mut()
            .find(|(card_id, _)| *card_id == id)
        {
            Some(entry) => entry.1 = open,
            None => self.card_open.push((id, open)),
        }
        if !self.card_restale.contains(&id) {
            self.card_restale.insert(id);
        }
    }

    /// The key text a capped card names in its `… N more lines (… to open)` row.
    ///
    /// Resolved from the live binding table here rather than inside the card
    /// renderer: a widget that read the keybindings manager would be a widget no
    /// test could aim, and resolving it is the whole point — a rebound
    /// `app.tools.expand` moves this text with it.
    fn expand_key(&self) -> Option<String> {
        self.keybindings.key_text("app.tools.expand")
    }

    /// Name the cards this event moved, so the render cache rebuilds them.
    ///
    /// Two halves, one walk. A card settles *inside* its line — the result marks the
    /// call's own row finished, so neither the row count nor its id changes and the
    /// cache would happily keep serving "pending" for a call that has returned. And a
    /// card's streamed output lands on its body while `ended_at_ms` is still `None`, so
    /// the finished-only walk never named it and the cache served the rows built when
    /// the card opened: the M152 fix was correct in state and **never on screen**, which
    /// is `AGENTS.md`'s own failure mode in its own words — a test that passes is not a
    /// claim that is true, and for anything a user sees the real draw path is what has to
    /// be asserted.
    ///
    /// A card's id is stable across its own status change — that is what lets the
    /// reader's open/closed choice survive the result landing — so the cache cannot
    /// notice the change from the id alone and has to be told. Naming the moved cards is
    /// deliberate: the rebuild happens once per card, and "which card changed" is a
    /// question the reducer is asked to answer here rather than in the cache.
    ///
    /// The walk is unchanged in cost. It already ran for this event class, and the
    /// membership test is a hash lookup, so one call is O(lines). It used to be
    /// O(lines × cards) — the set was a `Vec` scanned with `contains` — and the event loop
    /// can drain up to [`MAX_EVENT_DRAIN`] events before drawing a single frame, which
    /// made a busy stream pay that walk 128 times per frame for a list that almost
    /// always had nothing to add.
    pub(crate) fn restale_cards(&mut self, ev: &AgentEvent) {
        // The card this event names, if it names one that is still running: `Start`
        // moves it into `Running` and `Update` grows its body, and neither has set
        // `ended_at_ms`. `End` settles it and is covered by the clause above.
        let streaming = match ev {
            AgentEvent::ToolExecutionStart { tool_call_id, .. }
            | AgentEvent::ToolExecutionUpdate { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        };
        for line in self.ui.lines() {
            let Some(card) = line.card.as_ref() else {
                continue;
            };
            let moved = card.ended_at_ms.is_some()
                || streaming.is_some_and(|id| {
                    // `Some("")` is the abort sentinel: no id to match, so every
                    // in-flight card is named — the abort settles all of them.
                    id.is_empty() || (!card.tool_call_id.is_empty() && card.tool_call_id == id)
                });
            if moved && !self.card_restale.contains(&line.id) {
                self.card_restale.insert(line.id);
            }
        }
    }

    /// Name the finished cards, so the render cache rebuilds them.
    /// Run state for the header.
    ///
    /// The phase word is derived here rather than stored, so it cannot disagree
    /// with the flags it is derived from.
    ///
    /// **A retry in flight is the phase (P-189).** The countdown is recomputed per
    /// frame from the deadline the notice carried, which is the only way the row
    /// can show seconds falling rather than the number the first frame happened
    /// to read. It displaces the tool's own status line for as long as it lasts,
    /// because "waiting 4s before try 2" is what the run is *doing*, and it ends
    /// by itself when the wait expires ([`crate::state::UiState::retry_notice`]).
    /// The `esc` it names is the same `app.interrupt` the footer already
    /// advertises while a run is live, so the hint is not a key that does
    /// nothing.
    pub(super) fn run_state(&self) -> crate::ui::header::RunState {
        let now = crate::state::now_ms();
        let retrying = self.ui.retry_notice(now);
        let phase = if retrying.is_none() {
            self.ui.status().to_string()
        } else {
            String::new()
        };
        match self.run_started_at {
            Some(started) => crate::ui::header::RunState::Running {
                frame: self.spinner_phase,
                phase,
                elapsed_secs: started.elapsed().as_secs(),
                retry: retrying,
            },
            None if self.ui.running() => crate::ui::header::RunState::Running {
                frame: self.spinner_phase,
                phase,
                elapsed_secs: 0,
                retry: retrying,
            },
            None => crate::ui::header::RunState::Idle,
        }
    }

    /// The measurements the header reports.
    ///
    /// Cost is computed here, from catalog prices, and is `None` for a model the
    /// catalog cannot price — the header then omits it rather than printing a
    /// confident `$0`. Integer micro-USD throughout (ADR-010), so no float drift
    /// accumulates over a long session.
    ///
    /// Three of the four components are charged, not two. `usage.input` excludes cached
    /// prompt tokens (the providers' own split), so uncached input, output, cache reads
    /// and cache writes are disjoint quantities and each is priced at its own rate. The
    /// terms are summed before the single division, which keeps micro-USD rounding to one
    /// truncation instead of four.
    ///
    /// When the registry publishes no cache price and the session moved cache tokens, the
    /// total is a **lower bound**, and `Metrics::cost_is_floor` says so — the header prints
    /// `$0.42+` rather than a precise-looking number that is wrong (G18).
    ///
    /// # The context number, and why it is not the last prompt
    ///
    /// `context_used` is the **session context**, mirrored from
    /// `Agent::estimated_context_tokens` — the same figure the compaction trigger reads, so
    /// the gauge and the loop cannot disagree about how full the context is. It is not
    /// `latest_prompt_tokens`: that is the size of the last *request*, which on turn 1 is
    /// the whole conversation and thereafter is only its prefix. Printing it as a share of
    /// the window was the fabrication this field replaced — it is always smaller than the
    /// context, so it always flattered.
    ///
    /// The mirror is taken once per frame by the event loop
    /// (`App::mirror_context_size`), not read here, because the draw path holds no agent:
    /// `App` is the frame's whole input, and a widget that reached for the loop's state
    /// would be a widget no test could aim.
    ///
    /// `prompt_tokens_last` stays the provider's number and stays the cache rate's
    /// denominator: a hit rate is a claim about one call, and both halves of it have to
    /// come from that call.
    pub(super) fn metrics(&self) -> crate::ui::metrics::Metrics {
        let (tokens_in, tokens_out) = self.ui.usage();
        let (cache_read_session, cache_write_session) = self.ui.cache_usage();
        // The rate's numerator comes from the SAME call as its denominator, not from the
        // session totals above (G16).
        let cache_read_last = self.ui.latest_cache_read();
        let latest_prompt = self.ui.latest_prompt_tokens();
        // The context *window* is a property of the model selected now — it is a capacity,
        // not a bill — so this one lookup stays keyed on `self.model_id`. The cost below
        // deliberately does not.
        let info = epsilon_llm::catalog::lookup_model(&self.model_id);
        // M141: price each model's tokens at that model's own rates. Pricing the summed
        // totals at `self.model_id` — the model selected *now* — repriced every earlier
        // turn the moment you switched, and `/resume` did the same to replayed history
        // that was billed at other prices. The registry lookup is per model id rather
        // than one lookup for the session, so this is also what makes a resumed
        // multi-model session come back with the bill it actually incurred.
        let cost = if self.ui.usage_by_model().is_empty() {
            // No provider response yet: nothing has been billed, so there is nothing to
            // price and no model whose absence from the registry matters.
            None
        } else {
            Some(crate::ui::cost::breakdown_per_model(
                self.ui.usage_by_model(),
                |model_id| {
                    epsilon_llm::catalog::lookup_model(model_id).map(|info| {
                        crate::ui::cost::TokenPrices {
                            input: info.input_cost_micro_per_mtok,
                            output: info.output_cost_micro_per_mtok,
                            cache_read: info.cache_read_cost_micro_per_mtok,
                            cache_write: info.cache_write_cost_micro_per_mtok,
                        }
                    })
                },
            ))
        };
        let cost_micro_usd = cost.as_ref().map(crate::ui::cost::CostBreakdown::total);
        let cost_is_floor = cost.as_ref().is_some_and(|c| c.is_floor());
        // A frame that has not mirrored yet — the first one, or a bench frame —
        // still reports the provider's own count when there is one. This is a
        // fallback for a missing estimate, not a maximum: on every frame the loop
        // has drawn, the estimate is the measured prompt plus whatever was
        // appended since, so it is the larger of the two and this branch is dead.
        // An estimate is never *smaller* than the prompt it was anchored to, so
        // seeding from `latest_prompt` cannot overstate anything.
        let context_now = if self.ui.context_now > 0 {
            self.ui.context_now
        } else {
            latest_prompt
        };
        crate::ui::metrics::Metrics {
            // M233: the label names the provider this process is actually calling. Keying both
            // off `model_id` made `--provider faux` beside a saved `deepseek/...` model render
            // `DeepSeek V3.1` over a faux turn -- a screen naming a provider the harness is
            // not calling, which is the class of lie this repository refuses.
            model_label: if self.is_offline() {
                epsilon_llm::catalog::display_label("faux-1").to_string()
            } else {
                epsilon_llm::catalog::display_label(&self.model_id).to_string()
            },
            // **The level `/help` says is in the status bar, so it is (M312).** `self.thinking`
            // is the app's mirror of the agent's level, kept exact by `set_thinking`, which is
            // what `app::tests` already pins against `agent.thinking_level()`.
            thinking: Some(self.thinking.as_str()),
            offline: self.is_offline(),
            context_used: (context_now > 0).then_some(context_now),
            context_window: info.as_ref().map(|i| i.context_window).filter(|w| *w > 0),
            context_estimated: self.ui.context_estimated,
            tokens_in,
            tokens_out,
            cache_read_session,
            cache_write_session,
            cache_read_last,
            prompt_tokens_last: (latest_prompt > 0).then_some(latest_prompt),
            // K1: straight from the session, never inferred from a zero count.
            cache_reported: self.ui.cache_reported(),
            cost_micro_usd,
            cost_is_floor,
            compacted: self.ui.is_compacted(),
        }
    }

    /// One line naming what the session has cost and which part of it is the cache's doing.
    ///
    /// The header can only show a total, and a total is not actionable: `$0.31` reads the
    /// same whether the prompt cache saved 90% of your spend or none of it. The cache is
    /// the main lever a user has on cost, so `/session` shows the split that tells them
    /// whether it is working.
    ///
    /// **It says what it does not know.** Three cases, each with its own wording, because
    /// collapsing them would be the exact failure this repo keeps fixing:
    ///
    /// - no usage yet → "nothing billed yet", never `$0.00`, which is a precise-looking
    ///   number about nothing;
    /// - a model the price registry lacks → the total is a **lower bound** and is marked
    ///   with `+`, the same marker the header uses, so the two never disagree;
    /// - a genuinely free model (ollama, a `:free` row) → `$0.00` is the true answer, and
    ///   `is_floor` is what distinguishes it, so this line trusts that flag rather than
    ///   second-guessing the registry.
    pub(super) fn cost_summary(&self) -> String {
        use crate::ui::cost::{TokenPrices, breakdown_per_model};
        if self.ui.usage_by_model().is_empty() {
            return "cost: nothing billed yet".to_string();
        }
        let b = breakdown_per_model(self.ui.usage_by_model(), |model_id| {
            epsilon_llm::catalog::lookup_model(model_id).map(|info| TokenPrices {
                input: info.input_cost_micro_per_mtok,
                output: info.output_cost_micro_per_mtok,
                cache_read: info.cache_read_cost_micro_per_mtok,
                cache_write: info.cache_write_cost_micro_per_mtok,
            })
        });
        let total = crate::ui::format::format_cost(b.total());
        let floor = if b.is_floor() { "+" } else { "" };
        format!(
            "cost: {total}{floor} · in {} · out {} · cache read {} · cache write {}",
            crate::ui::format::format_cost(b.input()),
            crate::ui::format::format_cost(b.output()),
            crate::ui::format::format_cost(b.cache_read()),
            crate::ui::format::format_cost(b.cache_write()),
        )
    }

    /// Mirror the agent's context estimate onto the frame's input.
    ///
    /// The draw path takes `&mut App` and nothing else, which is what lets every
    /// screen be assembled in a test with no agent. The estimate therefore has
    /// to arrive *through* the app, and this is the one place it does — called
    /// by the event loop immediately before it draws, so the number on the row
    /// describes the same history the transcript beneath it is about to show.
    ///
    /// Cheap enough for that position: the estimate is a provider-anchored
    /// figure plus a heuristic over what has been appended since, and it is
    /// taken once per frame rather than once per header fragment.
    pub(crate) fn mirror_context_size(&mut self, agent: &Agent) {
        self.ui.context_now = agent.estimated_context_tokens() as u64;
        // Provenance travels with the number. A reader who cannot tell a measurement
        // from a guess cannot calibrate how much to trust the figure, and the moments
        // where it is a guess — turn one, after a compaction, after `/resume` — are
        // exactly the moments a user is watching it.
        self.ui.context_estimated = agent.context_is_estimated();
    }

    /// Footer hints for the current state.
    pub(super) fn hints(&mut self) -> crate::ui::footer::Hints {
        use crate::ui::footer::{Hints, View};
        // D11: the frozen state comes from the same measurement the transcript
        // draws from, so the badge cannot describe a viewport the reader is not
        // looking at. `scroll_from_bottom > 0` *is* the state — it is what
        // `apply_with_scroll` shifts when rows arrive, which is what keeps the
        // window frozen on the same content — and the counts come from the
        // geometry measured above.
        let view = View {
            frozen: self.scroll_from_bottom > 0,
            rows_below: self.geometry.rows_below(),
            total_rows: self.geometry.total,
        };
        let mut hints = if self.guidance_mode {
            // **Guidance mode gets its own hints (A42).**
            //
            // This branch used to fall through to `pending_ask.is_some()`, which printed
            // `y allow once · a always · …` — while `y` and `a` type letters into the
            // composer. The footer was not merely incomplete; it described the *wrong
            // keyboard*, on the screen where a reader is deciding whether work runs.
            Hints {
                mode: Some("guidance".into()),
                keys: vec![
                    ("enter".into(), "deny with this".into()),
                    ("esc".into(), "back to the ask".into()),
                ],
                ..Hints::default()
            }
        } else if self.pending_ask.is_some() {
            // Deny-only when the request carries display-steering characters: the approve
            // keys are refused in `handle_approval_key`, and a footer that still printed
            // them would be a key the window does not act on, advertised as though it would.
            let honest = self
                .pending_ask
                .as_ref()
                .is_some_and(crate::app::ask_display_is_honest);
            if honest {
                Hints::approval()
            } else {
                Hints::approval_deny_only()
            }
        } else if !self.viewer_tabs.is_empty() {
            let viewer = self.viewer_tabs.get(self.viewer_tab);
            let editing = viewer.is_some_and(|v| v.editing);
            let truncated = viewer.is_some_and(|v| v.truncated);
            Hints {
                mode: Some(if editing {
                    "edit".into()
                } else {
                    "view".into()
                }),
                keys: if editing {
                    let mut keys = vec![
                        ("type".into(), "edit".into()),
                        ("enter".into(), "newline".into()),
                    ];
                    if let Some(save) = self.keybindings.key_text("app.viewer.save") {
                        keys.push((save, "save".into()));
                    }
                    keys.push(("esc".into(), "cancel".into()));
                    keys
                } else if truncated {
                    // Invariant 7: `e` does not edit a capped file
                    // (`start_edit` refuses), so the footer must not offer
                    // it as edit.
                    vec![
                        ("arrows/pgup/pgdn".into(), "scroll".into()),
                        ("e".into(), "edit unavailable".into()),
                        ("esc".into(), "close".into()),
                    ]
                } else {
                    vec![
                        ("arrows/pgup/pgdn".into(), "scroll".into()),
                        ("e".into(), "edit".into()),
                        ("esc".into(), "close".into()),
                    ]
                },
                ..Hints::default()
            }
        } else if self.tree_open {
            // The tree has no filter box, so the generic overlay hints' "type
            // filter" would advertise a key the tree does not act on. Its own
            // row names what the tree honours: arrows, Enter, Esc, and F6.
            let arrows = crate::ui::overlay::up_down(
                crate::ui::theme::Glyphs::detect().up,
                crate::ui::theme::Glyphs::detect().down,
            );
            let mut keys = vec![
                (arrows, "move".into()),
                ("enter".into(), "view".into()),
                ("@".into(), "refer".into()),
                ("esc".into(), "close".into()),
            ];
            if let Some(toggle) = self.keybindings.key_text("app.tree.toggle") {
                keys.push((toggle, "tree".into()));
            }
            Hints {
                mode: Some("tree".into()),
                keys,
                ..Hints::default()
            }
        } else if self.overlay_open() {
            // Overlays own the keyboard; the footer points at their own keys
            // rather than advertising send/stop, which would not work. The
            // arrow pair is a glyph rather than a literal because it is
            // repertoire — and this builder has no `App` to ask, so it asks
            // `Glyphs::detect()` once, which is the same process decision the
            // app made at startup (M219).
            let arrows = crate::ui::overlay::up_down(
                crate::ui::theme::Glyphs::detect().up,
                crate::ui::theme::Glyphs::detect().down,
            );
            Hints {
                mode: Some("list".into()),
                keys: vec![
                    ("type".into(), "filter".into()),
                    (arrows, "move".into()),
                    ("enter".into(), "choose".into()),
                    ("esc".into(), "close".into()),
                ],
                ..Hints::default()
            }
        } else if self.ui.running() {
            Hints::running_at(&self.keybindings, self.steer_queue_len, view)
        } else {
            // P-022: a leading `!` runs a shell command, not a prompt — the
            // mode word says so before Enter is pressed (this composer has
            // no border to recolour; the word is the signal).
            let mode = if self.input.starts_with('!') {
                "bash"
            } else {
                "insert"
            };
            Hints::idle_at_with_mode(&self.keybindings, view, mode)
        };
        // Both of these used to be recomputed on every drawn frame: two
        // `env::var_os` lookups and a path re-shorten, thirty times a second, for
        // a value that is fixed for the life of the process. `format_path` reads
        // the environment only to collapse the home prefix, and the workspace root
        // is captured in `App::new` and never reassigned — so the result cannot
        // change and is computed once.
        // The location label is picked per repertoire, not baked at construction: the
        // draw's theme decides the ellipsis mark, and `…` is one a CP437 console cannot
        // draw (round 105's battery found twelve rows carrying it).
        let location =
            &self.location_labels[usize::from(self.theme.glyphs == crate::ui::theme::ASCII)];
        hints.location = Some(location.clone());
        if let Some(branch) = self.cached_branch() {
            hints.branch = Some(branch);
        }
        // P-200: the session name, read from the field the terminal title already
        // reads (`app/session.rs:545-553`) rather than recomputed — and stripped
        // here because a session name is user-typed text reaching a screen. The
        // title strips it for the OSC sequence's sake; this row strips it because
        // it is the same untrusted string on the same screen.
        hints.session = self
            .session_name
            .as_deref()
            .map(|n| epsilon_runtime::text_safety::display_safe(n).0.into_owned())
            .filter(|n| !n.trim().is_empty());
        // The goal badge rides the same reasoning as the location: it changes when someone
        // changes the goal, not when a frame is drawn, and reading it here would fold the whole
        // session log thirty times a second. `refresh_goal` is the one writer of this field.
        hints.goal = self.goal_badge.clone();
        hints
    }

    /// Re-read the session goal and cache the one-line badge the footer shows.
    ///
    /// Called from the two places a goal can change: a `/goal` command, and the end of a turn
    /// (where a `create_goal` / `update_goal` call lands). Reading it per frame instead would
    /// mean folding the session log on every draw for a value that changes a handful of times in
    /// a session.
    pub(crate) fn refresh_goal(&mut self, agent: &Agent) {
        self.goal_badge = agent.goal().map(|view| view.badge());
    }

    /// Whether any overlay currently owns the keyboard.
    pub(super) fn overlay_open(&self) -> bool {
        self.picker_open
            || self.palette_open
            || self.auth_prompt_open
            || self.label_prompt_open
            || self.auth_pick_open
            || self.think_pick_open
            || self.changes_open
            || self.settings_open
            || self.timeline_open
            || self.tree_open
            || !self.viewer_tabs.is_empty()
            || self.session_browser.is_some()
            || self.pending_ask.is_some()
    }

    /// Whether the transcript has nothing to show yet.
    ///
    /// Streaming text counts: a run that has started producing output must not
    /// have the welcome screen painted over it.
    pub(super) fn transcript_is_empty(&self) -> bool {
        self.ui.lines().is_empty()
            && self.ui.streaming_text().is_empty()
            && self.ui.streaming_thinking().is_empty()
    }

    // ---- D11: where the view is ---------------------------------------------

    /// The transcript's region for a frame of `area` and a draft of `draft_rows`
    /// text rows.
    ///
    /// The one place that knows the split the shell will perform, exposed so the
    /// position badge, the body and the tests all measure the same rectangle. The
    /// production draw calls it once, before the chrome; a test that renders the
    /// transcript directly calls it too rather than recomputing the layout.
    pub(super) fn transcript_window(area: Rect, draft_rows: u16) -> Rect {
        crate::ui::layout::Regions::split(area, draft_rows).transcript
    }

    /// Measure the viewport for a frame whose transcript region is not in hand.
    ///
    /// Used by the draw path (which must measure before the shell lays out) and by
    /// the tests that render [`draw_transcript`] on their own.
    pub(super) fn measure_transcript_window(
        &mut self,
        area: Rect,
        draft_rows: u16,
    ) -> TranscriptGeometry {
        self.measure_transcript(Self::transcript_window(area, draft_rows))
    }

    /// Bring the render cache in line with the transcript and measure the
    /// viewport the body region will show.
    ///
    /// `area` is the **transcript's** rectangle. Runs *before* the chrome, because
    /// the footer reports the frozen view's position and a footer painted before
    /// the transcript was measured could only ever describe the previous frame.
    ///
    /// Calling it twice in one frame is free and harmless: [`crate::render_cache::RenderCache::reconcile`]
    /// is a no-op once the cache matches the transcript. The draw path calls it
    /// here, once, and the body then renders the geometry this returned.
    pub(super) fn measure_transcript(&mut self, area: Rect) -> TranscriptGeometry {
        // Body width: the frame minus the gutter the decoration adds back. Wrapping
        // to the full frame width and then prefixing two columns is how a transcript
        // row ends up two cells over the edge and wraps, which turns one line into
        // two and shifts everything below it.
        let body_width = (area.width as usize).saturating_sub(crate::ui::transcript::GUTTER);
        // Taken rather than borrowed: the stale set is consumed by the reconcile
        // that rebuilds those rows, and a card left on the list would rebuild every
        // frame after.
        let restale = std::mem::take(&mut self.card_restale);
        let more_key = self.expand_key();
        let state = crate::render_cache::RenderState {
            expand_all: self.prefs.expand_outputs,
            hide_thinking: self.ui.hide_thinking,
            filter: self.filter,
            expanded: &self.card_open,
            more_key: more_key.as_deref(),
        };
        self.render_cache
            .reconcile(self.ui.lines(), body_width, &state, &self.theme, &restale);
        // The streaming tail is not a transcript line yet, so it is counted here
        // rather than by the cache: the viewport scrolls over history *and* the
        // answer arriving under it, and a total that ignored the tail would make
        // the position badge jump by a row every time the model wrapped a line.
        let total = self.render_cache.total_rows() + self.streaming_tail_rows(body_width);
        let visible = area.height as usize;
        self.viewport_rows = visible;
        // A search target becomes resolvable only now: the cache was reconciled two lines
        // above, and the viewport height that places the match a third of the way down was
        // unknown until this line.
        self.apply_pending_jump();
        TranscriptGeometry {
            skip: crate::scroll::transcript_skip(total, visible, self.scroll_from_bottom),
            total,
            visible,
        }
    }

    /// Open a search over the rendered transcript, or explain why not.
    ///
    /// A blank query and a query that matched nothing are different answers and used to look
    /// identical, because both printed one line of prose. One is a usage error; the other is
    /// a result.
    pub(super) fn search_own_transcript(&mut self, query: &str) {
        let texts: Vec<String> = self.ui.lines().iter().map(|l| l.text.clone()).collect();
        match crate::search::TranscriptSearch::open(&texts, query, self.scroll_from_bottom) {
            None => self
                .ui
                .push_notice("usage: /search <text> \u{2014} find it in this transcript"),
            Some(found) if found.matches.is_empty() => {
                self.ui
                    .push_notice(format!("no match for '{}' in this transcript", found.query));
            }
            Some(found) => {
                // Land on the first match immediately, so opening a search is not a state the
                // user has to act in before seeing anything.
                self.search = Some(found);
                self.jump_to_current_match();
            }
        }
    }

    /// Move the view so the current match is on screen.
    ///
    /// `scroll_from_bottom` is counted from the tail, so showing row `r` means setting it to
    /// "however many rows sit below the viewport's new top". The match is placed a third of
    /// the way down rather than at the top: at the top it hides whatever the user was
    /// reading above, and centred would need the viewport height, which the cache does not
    /// know at this point in the frame.
    fn jump_to_current_match(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let Some(line) = search.current_line() else {
            return;
        };
        // Recorded, not applied. The key path still holds the *previous* frame's cache, and
        // that cache is a perfectly good line-to-row map for the wrong transcript —
        // applying here consumed the target against stale offsets, cleared the flag, and left
        // the measurement with nothing to do. That is the whole bug, and it looked like a
        // search that reported its match perfectly and then did nothing.
        self.pending_jump = Some(line);
    }

    /// Apply [`App::pending_jump`] as far as the cache allows.
    ///
    /// Called only from [`App::measure_transcript`], *after* the cache has been reconciled
    /// for this frame. Calling it from the key path is a bug, not an optimisation: the cache
    /// there is the previous frame's, and it answers authoritatively about a transcript that
    /// has since grown.
    fn apply_pending_jump(&mut self) {
        let Some(line) = self.pending_jump else {
            return;
        };
        let Some(row) = self.render_cache.row_of_line(line) else {
            return;
        };
        self.pending_jump = None;
        let total = self.render_cache.total_rows();
        let visible = self.viewport_rows.max(1);
        let skip = row
            .saturating_sub(visible / 3)
            .min(total.saturating_sub(visible));
        self.scroll_from_bottom = total.saturating_sub(visible).saturating_sub(skip);
    }

    /// Step to the next match, or the previous, and follow it.
    pub(crate) fn search_step(&mut self, forward: bool) {
        let Some(mut search) = self.search.take() else {
            return;
        };
        let moved = if forward {
            search.next_match()
        } else {
            search.previous_match()
        };
        self.search = Some(search);
        if moved.is_some() {
            self.jump_to_current_match();
        }
    }

    /// Close the search and put the view back where the user left it.
    ///
    /// Restoring rather than snapping to the tail: a search that relocates the viewport and
    /// leaves it there makes *closing* the thing you have to undo.
    pub(crate) fn search_close(&mut self) {
        if let Some(search) = self.search.take() {
            self.scroll_from_bottom = search.saved_scroll_from_bottom;
        }
    }

    /// The in-flight answer's and reasoning trace's wrapped rows, as a count.
    ///
    /// Cheap by construction: two `split('\n')` passes over the in-flight text,
    /// never a wrap of the transcript. [`draw_transcript`] builds the same rows
    /// through the same [`crate::wrap::wrap_line`] call, so the count the footer
    /// reports is the count the body draws.
    ///
    /// This used to hand `wrap_line` an owned copy of every line, which made the
    /// counting pass as expensive as the drawing one — and the drawing pass runs
    /// over the whole in-flight answer, up to thirty times a second. Borrowing the
    /// slices removes the copies; the count is now bounded by the answer's length
    /// in *characters scanned* rather than in heap allocations.
    fn streaming_tail_rows(&self, body_width: usize) -> usize {
        // P-040: hidden thinking takes no rows either — the count must agree
        // with what the draw shows, or the footer reports rows that are not
        // there.
        let thinking_tail = if self.ui.hide_thinking {
            ""
        } else {
            self.ui.streaming_thinking()
        };
        let text_tail = self.ui.streaming_text();
        // The separator `draw_transcript` inserts between the two tails. This
        // function is a hand-copy of that draw loop, so the rule has to be
        // restated here rather than shared: two tails that differ in the count
        // would make the position badge disagree with the body.
        let separator = usize::from(!thinking_tail.is_empty() && !text_tail.is_empty());
        separator
            + [
                (thinking_tail, self.theme.thinking()),
                (text_tail, self.theme.assistant()),
            ]
            .into_iter()
            .filter(|(text, _)| !text.is_empty())
            .map(|(text, style)| {
                text.split('\n')
                    .map(|line| crate::wrap::wrap_line(&[(line, style)], body_width).len())
                    .sum::<usize>()
            })
            .sum::<usize>()
    }
}
