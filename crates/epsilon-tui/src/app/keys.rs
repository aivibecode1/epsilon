//! Turning key events into application changes.
//!
//! One function per keyboard owner, and the order they are consulted in is the
//! focus model — which is why they live together rather than beside whatever
//! they happen to mutate. Read [`handle_key`] top to bottom and you have the
//! precedence: quits, then a pending approval, then the auth prompts, then the
//! overlays, then the ordinary composer.
//!
//! Nothing here touches the terminal. Drawing happens between frames, in
//! `app.rs`, so a key handler can never leave the screen half-drawn.
//!
//! Compiled as part of the `app` module, so `super::*` reaches `App` and its
//! private methods.

use super::*;

/// M32: insert one char into the masked API-key buffer at `auth_key_cursor`.
pub(crate) fn auth_insert(app: &mut App, c: char) {
    let byte = app
        .auth_key
        .char_indices()
        .nth(app.auth_key_cursor)
        .map(|(i, _)| i)
        .unwrap_or(app.auth_key.len());
    app.auth_key.insert(byte, c);
    app.auth_key_cursor += 1;
}

/// M32c: read the system clipboard (arboard). Used by the Ctrl+V fallback when
/// the terminal does not deliver bracketed `TermEvent::Paste`. Best-effort:
/// empty string on failure so the caller can show a hint.
///
/// Reading stays on the native path on purpose: OSC 52 can *write* a remote
/// clipboard but reading it back needs a round-trip the terminal may never
/// answer, which is a paste that hangs rather than a paste that fails. See
/// [`crate::clipboard`].
pub(crate) fn clipboard_text() -> String {
    crate::clipboard::local_text()
}

/// M32: paste (bracketed or Ctrl+V) into the masked API-key buffer. Keys are
/// single-line — newlines/spaces at the edges are trimmed so a trailing
/// newline from the clipboard does not create an invisible mismatch.
pub(crate) fn handle_auth_paste(app: &mut App, text: String) {
    let cleaned = text.trim().replace(['\r', '\n'], "");
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        return;
    }
    for c in cleaned.chars() {
        auth_insert(app, c);
    }
}

/// M32: masked API-key prompt key handler — owns every key while open so the
/// transcript editor does not see them. Esc cancels, Enter saves to
/// `~/.config/epsilon/auth.json` via `epsilon_llm::auth`.
/// Re-derive the visible rows from `timeline_all`, keeping the selection on the
/// same entry when it survives.
///
/// **One implementation, because the filter and the two fold chords all change
/// what is on screen.** Written three times, the three copies would drift in
/// their selection handling -- and a fold that quietly moves the highlight onto
/// a different row is the kind of bug that makes a view feel haunted.
fn refilter(app: &mut App) {
    let labels: std::collections::HashMap<String, String> = app
        .timeline_all
        .iter()
        .filter_map(|r| r.label.clone().map(|l| (r.id.clone(), l)))
        .collect();
    let keep = app
        .timeline_entries
        .get(app.timeline_index)
        .map(|r| r.id.clone());
    app.timeline_entries = super::commands::visible_rows(
        &app.timeline_all,
        app.tree_filter,
        &labels,
        &app.tree_query,
        &app.folded,
    );
    app.timeline_index = keep
        .and_then(|id| app.timeline_entries.iter().position(|r| r.id == id))
        .unwrap_or(0)
        .min(app.timeline_entries.len().saturating_sub(1));
}

pub(crate) fn begin_auth_key_prompt(app: &mut App, provider: String) {
    app.auth_prompt_open = true;
    app.auth_provider = provider.clone();
    app.auth_key.clear();
    app.auth_key_cursor = 0;
    // M52: name the subscription route when the provider has one, so the
    // ceremony is discoverable from the prompt a user actually lands in.
    let oauth_hint = if epsilon_llm::oauth_login::login_flow(&provider).is_some() {
        " · Ctrl+O = subscription login (OAuth)"
    } else {
        ""
    };
    app.ui.push_notice(format!(
        "enter API key for '{provider}' — paste the key (masked), Enter saves to ~/.config/epsilon/auth.json, Esc cancels{oauth_hint}"
    ));
}

///
/// Deliberately tiny: navigation, and a way out. There is no editable query yet — the
/// query comes from `/search <text>` and refining it means leaving and retyping, which
/// `search.rs` records as the obvious next slice rather than a half-built filter box.
pub(crate) fn handle_search_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            // M196: Esc cancels an armed jump BEFORE it closes the search. An
            // armed jump holds no other key, so leaving it armed would mean the
            // next character the user types moves the cursor instead of being
            // typed.
            if app.jump_mode.take().is_some() {
                app.ui.push_notice("jump cancelled");
                return false;
            }
            app.search_close();
            true
        }
        KeyCode::Down | KeyCode::Enter | KeyCode::Char('n') => {
            app.search_step(true);
            true
        }
        KeyCode::Up | KeyCode::PageUp | KeyCode::Char('p') => {
            app.search_step(false);
            true
        }
        KeyCode::Char('q') => {
            app.search_close();
            true
        }
        _ => true,
    }
}

