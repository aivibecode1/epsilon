//! The event loop: what the TUI does while it is running.
//!
//! # The shape, and why
//!
//! One `select!` over five sources — terminal input, approval asks, agent
//! events, notices, and a frame timer — with a **biased** order so input is
//! served first. The bias is the whole reason a keystroke never queues behind a
//! stream of model deltas.
//!
//! Three rules make the loop feel responsive on a slow console, and each one
//! exists because its absence was measured:
//!
//! * **Drain before drawing.** A burst of keystrokes is handled to exhaustion
//!   before a frame is painted, so a burst costs one frame instead of one frame
//!   per key. Windows delivers Press *and* Release per keystroke, which doubles
//!   the naive cost.
//! * **Coalesce agent events.** A streamed answer arrives as one event per
//!   delta; drawing per delta re-wraps the whole in-flight answer each time,
//!   which is what made long answers feel frozen.
//! * **Cap the frame rate, and draw nothing when nothing changed.** An idle TUI
//!   repaints zero times per second.
//!
//! `run` in `app.rs` owns the terminal lifecycle around this; the loop owns
//! everything that happens between the first frame and the last.
//!
//! Compiled as part of the `app` module, so `super::*` reaches `App` and its
//! private methods.

use super::*;

pub(crate) async fn drive(cfg: TuiConfig, terminal: &mut ratatui::DefaultTerminal) -> i32 {
    let TuiConfig {
        agent,
        gate_runner,
        approvals,
        initial_task,
        model_id,
        live,
        startup_notices,
        trust_review_pending,
        theme_mode,
        offline,
    } = cfg;

    let mut events_rx = agent.subscribe();
    let (notices_tx, mut notices_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (fetched_tx, mut fetched_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, Result<Vec<String>, String>)>();
    let (oauth_tx, mut oauth_rx) = tokio::sync::mpsc::unbounded_channel::<OauthUiEvent>();
    let (goal_round_tx, goal_round_rx) =
        tokio::sync::mpsc::unbounded_channel::<epsilon_runtime::GoalRound>();
    let mut app = App::new(model_id, fetched_tx, notices_tx, oauth_tx);
    app.gate_runner = gate_runner;
    app.set_goal_round_channel(goal_round_tx, goal_round_rx);
    // D8: the palette was resolved by the CLI; `App::new` detected the rest.
    if let Some(mode) = theme_mode {
        app.theme.set_palette(mode);
    }
    app.live = live;
    // M233: the header reads this rather than re-deriving offline-ness from the model id, which
    // is the fact that let it name DeepSeek over a faux turn.
    app.offline = offline;
    // P-017: the terminal title tracks the session from the first frame. The
    // name is re-hydrated (a resumed named session would otherwise title
    // itself nameless until its first switch), then emitted once; switches and
    // renames refresh it from there.
    app.hydrate_session_name(&agent);
    app.refresh_title();
    // Prompt templates are discovered once here rather than per submit: expansion runs on
    // every message, and walking two directories to find files that have not changed would
    // tax the one path that has to stay fast. `/reload` refreshes them.
    //
    // **Moved below the `trust_review_pending` assignment, deliberately (A32).** This call
    // consults that flag to decide whether project templates may load, and it used to run
    // sixteen lines above where the flag is assigned — so the gate read a field that was
    // still `false` for the only call that happens at startup. An ordering bug inside a
    // security check survives review, because each line is correct on its own.
    // M16b: seed the recall ring from the persisted global history so Up works
    // across restarts.
    for entry in epsilon_core::config::load_prompt_history() {
        app.history.push_back(entry);
    }
    while app.history.len() > HISTORY_MAX {
        app.history.pop_front();
    }

    // M22 / DIRECTION-01: flush startup diagnostics as notices, and if the
    // project config changed since it was trusted, hold the first submit until
    // the user answers the review prompt (pi trust-manager semantics).
    // Audit fix: the trigger is the typed flag from TuiConfig, not a substring
    // match on banner copy (rewording text must never disable a gate).
    app.trust_review_pending = trust_review_pending;
    app.discover_prompt_templates();
    // The slash popup's skill rows refresh from the same agent (P-043),
    // beside the templates: `/reload` is the later refresh, same as for them.
    app.refresh_skills(&agent);
    for line in startup_notices {
        app.ui.push_notice(line);
    }
    if app.trust_review_pending {
        app.ui
            .push_notice("type `trust` to accept the new config, or `trust no` to quit");
    }
    // A17: a plain launch opens the session file, appends to it, and starts with an *empty*
    // in-memory history. The first thing a returning user sees is therefore a first-run screen
    // while the header reports a few percent of the context already used — both true (that
    // figure is the system prompt and the tool schemas) and together reading as a bug.
    app.note_prior_history(&agent);

    // M22 fix: an initial task (positional arg / --tui "task") must respect the
    // trust gate like any other submit. When review is pending, stash the text
    // into the editor instead of running it; after `trust` accepts, the user
    // presses Enter to send it (or edits first — their call, not ours).
    if let Some(task) = initial_task {
        if app.trust_review_pending {
            for c in task.chars() {
                app.insert_char(c);
            }
            app.ui
                .push_notice("task staged in the input; answer the config review first");
        } else {
            let msg = app.submit_text(task);
            app.spawn_run(Arc::clone(&agent), msg);
        }
    }

    let mut term_events = EventStream::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

    // Input responsiveness (owner report: "not responsive in cmd, pwsh"). A full
    // redraw costs a frame, and in a Windows console a frame measures tens of
    // milliseconds — drawing once per *event* made a burst of keys echo at about
    // 30 chars/s (measured on conhost: 99 typed characters took ~3 s to appear).
    // The loop now drains every input event that is already pending before
    // drawing, draws at most once per turn of the loop, and only when something
    // actually changed — so a burst costs one frame, and an idle TUI redraws
    // nothing at all.
    let mut dirty = true;
    let mut frame_due = std::time::Instant::now();
    // Set when an event in the drained batch could have changed the goal, so the badge is
    // re-read once per frame that needs it rather than once per event or once per draw.
    let mut goal_dirty = true;
    // Set when the drained batch contained a turn end, so the goal round driver runs once for
    // that turn rather than once per event or once per frame.
    let mut goal_turn_ended = false;
    loop {
        // M59: the external editor needs the terminal to itself, so the hand-off
        // happens between frames (set by `/editor` or the keybinding) — never
        // inside a key handler, which must not touch the terminal.
        if app.external_editor_requested {
            app.external_editor_requested = false;
            open_external_editor(terminal, &mut app);
            dirty = true;
        }

        // Drain: `timeout(ZERO)` polls once and reports `Elapsed` the moment no
        // event is ready, so this neither waits nor adds latency of its own.
        // Bounded (M94 review): a flood of input must not keep the loop from
        // reaching its frame decision; the remainder is handled by the biased
        // select below, which serves input first.
        let mut quit = false;
        for _ in 0..MAX_INPUT_DRAIN {
            let Ok(Some(Ok(ev))) =
                tokio::time::timeout(std::time::Duration::ZERO, term_events.next()).await
            else {
                break;
            };
            dirty = true;
            if handle_term_event(&mut app, &agent, ev) {
                quit = true;
                break;
            }
        }
        // Coalesce agent output the same way: a streamed answer arrives as one
        // event per delta, and drawing per delta was what made the TUI stutter
        // while the model was talking. Bounded, so a flood cannot starve input.
        for _ in 0..128 {
            match events_rx.try_recv() {
                Ok(ev) => {
                    dirty = true;
                    // A turn boundary is where a goal can have changed without this process
                    // writing it: `create_goal` / `update_goal` run inside a turn, and the
                    // footer's badge is cached rather than read per frame. Checked here rather
                    // than in `apply_with_scroll`, which has no agent to read a goal from.
                    if let epsilon_contracts::AgentEvent::AgentEnd { messages } = &*ev {
                        goal_dirty = true;
                        // **A run that produced nothing does not continue the goal.** A round
                        // that failed at the provider boundary — a rejected request, an expired
                        // credential, a network error — produces no messages, and repeating it
                        // would spend the whole round budget on a loop that cannot make progress.
                        // A successful turn always produces at least an assistant message, so
                        // this is exact rather than a heuristic.
                        goal_turn_ended = !messages.is_empty();
                    }
                    apply_with_scroll(&mut app, &ev);
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    dirty = true;
                    app.ui
                        .push_notice(format!("… {n} events dropped (subscriber lag)"));
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    quit = true;
                    break;
                }
            }
        }
        if quit {
            break;
        }

        // The goal's round driver. Decided here, once per drained batch, so a turn end cannot
        // start two rounds and a round cannot start before the frame that announced it. It
        // re-reads the goal rather than trusting the event, so a tool that completed the goal
        // during the turn stops the loop on the same frame.
        if goal_turn_ended {
            goal_turn_ended = false;
            if app.continue_goal(&agent) {
                dirty = true;
                goal_dirty = true;
            }
        }

        // A round whose gates were checked arrives here rather than at the line above,
        // because the check ran on its own task. Drained every frame, so a gate that took
        // thirty seconds costs thirty seconds of nothing happening and not thirty seconds of
        // a dead terminal.
        while let Some(round) = app.take_goal_round() {
            if app.start_gate_round(&agent, round) {
                dirty = true;
                goal_dirty = true;
            }
        }

        // Frame ceiling (~30 fps). Without it a fast provider stream would still
        // draw once per batch, and every streaming frame re-wraps the whole
        // in-flight answer — the cost that made long answers feel frozen.
        if dirty && std::time::Instant::now() >= frame_due {
            // The header's context figure is the agent's estimate of the next
            // request, and the draw path holds no agent — so it is mirrored here,
            // on the frame boundary, rather than read by a widget. Taking it here
            // rather than when an event arrives is what makes the number describe
            // the same history the frame is about to show.
            app.mirror_context_size(&agent);
            if goal_dirty {
                app.refresh_goal(&agent);
                goal_dirty = false;
            }
            let _ = terminal.draw(|f| draw(f, &mut app));
            // `/copy` parks an OSC 52 sequence instead of writing it, because a
            // key handler must not touch the terminal and the sequence must not
            // land inside the frame ratatui is handing over. Here the frame is
            // out, so it goes to the same stream the frame went to.
            flush_osc52(&mut app, terminal.backend_mut());
            dirty = false;
            frame_due = std::time::Instant::now() + FRAME_MIN;
        }

        // Only a due frame needs a timer here; when nothing is dirty the loop
        // parks on real events instead (the sleep is effectively infinite).
        let wait = if dirty {
            frame_due.saturating_duration_since(std::time::Instant::now())
        } else {
            std::time::Duration::from_secs(3600)
        };

        tokio::select! {
            // Input first: a busy agent event stream must never delay a keystroke.
            biased;

            maybe_ev = term_events.next() => {
                dirty = true;
                match maybe_ev {
                    Some(Ok(ev)) => {
                        if handle_term_event(&mut app, &agent, ev) {
                            break;
                        }
                    }
                    // The stream ended or errored — the previous shape stopped here.
                    _ => break,
                }
            },

            // Approvals outrank agent output: a pending decision must not wait
            // behind a stream of deltas.
            ask = approvals.next_ask() => {
                if let Some(pending) = ask {
                    app.scroll_from_bottom = 0;
                    app.pending_ask = Some(pending);
                    dirty = true;
                }
            },

            ev = events_rx.recv() => {
                dirty = true;
                match ev {
                    Ok(ev) => apply_with_scroll(&mut app, &ev),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        app.ui.push_notice(format!("… {n} events dropped (subscriber lag)"));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            },

            maybe_notice = notices_rx.recv() => {
                if let Some(text) = maybe_notice {
                    app.ui.push_notice(text);
                    dirty = true;
                }
            },

            maybe_oauth = oauth_rx.recv() => {
                if let Some(event) = maybe_oauth {
                    app.apply_oauth_event(event);
                    dirty = true;
                }
            },

            maybe_fetched = fetched_rx.recv() => {
                if let Some((provider, result)) = maybe_fetched
                    && let Some(text) = app.record_live_result(provider, result)
                {
                    app.ui.push_notice(text);
                    dirty = true;
                }
            },

            // A pending frame must not wait for the next event: wake when due.
            _ = tokio::time::sleep(wait) => {}

            _ = tick.tick() => {
                if app.ui.running() {
                    app.spinner_phase = app.spinner_phase.wrapping_add(1);
                    dirty = true;
                }
            },
        }
    }
    0
}