fn handle_auth_key(app: &mut App, agent: &Arc<Agent>, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            // M52: dropping the code sender disconnects the login thread,
            // which reports the cancellation itself.
            let was_oauth = app.oauth_code_tx.take().is_some();
            app.auth_prompt_open = false;
            app.auth_provider.clear();
            app.auth_key.clear();
            app.auth_key_cursor = 0;
            app.ui.push_notice(if was_oauth {
                "OAuth code entry cancelled"
            } else {
                "API key entry cancelled"
            });
            false
        }
        // Ctrl+V paste inside the masked prompt (fallback when bracketed paste
        // is not available — e.g. conhost, legacy Windows console). Reads the
        // system clipboard via arboard and inserts masked, like bracketed paste.
        KeyCode::Char('v' | 'V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let text = clipboard_text();
            if text.trim().is_empty() {
                app.ui
                    .push_notice("clipboard empty — copy a key first, then Ctrl+V");
            } else {
                handle_auth_paste(app, text);
            }
            false
        }
        // Shift+Insert is the other common paste chord on Windows/Linux
        // terminals. Treat it the same as Ctrl+V.
        KeyCode::Insert if key.modifiers.contains(KeyModifiers::SHIFT) => {
            let text = clipboard_text();
            if text.trim().is_empty() {
                app.ui
                    .push_notice("clipboard empty — copy a key first, then Shift+Insert");
            } else {
                handle_auth_paste(app, text);
            }
            false
        }
        // M52: discoverability — a pending key prompt for an OAuth-capable
        // provider can switch into the subscription ceremony without typing
        // `/login <provider> oauth` from scratch.
        KeyCode::Char('o' | 'O') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let provider = app.auth_provider.clone();
            app.auth_prompt_open = false;
            app.auth_provider.clear();
            app.auth_key.clear();
            app.auth_key_cursor = 0;
            app.oauth_code_tx = None;
            if provider.is_empty() || epsilon_llm::oauth_login::login_flow(&provider).is_none() {
                app.ui.push_notice(
                    "no OAuth login for that provider — /login <provider> <key> stores a key",
                );
            } else {
                app.start_oauth_login(provider);
            }
            false
        }
        KeyCode::Enter => {
            if app.oauth_code_tx.is_some() {
                let value = app.auth_key.trim().to_string();
                if value.is_empty() {
                    app.ui
                        .push_notice("empty — paste the redirect URL or code, or Esc to cancel");
                    return false;
                }
                let sender = app.oauth_code_tx.take().expect("checked");
                let _ = sender.send(value);
                app.auth_prompt_open = false;
                app.auth_provider.clear();
                app.auth_key.clear();
                app.auth_key_cursor = 0;
                app.ui
                    .push_notice("code handed to the login flow — exchanging…");
                return false;
            }
            let provider = app.auth_provider.clone();
            let key_val = app.auth_key.trim().to_string();
            if key_val.is_empty() {
                app.ui
                    .push_notice("empty key — paste a key or press Esc to cancel");
                return false;
            }
            match epsilon_llm::auth::save_credential(&provider, &key_val) {
                Ok(path) => {
                    app.ui
                        .push_notice(format!("saved key for '{provider}' → {}", path.display()));
                    let live = agent.try_hot_swap_to_real();
                    if live {
                        app.ui.push_notice(format!(
                            "now live on '{provider}' — next prompt hits the real API."
                        ));
                    } else {
                        app.ui.push_notice(format!(
                            "Restart epsilon to use '{provider}' — next prompt will hit the real API (was offline demo on '{m}').",
                            m = app.model_id
                        ));
                    }
                    app.invalidate_picker();
                }
                Err(e) => {
                    app.ui
                        .push_notice(format!("not saved for '{provider}': {e}"));
                }
            }
            app.auth_prompt_open = false;
            app.auth_provider.clear();
            app.auth_key.clear();
            app.auth_key_cursor = 0;
            false
        }
        KeyCode::Backspace => {
            if app.auth_key_cursor > 0 {
                let before: String = app.auth_key.chars().take(app.auth_key_cursor - 1).collect();
                let after: String = app.auth_key.chars().skip(app.auth_key_cursor).collect();
                app.auth_key = before + &after;
                app.auth_key_cursor -= 1;
            }
            false
        }
        KeyCode::Delete => {
            if app.auth_key_cursor < app.auth_key.chars().count() {
                let before: String = app.auth_key.chars().take(app.auth_key_cursor).collect();
                let after: String = app.auth_key.chars().skip(app.auth_key_cursor + 1).collect();
                app.auth_key = before + &after;
            }
            false
        }
        KeyCode::Left => {
            app.auth_key_cursor = app.auth_key_cursor.saturating_sub(1);
            false
        }
        KeyCode::Right => {
            app.auth_key_cursor = (app.auth_key_cursor + 1).min(app.auth_key.chars().count());
            false
        }
        KeyCode::Home => {
            app.auth_key_cursor = 0;
            false
        }
        KeyCode::End => {
            app.auth_key_cursor = app.auth_key.chars().count();
            false
        }
        KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+U — kill to start (pi editor convention).
            let after: String = app.auth_key.chars().skip(app.auth_key_cursor).collect();
            app.auth_key = after;
            app.auth_key_cursor = 0;
            false
        }
        KeyCode::Char('w' | 'W') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+W — kill word back.
            let start = crate::editor_ops::word_left(&app.auth_key, app.auth_key_cursor);
            let before: String = app.auth_key.chars().take(start).collect();
            let after: String = app.auth_key.chars().skip(app.auth_key_cursor).collect();
            app.auth_key = before + &after;
            app.auth_key_cursor = start;
            false
        }
        KeyCode::Char(c) => {
            // Printable char typed directly (covers paste via key events on terminals
            // without bracketed paste). Masked — not echoed.
            auth_insert(app, c);
            false
        }
        _ => false,
    }
}

/// M94 review: keys while an approval is pending. Extracted so the routing lives
/// in exactly one place — `handle_key` calls it BEFORE the auth branches, since
/// the dialog is painted over them.
///
/// Fail closed, identity-based: `App::approval_painted_id` names the ask the last
/// frame actually painted (`draw` writes it, the undersized fallback clears it).
/// An ask whose id does not match is refused with a notice and stays pending — a
/// second parked ask seated inside the same frame window, or any ask the dialog
/// could not fit for. A blind `y`/`a` would authorize work the user never saw and
/// a blind Esc would silently deny it.
pub(crate) fn handle_approval_key(app: &mut App, key: KeyEvent) -> bool {
    let Some(ask_id) = app
        .pending_ask
        .as_ref()
        .map(|ask| ask.request.request_id.clone())
    else {
        return false;
    };
    // The painted id is the ONLY ask these keys may decide: a key handled after a
    // NEW ask is seated (the channel is FIFO; parallel tool calls park several)
    // and before the next draw must not answer what no frame has shown.
    if app.approval_painted_id.as_deref() != Some(ask_id.as_str()) {
        // The floor is stated from the layout constants, not from a literal:
        // this sentence said "need 60×12" while `Regions::too_small` refused at
        // 44×10, so it named a size the layout did not want and told a reader
        // who obeyed it nothing about why the dialog was still missing.
        app.ui.push_notice(format!(
            "approval is not visible yet — it appears on the next frame; \
             enlarge the terminal to at least {}×{} if it does not",
            crate::ui::layout::MIN_WIDTH,
            crate::ui::layout::MIN_HEIGHT,
        ));
        return false;
    }
    // Guidance mode: Enter answers the ask too (DenyWithGuidance). Gated by the
    // same painted-id check above, so the refusal covers it as well.
    if app.guidance_mode {
        match key.code {
            KeyCode::Enter => {
                let guidance = std::mem::take(&mut app.input);
                app.cursor_chars = 0;
                app.guidance_mode = false;
                if let Some(ask) = app.pending_ask.take() {
                    ask.reply(PermissionDecision::DenyWithGuidance { guidance });
                }
            }
            KeyCode::Esc => {
                app.guidance_mode = false;
                app.input.clear();
                app.cursor_chars = 0;
            }
            KeyCode::Backspace => app.backspace(),
            KeyCode::Char(c) => app.insert_char(c),
            _ => {}
        }
        return false;
    }
    match key.code {
        KeyCode::Char('y' | 'Y') => {
            if let Some(ask) = app.pending_ask.take() {
                ask.reply(PermissionDecision::AllowOnce);
            }
        }
        KeyCode::Char('a' | 'A') => {
            if let Some(ask) = app.pending_ask.take() {
                ask.reply(PermissionDecision::AlwaysSession);
            }
        }
        KeyCode::Char('n' | 'N') | KeyCode::Esc => {
            if let Some(ask) = app.pending_ask.take() {
                ask.reply(PermissionDecision::Deny);
            }
        }
        KeyCode::Char('e' | 'E') => {
            app.guidance_mode = true;
            app.input.clear();
            app.cursor_chars = 0;
        }
        _ => {}
    }
    false
}

/// Handle one key press; returns true when the app should quit.
pub(crate) fn handle_key(app: &mut App, agent: &Arc<Agent>, key: KeyEvent) -> bool {
    // M30: mirror the steer queue length on every key event while running so
    // draw_status can show a count badge without needing the agent ref. Kept
    // above the modal branches below so the badge keeps up in every state.
    if app.ui.running() {
        app.steer_queue_len = agent.steer_queue_len();
    }
    // M94 re-review (coherence): with an ask pending the approval keys route
    // BEFORE the auth branches. The dialog is painted last — over the masked
    // `/login` key prompt and the provider picker — so it must own the keyboard
    // too; otherwise `y`/`a`/`n`/`Esc`/Enter went into a buffer the user could
    // not see (Enter there saves to `auth.json`) while they were looking at a
    // dialog they could not answer. The auth state is left untouched, so the
    // login flow resumes exactly where it was once the ask clears. The exit
    // keybind keeps its precedence over the approval branch (quitting drops the
    // parked ask, which resolves to Deny — fail closed), exactly as before.
    // M157: an open transcript search owns the keyboard, like every other overlay, and
    // for the same reason the approval dialog does: keys sent anywhere the user cannot see
    // them are keys the user believes they pressed and did not.
    //
    // It is placed after the approval branch on purpose. A pending permission ask is a
    // question about *doing work*, and a search is a question about *reading*; answering the
    // first with `n` should deny the tool, not silently move the viewport.
    if app.search.is_some() {
        return handle_search_key(app, key);
    }
    if app.pending_ask.is_some() && !app.keybindings.matches("app.exit", &key) {
        return handle_approval_key(app, key);
    }
    // M32: masked API-key prompt takes all keys while open (Esc cancels, Enter saves).
    if app.auth_prompt_open {
        return handle_auth_key(app, agent, key);
    }
    // M32b: provider picker for bare `/login` — filtered by `input`, Enter opens
    // the masked key prompt for the highlighted provider.
    if app.auth_pick_open {
        let rows = crate::picker::build_provider_rows(&app.model_id, &app.customs);
        let query = app.input.trim().to_ascii_lowercase();
        let filtered: Vec<_> = if query.is_empty() {
            rows.iter().enumerate().collect()
        } else {
            rows.iter()
                .enumerate()
                .filter(|(_, p)| p.id.to_ascii_lowercase().contains(&query))
                .collect()
        };
        match key.code {
            KeyCode::Esc => {
                app.auth_pick_open = false;
                app.auth_pick_index = 0;
                app.input.clear();
                app.cursor_chars = 0;
            }
            KeyCode::Enter => {
                if let Some((_, prow)) = filtered.get(app.auth_pick_index) {
                    let chosen = prow.id.clone();
                    app.auth_pick_open = false;
                    app.auth_pick_index = 0;
                    app.input.clear();
                    app.cursor_chars = 0;
                    begin_auth_key_prompt(app, chosen);
                }
            }
            KeyCode::Up => {
                app.auth_pick_index = app
                    .auth_pick_index
                    .saturating_sub(1)
                    .min(filtered.len().saturating_sub(1));
            }
            KeyCode::Down => {
                if !filtered.is_empty() {
                    app.auth_pick_index = (app.auth_pick_index + 1).min(filtered.len() - 1);
                }
            }
            // Ctrl+V / Shift+Insert paste into the provider filter — when the
            // terminal sends paste as a key chord instead of bracketed paste, let
            // the user paste a provider id from the clipboard.
            KeyCode::Char('v' | 'V') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let text = clipboard_text();
                let text = text.trim().to_string();
                if !text.is_empty() {
                    for c in text.chars() {
                        app.insert_char(c);
                    }
                    app.auth_pick_index = 0;
                } else {
                    app.ui
                        .push_notice("clipboard empty — copy text first, then Ctrl+V");
                }
            }
            KeyCode::Insert if key.modifiers.contains(KeyModifiers::SHIFT) => {
                let text = clipboard_text();
                let text = text.trim().to_string();
                if !text.is_empty() {
                    for c in text.chars() {
                        app.insert_char(c);
                    }
                    app.auth_pick_index = 0;
                } else {
                    app.ui
                        .push_notice("clipboard empty — copy text first, then Shift+Insert");
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                app.auth_pick_index = 0;
            }
            KeyCode::Char(c) => {
                app.insert_char(c);
                app.auth_pick_index = 0;
            }
            _ => {}
        }
        return false;
    }
    // `app.clear` is checked FIRST, and that ordering is the whole fix. Both
    // `app.clear` and `app.exit` were bound to ctrl+c; `app.exit` was matched here,
    // so Ctrl+C quit the process and discarded the draft while `/hotkeys` advertised
    // "Clear editor" for that same chord. pi's own defaults separate them: `app.clear`
    // is ctrl+c and `app.exit` is ctrl+d. Epsilon keeps ctrl+d for forward-delete, so
    // `app.exit` stays on ctrl+c/ctrl+q — but it must only win when there is nothing
    // to clear, which is the behaviour a user pressing Ctrl+C in an editor expects.
    if app.keybindings.matches("app.clear", &key) {
        app.clear_editor();
        return false;
    }
    if app.keybindings.matches("app.exit", &key) {
        return true;
    }

    // Plan 009: timeline picker — Enter navigates, Esc dismisses, arrows move.
    // The label editor owns the keyboard before the picker does: it is a text
    // field, so every printable key is content rather than a command.
    if app.label_prompt_open {
        match key.code {
            KeyCode::Esc => {
                app.label_prompt_open = false;
                app.label_prompt_target = None;
                app.label_prompt_text.clear();
            }
            KeyCode::Enter => {
                let target = app.label_prompt_target.take();
                let text = std::mem::take(&mut app.label_prompt_text);
                app.label_prompt_open = false;
                let Some(id) = target else { return false };
                let Some(store) = agent.session_store() else {
                    return false;
                };
                // An empty box CLEARS rather than writing a blank label, which is
                // pi's rule (`if (entry.label) ... else delete`) and is also what
                // `set_label` normalises. So the editor can clear a label without
                // needing a separate "delete" gesture.
                let label = if text.trim().is_empty() {
                    None
                } else {
                    Some(text.as_str())
                };
                // **The store lock is scoped to the write and released before the
                // re-projection.** M196 held it across `reproject_current_branch`,
                // which locks the same store again -- a self-deadlock that hung
                // the test for 60s rather than failing, which is worse. If the
                // write and the read of the result ever need to be one critical
                // section, the answer is a store method that does both, not a
                // longer-held lock.
                let written = {
                    let Ok(mut st) = store.lock() else {
                        return false;
                    };
                    st.set_label(&id, label)
                };
                match written {
                    Ok(_) => {
                        let what = label.unwrap_or("label cleared");
                        // M196: the `Label` entry is appended to the current lane,
                        // so the leaf has just moved (pi does the same --
                        // `session-manager.ts:1061`). Without this re-projection
                        // the transcript on screen would describe a session the
                        // store no longer holds: the "the view no longer matches
                        // the session" state the timeline handler already names.
                        app.ui.clear_lines();
                        match agent.reproject_current_branch() {
                            Ok(n) => {
                                let messages = agent.history();
                                app.ui.hydrate_from_history(&messages);
                                app.ui
                                    .push_notice(format!("{what} · {n} messages on this branch"));
                            }
                            Err(why) => app.ui.push_notice(format!(
                                "{what}, but the transcript could not be re-read ({why})"
                            )),
                        }
                    }
                    Err(e) => app.ui.push_notice(format!("could not set label: {e}")),
                }
            }
            KeyCode::Backspace => {
                if app.label_prompt_cursor > 0 {
                    let at = app.label_prompt_cursor - 1;
                    app.label_prompt_text.remove(at);
                    app.label_prompt_cursor = at;
                }
            }
            KeyCode::Left => app.label_prompt_cursor = app.label_prompt_cursor.saturating_sub(1),
            KeyCode::Right => {
                app.label_prompt_cursor =
                    (app.label_prompt_cursor + 1).min(app.label_prompt_text.chars().count());
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.label_prompt_text.insert(app.label_prompt_cursor, c);
                app.label_prompt_cursor += 1;
            }
            _ => {}
        }
        return false;
    }

    if app.timeline_open {
        match key.code {
            // M196: the search field, drawn inside the panel. While it is open
            // every printable key is content, so this arm comes first.
            //
            // `_ if`, not a bare `if`: this arm is chosen by APP STATE, not by
            // the key pressed, and a bare `if` is not a match arm at all. The
            // inner `match key.code` then decides what the key means.
            _ if app.tree_searching => {
                match key.code {
                    KeyCode::Esc => {
                        // pi: Esc clears the query first and only closes the
                        // picker on a second press (`tree-selector.ts:1030-1035`).
                        // Losing a typed query to one stray Esc is the kind of
                        // thing a reader remembers badly.
                        if app.tree_query.is_empty() {
                            app.tree_searching = false;
                        } else {
                            app.tree_query.clear();
                            refilter(app);
                        }
                    }
                    KeyCode::Enter | KeyCode::Down | KeyCode::Up => {
                        app.tree_searching = false;
                    }
                    KeyCode::Backspace => {
                        app.tree_query.pop();
                        refilter(app);
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.tree_query.push(c);
                        refilter(app);
                    }
                    _ => {}
                }
                return false;
            }
            KeyCode::Esc => {
                app.timeline_open = false;
                app.timeline_index = 0;
                app.timeline_entries.clear();
            }
            KeyCode::Enter => {
                if let Some(row) = app.timeline_entries.get(app.timeline_index).cloned() {
                    let Some(store) = agent.session_store() else {
                        return false;
                    };
                    // Navigate by the row's own id. M196: this used to look the
                    // seq up inside `branch_entries()` and navigate to whatever it
                    // found there -- which is the *active lane*, so selecting a row
                    // on an abandoned branch found nothing and silently did
                    // nothing. The row already carries the id the tree gave it, so
                    // the lookup was both redundant and the bug.
                    let jumped = if let Ok(mut st) = store.lock() {
                        st.navigate_to(&row.id).is_ok()
                    } else {
                        false
                    };
                    if jumped {
                        // Re-project the transcript. Moving the store's leaf without
                        // this left the user reading a conversation that no longer
                        // described the session -- which is why `/tree` was a viewer.
                        app.ui.clear_lines();
                        match agent.reproject_current_branch() {
                            Ok(n) => {
                                let messages = agent.history();
                                app.ui.hydrate_from_history(&messages);
                                app.ui.push_notice(format!(
                                    "jumped to #{} · {n} messages on this branch",
                                    row.seq
                                ));
                            }
                            // The leaf already moved, so a failed re-projection leaves
                            // the store and the screen disagreeing. Say exactly that,
                            // rather than showing a transcript that is not the session.
                            Err(why) => app.ui.push_notice(format!(
                                "jumped to #{}, but the transcript could not be re-read \
                                 ({why}) — the view below no longer matches the session",
                                row.seq
                            )),
                        }
                        // The branch is NOT listed here any more.
                        //
                        // It used to push one notice per entry, "so the user can see
                        // what they jumped into without re-opening the picker". Jumping
                        // to entry 3 of a 400-entry session therefore pushed 400 rows
                        // into the transcript and scrolled the reader away from the
                        // message they had just jumped to — the opposite of the intent,
                        // and the more so because the lines are *notices*, which sit at
                        // the bottom and push everything above them up.
                        //
                        // The transcript above is now the branch, re-projected and
                        // re-read; re-printing it as a list told the user nothing they
                        // could not read directly. The count is what was actually worth
                        // keeping, and the jump notice already carries it.
                    }
                    app.timeline_open = false;
                    app.timeline_index = 0;
                    app.timeline_entries.clear();
                    app.timeline_all.clear();
                }
            }
            // `/` opens the field, the conventional "search here" chord.
            KeyCode::Char('/') => {
                app.tree_searching = true;
            }
            // M196: cycle the filter. Re-derived from `timeline_all`, never from
            // the already-filtered list, so `f` `f` returns to where it started
            // instead of narrowing twice.
            KeyCode::Char('f') | KeyCode::Char('F') => {
                app.tree_filter = app.tree_filter.next();
                refilter(app);
            }
            // M196: fork the session at the SELECTED row, which is the whole
            // point of the tree. `/fork` forks at the leaf, so before the tree
            // existed there was no way to say "keep everything up to here and
            // try something else" from the TUI at all -- the primitive existed
            // (and had an RPC caller from M192) with no TUI path to reach it.
            //
            // A user message is the only sensible fork point: a fork of an
            // assistant turn or a tool result leaves a session that ends
            // mid-answer, which is a file pi can produce and a user cannot use.
            KeyCode::Char('k') | KeyCode::Char('K') => {
                let Some(row) = app.timeline_entries.get(app.timeline_index).cloned() else {
                    return false;
                };
                if row.kind != "user" {
                    app.ui.push_notice(format!(
                        "fork at a USER message — #{} [{}] is {}",
                        row.seq, row.kind, row.kind
                    ));
                    return false;
                }
                let Some(store) = agent.session_store() else {
                    return false;
                };
                // **The session file's own parent directory**, which is what
                // `/fork` uses -- so both fork paths write their session where the
                // user's existing ones already live, rather than into a directory
                // only the session browser knows to look in. Guessing a
                // `sessions_dir()` here would have put the fork somewhere the
                // browser would not list it.
                let forked = {
                    let Ok(st) = store.lock() else {
                        return false;
                    };
                    let dest = st.path().parent().map(|d| d.to_path_buf());
                    let Some(dest) = dest else {
                        app.ui
                            .push_notice("fork: the session file has no parent directory");
                        return false;
                    };
                    st.fork_branch_at(&row.id, dest)
                };
                match forked {
                    Ok(path) => {
                        app.ui.push_notice(format!(
                            "forked at #{} — the new session is {}",
                            row.seq,
                            path.display()
                        ));
                        app.timeline_open = false;
                        app.timeline_entries.clear();
                        app.timeline_all.clear();
                    }
                    Err(e) => app
                        .ui
                        .push_notice(format!("fork at #{} failed: {e}", row.seq)),
                }
            }
            // M196: label the selected row. pi binds this in its tree selector
            // (`updateNodeLabel`); the store half (`SessionStore::set_label`)
            // existed and was tested, with no TUI half at all -- the same
            // "faithful port, no production caller" shape.
            // pi binds this to `shift+l` (`keybindings.ts:159`), NOT to a bare
            // `l` -- a bare letter would be a plausible "navigate left" chord the
            // day someone adds one. Matching the reference keeps the muscle
            // memory and leaves `l` free.
            KeyCode::Char('L') => {
                if let Some(row) = app.timeline_entries.get(app.timeline_index).cloned() {
                    // Start from the label already on the row, so re-labelling is
                    // an edit rather than a retype. pi seeds the editor the same
                    // way (`tree-selector.ts:1287`, `currentLabel`).
                    let existing: String = row.label.clone().unwrap_or_default();
                    app.label_prompt_cursor = existing.chars().count();
                    app.label_prompt_text = existing;
                    app.label_prompt_target = Some(row.id);
                    app.label_prompt_open = true;
                    app.timeline_open = false;
                }
            }
            KeyCode::Up => {
                let total = app.timeline_entries.len();
                app.timeline_index = app
                    .timeline_index
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
            }
            KeyCode::Down => {
                let total = app.timeline_entries.len();
                if total > 0 {
                    app.timeline_index = (app.timeline_index + 1).min(total - 1);
                }
            }
            // M196: fold/unfold, on pi's chords (`keybindings.ts:150-157`:
            // Ctrl+Left folds-or-up, Ctrl+Right unfolds-or-down).
            //
            // The "or" is the whole behaviour: on a foldable row the chord
            // folds, and on a leaf it moves the cursor instead. That is why a
            // fold on a leaf is not a silent no-op -- the key still does the
            // other thing pi says it does.
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let foldable = app
                    .timeline_entries
                    .get(app.timeline_index)
                    .cloned()
                    .filter(|r| super::commands::has_children(r, &app.timeline_all));
                match foldable {
                    Some(r) => {
                        app.folded.insert(r.id);
                        refilter(app);
                    }
                    None => {
                        app.timeline_index = app.timeline_index.saturating_sub(1);
                    }
                }
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let folded_row = app
                    .timeline_entries
                    .get(app.timeline_index)
                    .cloned()
                    .filter(|r| app.folded.contains(&r.id));
                match folded_row {
                    Some(r) => {
                        app.folded.remove(&r.id);
                        refilter(app);
                    }
                    None => {
                        let total = app.timeline_entries.len();
                        if total > 0 {
                            app.timeline_index = (app.timeline_index + 1).min(total - 1);
                        }
                    }
                }
            }
            // pi binds `app.message.copy` inside the tree
            // (`tree-selector.ts:1029-1030` -> `copySelected` at `:627`), and the
            // key was already bound here at `keybindings.rs:140` — but the arm
            // that honoured it sat 620 lines BELOW this block, which returns for
            // every key. So inside the tree the copy key did nothing at all, with
            // no notice: a chord advertised by `/hotkeys` and printed in the panel,
            // swallowed by a `_ => {}`. The `k`/`⇧L`/`f`/`/` arms worked only
            // because they match `KeyCode::Char` directly.
            //
            // Routed through the same command the global arm uses, so the OSC-52
            // parking, the multiplexer choice, and the notice naming which path
            // landed stay in one place.
            // A statement, not the global arm's tail expression: the tree's
            // match arms all end in `;` because the block's value is discarded
            // and `run_command` returns `bool`.
            _ if app.keybindings.matches("app.message.copy", &key) => {
                app.run_command(agent, crate::picker::Command::Copy);
            }
            _ => {}
        }
        return false;
    }

    // Command palette (M16c) captures keys before everything else.
    if app.palette_open {
        match key.code {
            KeyCode::Esc => {
                app.palette_open = false;
                app.picker_index = 0;
                app.input.clear();
                app.cursor_chars = 0;
            }
            KeyCode::Enter => {
                let entries = crate::picker::filter_palette(&app.input);
                if let Some(entry) = entries.get(app.picker_index) {
                    let command = entry.command.clone();
                    app.palette_open = false;
                    app.picker_index = 0;
                    app.input.clear();
                    app.cursor_chars = 0;
                    if let Some(parsed) = crate::picker::parse_command(&command) {
                        return app.run_command(agent, parsed);
                    }
                }
            }
            KeyCode::Up => {
                let total = crate::picker::filter_palette(&app.input).len();
                app.picker_index = app
                    .picker_index
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
            }
            KeyCode::Down => {
                let total = crate::picker::filter_palette(&app.input).len();
                if total > 0 {
                    app.picker_index = (app.picker_index + 1).min(total - 1);
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                app.invalidate_picker();
            }
            KeyCode::Char(c) => {
                app.insert_char(c);
                app.picker_index = 0;
            }
            _ => {}
        }
        return false;
    }

    // M57: the session browser owns its keys while open. The filter IS the
    // input buffer (same as the model picker), so typing/backspace reuse the
    // editor's own grapheme-safe paths.
    if app.session_browser.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.session_browser = None;
                app.input.clear();
                app.cursor_chars = 0;
            }
            KeyCode::Enter => {
                let picked = app
                    .session_browser
                    .as_ref()
                    .and_then(|b| b.rows.get(b.index))
                    .map(|r| r.path.clone());
                if let Some(path) = picked {
                    app.session_browser = None;
                    app.input.clear();
                    app.cursor_chars = 0;
                    match app.switch_session_file(agent, &path, None) {
                        Ok(count) => app.ui.push_notice(format!(
                            "resumed {} messages from {} (history restored, store swapped)",
                            count,
                            path.display()
                        )),
                        Err(e) => app.ui.push_notice(format!("resume failed: {e}")),
                    }
                }
            }
            // Page keys move by the screenful the browser actually showed
            // (`b.capacity`, measured at paint time) -- pi pages by `maxVisible`
            // (`session-selector.ts:612-617`). `max(1)` so a key before the first
            // paint still moves rather than silently doing nothing.
            KeyCode::Up | KeyCode::PageUp => {
                if let Some(b) = app.session_browser.as_mut() {
                    let step = if key.code == KeyCode::PageUp {
                        b.capacity.max(1)
                    } else {
                        1
                    };
                    b.index = b.index.saturating_sub(step);
                }
            }
            KeyCode::Down | KeyCode::PageDown => {
                if let Some(b) = app.session_browser.as_mut() {
                    let last = b.rows.len().saturating_sub(1);
                    let step = if key.code == KeyCode::PageDown {
                        b.capacity.max(1)
                    } else {
                        1
                    };
                    b.index = (b.index + step).min(last);
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                let filter = app.input.clone();
                if let Some(b) = app.session_browser.as_mut() {
                    b.rebuild(&filter);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.insert_char(c);
                let filter = app.input.clone();
                if let Some(b) = app.session_browser.as_mut() {
                    b.rebuild(&filter);
                }
            }
            _ => {}
        }
        return false;
    }

    // Model picker captures keys before the permission dialog (it can open while a
    // run is active, and must always be operable).
    if app.picker_open {
        // Stage-aware navigation (M16c): stage 1 = providers, stage 2 = models.
        let stage2 = app.picker_provider.clone();
        match key.code {
            KeyCode::Esc => {
                if stage2.is_some() {
                    // Stage 2 → back to provider list.
                    app.picker_provider = None;
                    app.picker_index = 0;
                    app.input.clear();
                } else {
                    app.picker_open = false;
                    app.picker_index = 0;
                    app.input.clear();
                    app.cursor_chars = 0;
                    app.unstash_draft();
                }
            }
            KeyCode::Enter => {
                if let Some(provider) = &stage2 {
                    let rows = crate::picker::build_models_for_provider(
                        provider,
                        &app.input,
                        &app.model_id,
                        &app.live_cache,
                        &app.customs,
                    );
                    if let Some(row) = rows.get(app.picker_index) {
                        let id = row.id.clone();
                        app.picker_open = false;
                        app.picker_provider = None;
                        app.picker_index = 0;
                        app.input.clear();
                        app.cursor_chars = 0;
                        app.apply_model(agent, id);
                    }
                } else {
                    // Stage 1: descend into the selected provider's model list.
                    let rows = crate::picker::build_provider_rows(&app.model_id, &app.customs);
                    if let Some(row) = rows.get(app.picker_index) {
                        app.picker_provider = Some(row.id.clone());
                        app.picker_index = 0;
                        app.input.clear();
                        app.cursor_chars = 0;
                        // M20: entering a provider refreshes its live list once
                        // per session (no-op when it already answered).
                        app.request_live(false);
                    }
                }
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Char('f' | 'F')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                unreachable!("handled by guarded arms below")
            }
            _ => {}
        }
        if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
            return false;
        }
        // Remaining keys: Up/Down move, Backspace filters (stage 2 only), chars filter.
        // One helper for the row count, because three arms need it and the model
        // list is built twice in the original (`build_models_for_provider` for
        // stage 2, `build_provider_rows` otherwise). Duplicating that choice a
        // third time is how a page key ends up paging a different list than the
        // arrows do.
        let total = || -> usize {
            if let Some(provider) = &stage2 {
                crate::picker::build_models_for_provider(
                    provider,
                    &app.input,
                    &app.model_id,
                    &app.live_cache,
                    &app.customs,
                )
                .len()
            } else {
                crate::picker::build_provider_rows(&app.model_id, &app.customs).len()
            }
        };
        // pi pages by `maxVisible` (`session-selector.ts:612-617`) -- the screenful
        // the user can actually see, which `picker_capacity` holds from the last
        // paint. `max(1)` so an unpainted picker still moves.
        let page = app.picker_capacity.max(1);
        match key.code {
            KeyCode::Up => {
                let last = total().saturating_sub(1);
                app.picker_index = app.picker_index.saturating_sub(1).min(last);
            }
            KeyCode::PageUp => {
                let last = total().saturating_sub(1);
                app.picker_index = app.picker_index.saturating_sub(page).min(last);
            }
            KeyCode::PageDown => {
                let n = total();
                if n > 0 {
                    app.picker_index = (app.picker_index + page).min(n - 1);
                }
            }
            KeyCode::Down => {
                let n = total();
                if n > 0 {
                    app.picker_index = (app.picker_index + 1).min(n - 1);
                }
            }
            KeyCode::Backspace => {
                app.backspace();
                app.invalidate_picker();
                if stage2.is_none() {
                    return false; // stage-1 filter is display-only; no index fixups
                }
                let total = crate::picker::build_models_for_provider(
                    stage2.as_deref().unwrap_or(""),
                    &app.input,
                    &app.model_id,
                    &app.live_cache,
                    &app.customs,
                )
                .len();
                if app.picker_index >= total {
                    app.picker_index = total.saturating_sub(1);
                }
            }
            KeyCode::Char('f' | 'F') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // M23: Ctrl+F toggles the favorite star on the highlighted row
                // (opencode model_favorite_toggle). Live force-refresh moved to
                // Ctrl+G; auto-fetch on open still covers the common case.
                // Audit fix: stage 1 lists PROVIDERS while picker_rows() holds
                // MODELS — with a filter typed, rows[picker_index] is an
                // invisible model and the star lands on a hidden target.
                // Stage 1 therefore stars the highlighted provider's active or
                // first known model instead of indexing the wrong list.
                let stage1_target = if app.picker_provider.is_none() {
                    crate::picker::build_provider_rows(&app.model_id, &app.customs)
                        .get(app.picker_index)
                        .map(|p| p.id.clone())
                        .and_then(|provider| {
                            epsilon_llm::catalog::lookup_model(&app.model_id)
                                .filter(|m| m.provider == provider)
                                .map(|_| app.model_id.clone())
                                .or_else(|| {
                                    crate::picker::build_models_for_provider(
                                        &provider,
                                        &app.input,
                                        &app.model_id,
                                        &app.live_cache,
                                        &app.customs,
                                    )
                                    .first()
                                    .map(|r| r.id.clone())
                                })
                        })
                } else {
                    None
                };
                match stage1_target {
                    Some(id) => app.toggle_favorite(agent, &id),
                    None => {
                        let rows = crate::picker::build_models_for_provider(
                            app.picker_provider.as_deref().unwrap_or(""),
                            &app.input,
                            &app.model_id,
                            &app.live_cache,
                            &app.customs,
                        );
                        if let Some(row) = rows.get(app.picker_index) {
                            let id = row.id.clone();
                            app.toggle_favorite(agent, &id);
                        }
                    }
                }
            }
            _ if app.keybindings.matches("app.live.refresh", &key) => {
                // Ctrl+G: force a live listing (M13's Ctrl+F job; M20 also
                // auto-fetches without any key).
                app.request_live(true);
            }
            KeyCode::Char(c) => {
                app.insert_char(c);
                app.invalidate_picker();
                app.picker_index = 0;
            }
            _ => {}
        }
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            // M17b: Esc first dismisses the slash popup (pi prompt.autocomplete.hide);
            // then aborts a run; only when idle+quiet does it quit.
            if app.slash_open {
                app.slash_open = false;
                app.slash_index = 0;
                return false;
            }
            if app.at_open {
                app.at_open = false;
                app.at_index = 0;
                return false;
            }
            if app.ui.running() {
                if agent.abort_current_run() {
                    // M30: drain queued steering so it doesn't leak into the
                    // next run, and restore the last message to the editor so
                    // the user can re-edit rather than losing it.
                    let drained = agent.drain_steering_all();
                    if let Some(last_user_text) = drained.into_iter().rev().find_map(|m| {
                        if let Message::User(u) = m {
                            u.content.into_iter().find_map(|c| match c {
                                UserContent::Text { text } => Some(text),
                                _ => None,
                            })
                        } else {
                            None
                        }
                    }) {
                        app.input = last_user_text.clone();
                        app.cursor_chars = last_user_text.chars().count();
                        app.notice("abort requested — queued text restored to editor");
                    } else {
                        app.notice("abort requested — cancelling current run…");
                    }
                }
                false
            } else {
                true
            }
        }
        KeyCode::Enter if app.keybindings.matches("tui.input.submit", &key) => {
            if app.at_open {
                app.apply_at_selection(true);
                return false;
            }
            // M17b: with the popup open, Enter first applies the highlighted
            // suggestion (pi prompt.autocomplete.select) — then FALLS THROUGH to
            // submit, because a completed command like "/clear" is meant to RUN.
            // Bare-text suggestions (no popup) submit unchanged.
            if app.slash_open {
                app.apply_slash_selection();
            }
            app.submit_editor(agent)
        }
        _ if app.keybindings.matches("app.model.select", &key) => {
            // Layout-independent picker access (slash may need AltGr on some layouts).
            if !app.picker_open {
                app.picker_open = true;
                app.picker_index = 0;
                app.stash_draft();
                // M20: same auto-fetch as /models (one code path per open).
                app.request_live(false);
            }
            false
        }
        _ if app.keybindings.matches("app.thinking.cycle", &key) => {
            // Cycle thinking level (pi app.thinking.cycle, Shift+Tab there —
            // Shift+Tab is unreliable across terminals, Ctrl+T is not taken).
            let next = agent.thinking_level().next();
            app.set_thinking(agent, next);
            false
        }
        _ if app.keybindings.matches("app.palette", &key) => {
            // M16c: command palette (opencode prompt.palette).
            // M22: not while the trust review holds — palette commands bypass
            // submit_editor, so they must not open against unreviewed rules.
            if !app.palette_open && !app.picker_open {
                if app.trust_review_pending {
                    app.ui.push_notice(
                        "config review pending - type `trust` to accept or `trust no` to quit",
                    );
                } else {
                    app.palette_open = true;
                    app.picker_index = 0;
                    app.stash_draft();
                }
            }
            false
        }
        _ if app.keybindings.matches("app.model.cycleForward", &key)
            || app.keybindings.matches("app.model.cycleBackward", &key) =>
        {
            // M23: F2 cycles favorites forward (pi app.model.cycleForward over
            // scoped models; opencode model_cycle_recent family). Shift+F2 goes
            // backward. M27 audit: gated like Ctrl+P — cycling persists config
            // (model id, recents) and since M26 also mutates the thinking
            // level, so it must not act against unreviewed rules.
            if app.trust_review_pending {
                app.ui.push_notice(
                    "config review pending - type `trust` to accept or `trust no` to quit",
                );
                return false;
            }
            let dir = if app.keybindings.matches("app.model.cycleBackward", &key) {
                -1
            } else {
                1
            };
            app.cycle_favorite(agent, dir);
            false
        }
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
            // Alt+Backspace: kill word backward (opencode input_delete_word_backward).
            app.kill_word_back();
            app.sync_slash_state();
            false
        }
        KeyCode::Backspace => {
            app.push_undo();
            app.backspace_tracked();
            app.end_yank();
            false
        }
        _ if app.keybindings.matches("tui.editor.undo", &key) => {
            // Parity sweep: editor undo (pi undo-stack.ts).
            app.undo();
            false
        }
        // M17b: redo (opencode input_redo, ctrl+.) and line-wise ops (pi/opencode).
        _ if app.keybindings.matches("tui.editor.redo", &key) => {
            app.redo();
            false
        }
        // M196: jump-to-character, on pi's chords (`tui/src/keybindings.ts:106-113`
        // -- `ctrl+]` forward, `ctrl+alt+]` backward). Read from the reference
        // rather than invented, and both are free here; `ctrl+a` is NOT, it is
        // line-home in this editor.
        // **The ALT arm must come first.** Match arms are tried in order, and the
        // plain `ctrl+]` guard is also satisfied by `ctrl+alt+]` -- so with the
        // plain arm first, backward would never be reachable and the key would
        // silently do the other thing.
        KeyCode::Char(']')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.modifiers.contains(KeyModifiers::ALT) =>
        {
            app.toggle_jump(crate::app::JumpDir::Backward);
            false
        }
        KeyCode::Char(']') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.toggle_jump(crate::app::JumpDir::Forward);
            false
        }
        KeyCode::Char('a' | 'A') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = crate::editor_ops::line_home(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('e' | 'E') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = crate::editor_ops::line_end(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('k' | 'K') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let (_, killed, _) = crate::editor_ops::kill_to_line_end(&app.input, app.cursor_chars);
            let (s, c) = (app.input.clone(), app.cursor_chars);
            let kept: String = {
                let chars: Vec<char> = s.chars().collect();
                chars[..c]
                    .iter()
                    .chain(chars[c + killed.chars().count()..].iter())
                    .collect()
            };
            app.push_undo();
            let contiguous = app.last_kill_was_contiguous;
            app.kill_ring.push(killed, contiguous, false);
            app.last_kill_was_contiguous = true;
            app.input = kept;
            app.end_yank();
            false
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_chars = crate::editor_ops::word_left_atomic(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('b' | 'B') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // M36: cluster-stepped, like Left.
            app.cursor_chars = crate::editor_ops::prev_cluster_start(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('f' | 'F') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // M36: cluster-stepped, like Right.
            app.cursor_chars = crate::editor_ops::next_cluster_end(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('b' | 'B') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_chars = crate::editor_ops::word_left_atomic(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('f' | 'F') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_chars = crate::editor_ops::word_right_atomic(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Char('d' | 'D') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Forward-delete character under cursor (pi input.delete / opencode ctrl+d).
            if app.cursor_chars < app.input.chars().count() {
                let (s, _) = crate::editor_ops::delete_forward(&app.input, app.cursor_chars);
                app.push_undo();
                app.input = s;
                app.sync_slash_state();
            }
            false
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = crate::editor_ops::word_left(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_chars = crate::editor_ops::word_right(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Left => {
            // M36: one grapheme cluster per press — never park mid-cluster.
            app.cursor_chars = crate::editor_ops::prev_cluster_start(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Right => {
            // M36: one grapheme cluster per press.
            app.cursor_chars = crate::editor_ops::next_cluster_end(&app.input, app.cursor_chars);
            false
        }
        KeyCode::Up => {
            if app.at_open {
                let total = app.at_rows.as_ref().map(|r| r.len()).unwrap_or(0);
                app.at_index = app.at_index.saturating_sub(1).min(total.saturating_sub(1));
                return false;
            }
            // M17b: when the slash popup is open, arrows navigate IT (pi
            // prompt.autocomplete.prev) instead of history recall.
            if app.slash_open {
                let total = crate::picker::slash_items(&app.input).len();
                app.slash_index = app
                    .slash_index
                    .saturating_sub(1)
                    .min(total.saturating_sub(1));
                return false;
            }
            app.recall(true);
            false
        }
        KeyCode::Down => {
            if app.at_open {
                if let Some(rows) = &app.at_rows
                    && !rows.is_empty()
                {
                    app.at_index = (app.at_index + 1).min(rows.len() - 1);
                }
                return false;
            }
            if app.slash_open {
                let total = crate::picker::slash_items(&app.input).len();
                if total > 0 {
                    app.slash_index = (app.slash_index + 1).min(total - 1);
                }
                return false;
            }
            app.recall(false);
            false
        }
        // Transcript scrolling (M-unstaged): the chords are bindings rather than
        // `KeyCode::PageUp` arms so the footer can name the key that scrolls
        // without hardcoding the same literal a second time — and so a user who
        // prefers `ctrl+u`/`ctrl+d` can say so. The arm is a guard on purpose:
        // a rebound key falls through to the arms below instead of being
        // swallowed by a keycode this handler no longer owns.
        _ if app.keybindings.matches("app.transcript.pageUp", &key) => {
            app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(10);
            false
        }
        _ if app.keybindings.matches("app.transcript.pageDown", &key) => {
            app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(10);
            false
        }
        // M196: jump to the previous/next SEMANTIC prompt — a user turn, not a
        // line. pi matches OSC-133 prompt markers in the rendered rows
        // (`tui-alt-screen.ts:489`); Epsilon emits none, so the equivalent is
        // `LineKind::User`, which is the same idea expressed in the data the
        // transcript already has.
        //
        // **Why this is worth a key.** A long session is mostly tool results;
        // paging by ten rows stops on things nobody was reading, and the reader
        // re-pages four times to reach the turn they want. Jumping prompt to
        // prompt is the difference between one press and four.
        _ if app.keybindings.matches("app.transcript.prevPrompt", &key) => {
            match crate::app::jump_to_prompt(app.ui.lines(), app.scroll_from_bottom, -1) {
                Some(rows) => app.scroll_from_bottom = rows,
                None => app.ui.push_notice("no earlier prompt"),
            }
            false
        }
        // M196: half-page and the two ends.
        //
        // **The half page is the MEASURED viewport**, not a fixed ten rows --
        // that is what makes it a half page rather than "pageUp again". Zero
        // means the frame has not been painted yet, and guessing a number then
        // would scroll by an amount unrelated to any screen the user has seen.
        // M196: the copy key, bound to the command that already does the work.
        // Routing through `Command::Copy` rather than re-implementing the clipboard
        // call here is the point: the OSC-52 parking, the multiplexer choice, and
        // the notice that names which path landed are all in that one arm, and a
        // second copy of them is how a notice starts disagreeing with the copy.
        _ if app.keybindings.matches("app.message.copy", &key) => {
            // Routed through the command that already does the work, rather than
            // re-implementing the clipboard call: the OSC-52 parking, the
            // multiplexer choice, and the notice naming which path landed are all
            // in that one arm, and a second copy of them is how a notice starts
            // disagreeing with the copy.
            app.run_command(agent, crate::picker::Command::Copy)
        }
        _ if app.keybindings.matches("app.transcript.halfUp", &key) => {
            let half = (app.geometry.visible / 2).max(1);
            app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(half);
            false
        }
        _ if app.keybindings.matches("app.transcript.halfDown", &key) => {
            let half = (app.geometry.visible / 2).max(1);
            app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(half);
            false
        }
        // Top and bottom. `total` is the whole transcript, so the top is one
        // screen past the last row -- `saturating_sub` keeps it pinned there
        // rather than running the offset away into rows that do not exist.
        _ if app.keybindings.matches("app.transcript.top", &key) => {
            app.scroll_from_bottom = app.geometry.total.max(1);
            false
        }
        _ if app.keybindings.matches("app.transcript.bottom", &key) => {
            app.scroll_from_bottom = 0;
            false
        }
        _ if app.keybindings.matches("app.transcript.nextPrompt", &key) => {
            match crate::app::jump_to_prompt(app.ui.lines(), app.scroll_from_bottom, 1) {
                Some(rows) => app.scroll_from_bottom = rows,
                None => app.ui.push_notice("no later prompt"),
            }
            false
        }
        KeyCode::Tab => {
            if app.at_open {
                app.apply_at_selection(false);
                return false;
            }
            // M15/M17b: Tab applies the HIGHLIGHTED slash candidate (pi tui.input.tab).
            if app.slash_open {
                app.apply_slash_selection();
            }
            false
        }
        KeyCode::Home => {
            app.cursor_chars = 0;
            false
        }
        KeyCode::End => {
            app.cursor_chars = app.input.chars().count();
            false
        }
        KeyCode::Delete => {
            let (s, _) = crate::editor_ops::delete_forward(&app.input, app.cursor_chars);
            app.input = s;
            false
        }
        KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.kill_to_start();
            false
        }
        KeyCode::Char('w' | 'W') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.kill_word_back();
            app.sync_slash_state();
            false
        }
        KeyCode::Char('y' | 'Y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.yank();
            false
        }
        // Ctrl+V paste (M32c): when the terminal handles Ctrl+V as paste instead of
        // sending bracketed TermEvent::Paste (conhost, cmd.exe, some WSL setups),
        // read the system clipboard via arboard. Outside the auth prompt this
        // becomes an editor paste marker so huge pastes don't flood the buffer.
        _ if app.keybindings.matches("app.clipboard.paste", &key) => {
            let text = clipboard_text();
            if text.trim().is_empty() {
                app.ui
                    .push_notice("clipboard empty — copy text first, then Ctrl+V");
                return false;
            }
            // Reuse the bracketed-paste path shape: large/multi-line → marker,
            // single short → literal. That path lives in drive/TermEvent::Paste;
            // factor it here so ctrl+V and shift+insert behave identically without
            // duplicating the 3-line marker/payload bookkeeping.
            let marker = crate::kill_ring::make_paste_marker(app.next_paste_id, &text);
            if marker.starts_with(crate::kill_ring::MARKER_PREFIX) {
                app.paste_payloads.insert(app.next_paste_id, text.clone());
                app.next_paste_id += 1;
            }
            app.insert_str(&marker);
            false
        }
        // M16b: Ctrl+J inserts a newline (the reliable multi-line combo — Ctrl+Enter
        // is indistinguishable from Enter on legacy Windows consoles).
        _ if app.keybindings.matches("tui.input.newLine", &key) => {
            app.insert_char('\n');
            app.end_yank();
            false
        }
        // M59: hand the draft to $EDITOR (pi `app.editor.external`; the request
        // is performed by the event loop, which owns the terminal).
        _ if app.keybindings.matches("app.editor.external", &key) => {
            app.request_external_editor();
            false
        }
        // M13 card polish: expand/collapse truncated tool outputs. Originally a
        // bare `x` (M13), which made the letter x untypable in the editor —
        // M29 fix: it needs Ctrl+X like every other non-typing key.
        //
        // M126: the key now does both of the things a reader could mean by it, and
        // says which. The **global** toggle still means expand-all, so a user who
        // The **global** toggle still means expand-all, so a user who relies on it
        // keeps it; on top of that the newest card with a body gets its own state
        // recorded, which is what makes one card openable without opening every
        // card. Per-card state is the *exception* to the global toggle: a card the
        // user opened by hand stays open when the global toggle is later turned
        // off, and one they closed stays closed.
        //
        // The card's own state is read **before** the global is flipped. Read
        // after, `card_is_open` would already be answering from the new global
        // value, the key would "helpfully" record the opposite of what it just
        // showed, and pressing it would look like it did nothing at all.
        _ if app.keybindings.matches("app.tools.expand", &key) => {
            app.toggle_tool_expansion();
            false
        }
        // D11: jump to live — the exit from a frozen view.
        //
        // Guarded on `scroll_from_bottom`, which is the frozen state itself, so
        // this arm is unreachable from a live view and the chord keeps its letter
        // while the reader is typing. That guard is the whole reason a bare
        // letter is acceptable here: M13 learned it the expensive way, when a
        // bare `x` for "expand" made the letter x untypable in the editor.
        //
        // Zero means "follow the tail" rather than "one row above it" — the same
        // value the transcript's follow-scroll snaps to, so there is exactly one
        // representation of live and no way to land a row short of it.
        _ if app.scroll_from_bottom > 0 && app.keybindings.matches("app.transcript.live", &key) => {
            app.scroll_from_bottom = 0;
            false
        }
        KeyCode::Char(c) => {
            app.insert_char_tracked(c);
            false
        }
        // G10: a key the TUI does not act on says so.
        //
        // This arm used to be `_ => false` — a silent no-op. A user who presses a
        // chord they reasonably expect to work (`ctrl+]`, `alt+p`, `shift+tab`) saw a
        // screen that did not change and had no way to learn why, which is the shape
        // of the freeze this project was reported to have: **"nothing happened" must
        // never be a state the UI can be in.** The notice makes the outcome legible
        // without changing what the key does, and it names the key so the user can
        // tell "not bound" from "bound to something else".
        //
        // `Char` is handled above and is never reported: an ordinary letter always
        // inserts, so reaching here with one would mean a real bug, not a bad guess.
        other => {
            app.notice(format!(
                "{other} is not bound here — /hotkeys lists what is"
            ));
            false
        }
    }
}

/// Apply one terminal event; `true` means the run loop should quit.
///
/// Extracted from the loop so the pre-draw drain and the select's input arm
/// share exactly one implementation — the drain is what keeps a burst of keys
/// down to a single frame.
pub(crate) fn handle_term_event(app: &mut App, agent: &Arc<Agent>, ev: TermEvent) -> bool {
    match ev {
        TermEvent::Key(key) => {
            if key.kind == KeyEventKind::Press && handle_key(app, agent, key) {
                return true;
            }
        }
        // Mouse wheel: Up/Down/Left/Right all forwarded by most terminals even
        // without EnableMouseCapture. Swallowing Down/Up/Press/Release is what
        // blocks drag-select.
        TermEvent::Mouse(m) => match m.kind {
            crossterm::event::MouseEventKind::ScrollUp => {
                if app.picker_open {
                    app.picker_index = app.picker_index.saturating_sub(1);
                } else {
                    app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(3);
                }
            }
            crossterm::event::MouseEventKind::ScrollDown => {
                if app.picker_open {
                    let total = app.picker_rows().len();
                    if total > 0 {
                        app.picker_index = (app.picker_index + 1).min(total - 1);
                    }
                } else {
                    app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(3);
                }
            }
            _ => {} // drag/select: let the terminal own it
        },
        // M16b: bracketed pastes become atomic `[paste #N]` markers.
        TermEvent::Paste(text) => {
            if app.auth_prompt_open {
                // Masked API-key paste: go straight into auth_key, not the editor.
                handle_auth_paste(app, text);
            } else if app.auth_pick_open {
                // Typing pastes into the filter — useful when pasting a provider id.
                for c in text.trim().chars() {
                    app.insert_char(c);
                }
            } else {
                let marker = crate::kill_ring::make_paste_marker(app.next_paste_id, &text);
                if marker.starts_with(crate::kill_ring::MARKER_PREFIX) {
                    app.paste_payloads.insert(app.next_paste_id, text.clone());
                    app.next_paste_id += 1;
                }
                app.insert_str(&marker);
            }
        }
        _ => {}
    }
    false
}
