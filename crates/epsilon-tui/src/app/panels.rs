//! The panels the TUI draws on top of the transcript.
//!
//! ## Why this file exists
//!
//! `app.rs` was 4,526 lines and mixed four separable concerns; this is the first seam cut, and
//! it is the cleanest one: every function here is a *panel* — a popup, an overlay, or the
//! transcript itself — with the same shape (`f`, the `App`, and a rectangle) and no
//! orchestration of its own. `draw()` composes them; none of them calls `draw()`.
//!
//! They were moved **verbatim**. Nothing was renamed, reworded or refactored in the pass, because
//! a refactor that also moves code cannot tell you which of the two broke the build. The only
//! edits the move required are the visibility keywords: these were private to `app`, and the
//! parent module still calls every one of them, so each became `pub(super)`.
//!
//! `composer_rows` deliberately stayed behind in `app.rs`. It is a layout helper rather than a
//! panel, and leaving it there keeps `app/composer_viewport_tests.rs` — which reaches it through
//! `use super::*` — completely untouched, which is the cheapest possible proof that the move
//! changed nothing.

use super::*;

/// Text a row may draw, stripped of anything that steers the display.
///
/// # Why these panels strip rather than refuse
///
/// The permission dialog **refuses** to be answered when a request carries display-steering
/// characters (M261), because a reader there is making a decision and a cleaned line still
/// means agreeing to something they did not fully see.
///
/// Every panel that calls this is the other case: **display only**. The `@` popup shows a path
/// and inserting it opens that path; the session browser shows a title and resuming uses the
/// path beside it; the model picker shows a label and the applied model comes from the row's
/// own field. None of them decides anything from the string it draws, so removing the override
/// is the whole correct response — refusing would break a legitimate row over bytes that were
/// never going to be executed.
///
/// # Why the count is not reported
///
/// It is, everywhere it matters: `TranscriptLine::hidden` carries it, and a card's strip is
/// folded into it. **An overlay has nowhere to put it** — no field, no footer, no line — and
/// inventing one for a value that cannot cause harm is the kind of noise this repository keeps
/// arguing against. The honest statement is that this is a limitation, not a design.
fn safe_to_show(text: &str) -> String {
    let (safe, _) = crate::text_safety::display_safe(text);
    safe.into_owned()
}

/// `/models` overlay: filter-driven list with provider grouping hints and an auth
/// column. The active model floats to the top (picker::build_rows).
pub(super) fn draw_picker(f: &mut Frame<'_>, app: &mut App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    // `●` is not in CP437, so the "you are here" mark is a glyph rather than a literal: on a legacy
    // console the ASCII repertoire draws `*` (M219/A22).
    let active_marker = app.theme.glyphs.dot;

    // M16c two-stage: stage 1 lists providers; stage 2 that provider's models.
    if let Some(provider) = &app.picker_provider.clone() {
        draw_picker_stage2(f, app, outer, provider);
        return;
    }
    let rows = crate::picker::build_provider_rows(&app.model_id, &app.customs)
        .into_iter()
        .map(|p| (p.id, p.model_count, p.auth_ready, p.is_active))
        .collect::<Vec<_>>();
    let width = Width::PANEL.resolve(outer);
    let height = 15u16.min(outer.height.saturating_sub(2));
    // **Remember the measured window.** A Page key arrives in the key handler,
    // which knows nothing about geometry -- and pi's page size is `maxVisible`
    // (`session-selector.ts:612-617`), the same measured window rather than a
    // constant. A hardcoded ten would be a whole screen on a tall terminal and a
    // fifth of one on a short one, and the key would jump by an amount the user
    // cannot predict from what they can see.
    //
    // Set from the PANEL height rather than the inner one, so it holds for
    // `draw_picker_stage2` too -- that function takes `&App` and cannot write
    // here, and deriving the same number in both places is how two lists end up
    // paging by different amounts.
    app.picker_capacity = height.saturating_sub(3).max(1) as usize;
    let title = format!("models {} filter: '{}'", app.theme.glyphs.dash, app.input);
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned rather than centred, like the palette: the picker opens
        // where the composer is and stays put while the filter changes.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: &title,
        focused: true,
        border: None,
    };
    let inner = panel.open(f);

    // M15 viewport fix: pi's centered scroll window — render only the visible slice
    // so long lists stay navigable (previously rows past the box were unreachable).
    let capacity = inner.height.saturating_sub(2).max(1) as usize; // blank line + hint footer
    // **Remember the measured capacity.** A Page key arrives in the key handler,
    // which knows nothing about geometry -- and pi's page size is `maxVisible`
    // (`session-selector.ts:612-617`), the same measured window, not a constant.
    // A hardcoded ten would be a whole screen on a tall terminal and a fifth of one
    // on a short one, and the key would jump by an amount the user cannot predict
    // from what they see. So the paint is what teaches the handler how far a page
    // is, and the handler pages by exactly that.
    let start = crate::scroll::scroll_start(rows.len(), app.picker_index, capacity);
    let end = (start + capacity).min(rows.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled("no providers", app.theme.dim())));
    }
    for (i, (id, count, auth_ready, is_active)) in
        rows.iter().enumerate().skip(start).take(end - start)
    {
        let selected = i == app.picker_index;
        // The active-provider mark and the auth badge are leading columns of the
        // label; `list_row` owns the selection bar and right-aligns the count.
        let marker = if *is_active { active_marker } else { "  " };
        let auth = if *auth_ready {
            app.theme.glyphs.key
        } else {
            "· "
        };
        lines.push(overlay::list_row(
            &app.theme,
            selected,
            &format!("{marker}{auth} {id}"),
            Some(format!("{count} models")).as_deref(),
            inner.width as usize,
        ));
    }

    lines.push(Line::from(""));
    let arrows = overlay::up_down(app.theme.glyphs.up, app.theme.glyphs.down);
    lines.push(overlay::hint_row(
        &app.theme,
        &[
            (&arrows, "move"),
            ("enter", "browse models"),
            ("esc", "close"),
        ],
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

/// Stage-2 of the two-stage picker: one provider's models with fuzzy filter.
/// M57: the `/sessions` overlay. Rows are session files from the workspace's
/// `.epsilon/sessions`, newest first; the current one is marked `●`.
pub(super) fn draw_session_browser(f: &mut Frame<'_>, app: &mut App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    // The "you are here" mark is a glyph: `●` is not in CP437 (M219/A22).
    let active_marker = app.theme.glyphs.dot;

    // Take the mutable handle FIRST, so the capacity can be recorded before the
    // immutable borrow below. Doing it after would ask for `&mut app` while
    // `browser` is still alive, which the borrow checker refuses -- rightly.
    let Some(browser) = app.session_browser.as_mut() else {
        return;
    };
    let width = Width::PANEL.resolve(outer);
    let height = 15u16.min(outer.height.saturating_sub(2));
    // P-230: the title names the scope Tab switches, so the key has a visible
    // state — a toggle whose position is invisible is a mode error waiting.
    let scope_word = match browser.scope {
        epsilon_session::SessionScope::Workspace => "this workspace",
        epsilon_session::SessionScope::All => "all workspaces",
    };
    let title = format!(
        "sessions \u{2014} {scope_word} \u{b7} filter '{}'",
        app.input
    );
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned like the palette and the picker: the browser opens at the
        // composer's edge and does not jump as the filter narrows the list.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: &title,
        focused: true,
        border: None,
    };
    let inner = panel.open(f);

    // Same centered scroll window as the pickers: only the visible slice is built.
    let capacity = inner.height.saturating_sub(1).max(1) as usize; // footer
    // The same measured window as the model picker, for the same reason: a Page
    // key in the handler knows no geometry, and pi's page size is the visible
    // count (`session-selector.ts:612-617`).
    browser.capacity = capacity;
    let start = crate::scroll::scroll_start(browser.rows.len(), browser.index, capacity);
    let end = (start + capacity).min(browser.rows.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    if browser.rows.is_empty() {
        // Empty directory vs filtered-out (P-243): an empty filter with no
        // rows means no sessions exist yet, so the message names the way to
        // make one; a non-empty filter means the filter hides them. The scope
        // rides along (P-230): "in this workspace" when the reader may only
        // need to press Tab.
        let empty = if app.input.trim().is_empty() {
            match browser.scope {
                epsilon_session::SessionScope::Workspace => {
                    "No sessions in this workspace yet \u{2014} Ctrl+N starts one, Tab sees all, Esc closes"
                }
                epsilon_session::SessionScope::All => {
                    "No sessions anywhere yet \u{2014} Ctrl+N starts one, Esc closes"
                }
            }
        } else {
            "no matches \u{2014} Backspace to widen, Esc closes"
        };
        lines.push(Line::from(Span::styled(empty, app.theme.dim())));
    }
    for (i, row) in browser
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        let selected = i == browser.index;
        // The current-session mark leads the title column; the age/size meta
        // stays the right-aligned column it was.
        let marker = if row.is_current { active_marker } else { "  " };
        lines.push(overlay::list_row(
            &app.theme,
            selected,
            // A session's title is a **user's own words** — the first message of that
            // conversation — read from a file on disk, so a session shared or imported from
            // elsewhere can put anything in it. See `safe_to_show`.
            &safe_to_show(&format!("{marker}{}", row.title)),
            Some(safe_to_show(row.meta.as_str()).as_str()),
            inner.width as usize,
        ));
    }
    // The position counter and the key hints share the footer line: a `hint_row`
    // carries the hints, and the counter stays where the reader looks for it.
    let mut footer = vec![Span::styled(
        format!("{} of {}", browser.index + 1, browser.rows.len()),
        app.theme.dim(),
    )];
    // P-230: the footer names the scope Tab moves TO, not the one showing —
    // the toggle's destination is what the reader needs before pressing.
    let other_scope = match browser.scope {
        epsilon_session::SessionScope::Workspace => "all sessions",
        epsilon_session::SessionScope::All => "this workspace",
    };
    footer.push(Span::styled(" \u{b7} ", app.theme.faint()));
    footer.extend(
        overlay::hint_row(
            &app.theme,
            &[
                ("enter", "resumes"),
                ("tab", other_scope),
                // The bound key, named so a reader can learn it exists — §2.7.
                // Resolved from the table rather than typed, so a rebind moves
                // the hint with it.
                ("ctrl+d", "deletes"),
                ("esc", "closes"),
            ],
        )
        .spans,
    );
    lines.push(Line::from(footer));

    // P-237: the confirmation replaces the list, drawn in the danger colour, and
    // names the session being deleted. Pi draws `Delete session? confirm · cancel`
    // in error colour (`session-selector.ts:158-161`); this also repeats the
    // **title**, because a reader should decide against the text they just read
    // rather than against a recollection of which row was highlighted — the same
    // rule `/restore`'s two-press confirmation follows.
    if let Some(pending) = browser.confirming_delete.as_deref() {
        let title = browser
            .rows
            .iter()
            .find(|r| r.path == pending)
            .map(|r| safe_to_show(&r.title))
            .unwrap_or_else(|| safe_to_show(&pending.display().to_string()));
        let confirm = overlay::hint_row_in(
            &app.theme,
            &[("enter", "deletes"), ("any other key", "cancels")],
            inner.width as usize,
        );
        lines = vec![
            Line::from(Span::styled(
                format!("Delete session? {title}"),
                app.theme.danger(),
            )),
            Line::from(confirm.spans),
        ];
    }
    // And a refusal, in the same place for the same reason: the transcript a
    // notice would land in is behind this overlay.
    if let Some(why) = browser.delete_refusal.as_deref() {
        let dismiss = overlay::hint_row_in(
            &app.theme,
            &[("any key", "dismisses")],
            inner.width as usize,
        );
        lines = vec![
            Line::from(Span::styled(safe_to_show(why), app.theme.danger())),
            Line::from(dismiss.spans),
        ];
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_picker_stage2(f: &mut Frame<'_>, app: &App, outer: Rect, provider: &str) {
    use crate::ui::overlay::{self, Panel, Width};

    // The "you are here" mark, the favourite star and the hint arrows are all glyphs (M219/A22):
    // `●` and `★` are not in CP437, and a legacy console drew both as boxes.
    let active_marker = app.theme.glyphs.dot;
    let rows = crate::picker::build_models_for_provider(
        provider,
        &app.input,
        &app.model_id,
        // No favorites in the drilldown (preserves current rows; the
        // starred section lives in the flat picker). The default rides
        // along: second place is about the model list, not the picker.
        &[],
        app.default_model.as_deref(),
        &app.live_cache,
        &app.customs,
    );
    let width = Width::PANEL.resolve(outer);
    let height = 15u16.min(outer.height.saturating_sub(2));
    let title = format!(
        "models {} {provider}: filter '{}'",
        app.theme.glyphs.dash, app.input
    );
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned like stage 1, so stepping between the two stages does not
        // move the frame under the reader.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: &title,
        focused: true,
        border: None,
    };
    let inner = panel.open(f);

    let capacity = inner.height.saturating_sub(2).max(1) as usize; // blank line + hint footer
    // **Remember the measured capacity.** A Page key arrives in the key handler,
    // which knows nothing about geometry -- and pi's page size is `maxVisible`
    // (`session-selector.ts:612-617`), the same measured window, not a constant.
    // A hardcoded ten would be a whole screen on a tall terminal and a fifth of one
    // on a short one, and the key would jump by an amount the user cannot predict
    // from what they see. So the paint is what teaches the handler how far a page
    // is, and the handler pages by exactly that.
    let start = crate::scroll::scroll_start(rows.len(), app.picker_index, capacity);
    let end = (start + capacity).min(rows.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches — Backspace to widen, Esc closes",
            app.theme.dim(),
        )));
    }
    for (i, row) in rows.iter().enumerate().skip(start).take(end - start) {
        let selected = i == app.picker_index;
        // The active-model mark, the auth badge and the favorite star are leading
        // columns of the label; the model id stays the right-aligned meta column.
        let marker = if row.id == app.model_id {
            active_marker
        } else {
            "  "
        };
        let auth = if row.auth_ready {
            app.theme.glyphs.key
        } else {
            "· "
        };
        // M23: ★ marks favorites (opencode's star column); Ctrl+F toggles.
        let star = if row.is_favorite {
            app.theme.glyphs.star
        } else {
            " "
        };
        // P-249: the saved default is marked, like pi's `· default`.
        //
        // P-260: a configured id the catalog does not know keeps its row, gets
        // `[unavailable]` as a word and is struck through. The badge is a word
        // rather than another glyph because `AGENTS.md` §8 forbids meaning by
        // colour or shape alone, and `Modifier::CROSSED_OUT` is the terminal's
        // standard "exists, not usable" mark — the same one Pi reaches for
        // (`scoped-models-selector.ts:255`).
        //
        // The row is left dim as well: the strike is invisible on a terminal
        // that ignores SGR 9 and the dim is invisible on one that ignores
        // colour, so between them at least one channel always carries the fact.
        let label = if row.is_unavailable {
            format!("{} [unavailable]", row.label)
        } else if row.is_default {
            format!("{} · default", row.label)
        } else {
            row.label.clone()
        };
        lines.push(overlay::list_row_marked(
            &app.theme,
            selected,
            row.is_unavailable
                .then_some(ratatui::style::Modifier::CROSSED_OUT),
            // A **live** model's label and id come from a third-party endpoint, so they are
            // remote text in a row the reader chooses by reading. See `safe_to_show`.
            &safe_to_show(&format!("{marker}{auth} {star} {}", label)),
            Some(safe_to_show(row.id.as_str()).as_str()),
            inner.width as usize,
        ));
    }

    lines.push(Line::from(""));
    // P-260: the count of unavailable rows, like Pi's
    // `${enabledCount}/${allIds.length} enabled${unavailableCount ? ` · ${unavailableCount} unavailable` : ""}`
    // (`scoped-models-selector.ts:196-201`). Shown only when there are any: a
    // count of zero is noise on every ordinary stage-2 screen.
    let unavailable = rows.iter().filter(|r| r.is_unavailable).count();
    let count = format!("{unavailable} unavailable");
    let mut hints = vec![("type", "filters (fuzzy)"), ("enter", "apply")];
    if unavailable > 0 {
        hints.push(("", count.as_str()));
    }
    hints.push(("ctrl+s", "save default"));
    hints.push(("backspace/esc", "back"));
    lines.push(overlay::hint_row(&app.theme, &hints));
    f.render_widget(Paragraph::new(lines), inner);
}

/// M16a virtualized draw: reconcile the render cache (renders ONLY unseen lines),
/// then materialize just the visible window. Per-frame cost is O(visible), not
/// O(transcript).
///
/// The last step is decoration: each visible row comes out of the cache as a bare
/// body, and [`crate::ui::transcript::decorate`] adds the two-column gutter and,
/// on a truecolor terminal, the band that reaches the right edge. That order is
/// deliberate — decoration sits OUTSIDE the cache, so switching colour mode (or
/// losing `NO_COLOR`) never invalidates a rendered row and the cache key stays
/// `(width, render state)`.
///
/// M126 adds the second half of that key: **which cards are open**. It is part of
/// the key rather than a render-time lookup because a card's rows depend on it —
/// an open card is taller, it is wrapped differently, and serving the collapsed
/// rows under an open state is the one bug the key exists to prevent. The state is
/// cheap to compare (a bool, a short list and one string) and changes only when
/// the user presses a key.
/// The measurement already happened in [`App::measure_transcript`]; this function
/// renders a [`TranscriptGeometry`] rather than deriving it a second time, so the
/// rows the reader sees are the rows the footer counted. A zero-height geometry
/// is not a small viewport — it means "not measured", and the caller below the
/// layout floor legitimately has none.
pub(super) fn draw_transcript(
    f: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    geometry: TranscriptGeometry,
) {
    if geometry.visible == 0 {
        // No viewport, no rows. Drawing with a zero-height window would append the
        // whole streaming tail and hand ratatui more rows than the area holds.
        return;
    }
    let frame_width = area.width as usize;
    let body_width = frame_width.saturating_sub(crate::ui::transcript::GUTTER);

    // Keep thinking/prose and explicit newlines separate; slice the combined
    // history + live tail in screen rows, including tails taller than the viewport.
    // The tail is not a transcript line yet, so its kind is decided here: it is
    // the assistant's own prose (or its reasoning) arriving token by token.
    let mut tail_rows: Vec<(LineKind, bool, Line<'static>)> = Vec::new();
    // P-040: the live thinking tail obeys the hide flag like committed
    // Thinking lines do — a hidden transcript that still streams reasoning
    // is not hidden.
    let thinking_tail = if app.ui.hide_thinking {
        ""
    } else {
        app.ui.streaming_thinking()
    };
    // Round 72's rule, one level in. The two tails are built in one pass, and
    // that made reasoning and the answer **adjacent** for the whole of the
    // text-streaming phase: the frame then carries a `│` gutter row directly
    // above an answer row, and the transition a reader is watching for — the
    // model stops reasoning and starts answering — has no marker at all. On
    // commit `push_line`'s kind-change rule separates them, so the frame also
    // changed shape mid-turn. Same rule, applied at draw time inside the tail:
    // the first row of a second tail kind gets a blank above it.
    let mut tail_kinds_seen = false;
    let mut last_tail_kind = None;
    for (text, style, kind) in [
        (thinking_tail, app.theme.thinking(), LineKind::Thinking),
        (
            app.ui.streaming_text(),
            app.theme.assistant(),
            LineKind::Assistant,
        ),
    ] {
        if !text.is_empty() {
            // The separator goes above the first row of this kind, so it counts
            // as a tail row for the skip arithmetic below — exactly like the
            // separator round 72 put above the whole tail.
            if tail_kinds_seen && last_tail_kind != Some(kind) {
                tail_rows.push((LineKind::Blank, false, Line::from(Vec::new())));
            }
            tail_kinds_seen = true;
            last_tail_kind = Some(kind);
            for line in text.split('\n') {
                // The same continuation rule the cached rows get: the tail is
                // not in the cache, so it has no `line_of_row` answer and has
                // to count its own wrapped rows. Without it a streaming answer
                // drew its role glyph on every row it wrapped to.
                let wrapped = crate::wrap::wrap_line(&[(line, style)], body_width);
                for (at, row) in wrapped.into_iter().enumerate() {
                    tail_rows.push((
                        kind,
                        at != 0,
                        Line::from(
                            row.into_iter()
                                .map(|(text, style)| Span::styled(text, style))
                                .collect::<Vec<_>>(),
                        ),
                    ));
                }
            }
        }
    }
    // `total` here is the CACHED row count, which is what the tail rows are
    // skipped against: the tail is not in the cache, so it starts at that offset.
    let total = app.render_cache.total_rows();
    let skip = geometry.skip;
    let visible = geometry.visible;
    // **A wrapped row is a continuation, and it says so in the gutter.**
    //
    // Every row of a wrapped paragraph used to carry the paragraph's own glyph,
    // so one assistant answer drew as three `▪` rows — a bulleted list nobody
    // wrote. The role's own glyph now belongs to the line's FIRST row and the
    // continuation rule (`glyphs.vertical`, the same one a card's body uses) to
    // every row after it. The flag travels beside the kind rather than replacing
    // it, because a wrapped user turn keeps its BAND: `decorate` reads the band
    // from the kind and the glyph from this.
    //
    // `line_of_row` is the row→line map the search highlight already walks, so
    // the two cannot disagree about which rows are one line.
    let mut rows: Vec<(LineKind, bool, Line<'static>)> = app
        .render_cache
        .visible_rows_with_kinds(skip, visible)
        .into_iter()
        .enumerate()
        .map(|(i, (kind, line))| {
            let continued = i > 0
                && app.render_cache.line_of_row(skip + i)
                    == app.render_cache.line_of_row(skip + i - 1);
            (kind, continued, line)
        })
        .collect();
    // The live tail is not a transcript line, so `push_line`'s separation
    // rule never sees it — without this it glues to the previous block
    // while streaming and only separates when committed. Same rule, at draw
    // time: blank before live text when the last visible row isn't blank.
    // Inside `tail_rows` (not the cache), so scroll math below counts it
    // like any other tail row.
    if !tail_rows.is_empty()
        && rows
            .last()
            .is_some_and(|(kind, _, _)| *kind != crate::state::LineKind::Blank)
    {
        tail_rows.insert(
            0,
            (crate::state::LineKind::Blank, false, Line::from(Vec::new())),
        );
    }
    let remaining = visible.saturating_sub(rows.len());
    rows.extend(
        tail_rows
            .into_iter()
            .skip(skip.saturating_sub(total))
            .take(remaining),
    );

    // The conversation's live edge sits next to the composer.
    //
    // A `Paragraph` lays its rows out from the top of its region, so until now a
    // conversation shorter than the viewport drew under the header and left
    // every remaining body row blank: at 80x24 a user turn, a separator and one
    // notice drew on rows 1-3 of a twenty-row body, leaving seventeen blank rows
    // between the newest text and the composer, and while an answer streamed in
    // it arrived as far from the input as the screen allows.
    //
    // So the spare rows are moved *above* the content rather than left below it.
    // They are `LineKind::Blank` for a reason and not decoration: a blank row is
    // exactly what the separation rules above already insert between blocks, so
    // the padding draws the same pixels the transcript already uses for "no
    // conversation here".
    //
    // `Paragraph::scroll` is not the shortcut it looks like. It skips *forward*
    // through the composed lines — `for _ in 0..self.scroll.y` — so it shifts
    // content up out of the top of the region, which is top-anchoring with a
    // different window, and its offset is unsigned so nothing can ask it for the
    // other direction (`ratatui-widgets-0.3.2/src/paragraph.rs`). Padding rows
    // is the only way to say "there is no conversation here" in the space above.
    //
    // **Only while the view is following its tail.** `transcript_skip` returns 0
    // exactly when the content fits, so a window the reader scrolled back always
    // has more rows than fit and this pads nothing — the frozen window keeps
    // reading from the top, which is the one direction a history scroll has.
    // Padding is therefore computed from what survived the window, not from a
    // separate test of the scroll state.
    let spare = visible.saturating_sub(rows.len());

    // **The search highlight runs here, on the drawn rows, and not in the cache.**
    // The cache is keyed by width and render state and deliberately carries no
    // theme (see `CacheKey`), and a search is more transient than either: the
    // same rows are queried, re-queried and cleared inside a second. Highlighting
    // at draw time also means the mark lands on the text *as wrapped*, which is
    // the text the reader is reading, rather than on the logical line's source.
    //
    // **Before the padding splice below**, and that ordering is the point: at this
    // moment row `i` of `rows` is screen row `skip + i` exactly, which is the
    // coordinate `line_of_row` speaks. Once the live-edge blanks are spliced in
    // at the front the two no longer agree, and a highlight would land on
    // whatever the padding happened to push it onto.
    if let Some(s) = app.search.as_ref().filter(|s| !s.query.is_empty()) {
        for (i, (_kind, _continued, line)) in rows.iter_mut().enumerate() {
            // Every screen row of a matching line is a hit, so a hit that wrapped
            // is marked on both of its rows rather than half of it.
            let row_line = app.render_cache.line_of_row(skip + i);
            let is_hit = row_line.is_some_and(|l| s.matches.binary_search(&l).is_ok());
            let is_current = row_line.is_some_and(|l| s.matches.get(s.current) == Some(&l));
            if is_hit || is_current {
                let taken = std::mem::take(&mut line.spans);
                line.spans = crate::ui::transcript::highlight_matches(
                    taken,
                    &s.query,
                    if is_current {
                        app.theme.search_strong()
                    } else {
                        app.theme.search()
                    },
                    is_current,
                );
            }
        }
    }

    if spare > 0 {
        let blank = (crate::state::LineKind::Blank, false, Line::from(Vec::new()));
        rows.splice(0..0, std::iter::repeat_n(blank, spare));
    }

    // visible_rows already applied the viewport offset; decoration runs on the
    // rows that survived, and pads to the FRAME width, not the body width, so a
    // banded row reaches the terminal edge.
    let rows: Vec<Line<'static>> = rows
        .into_iter()
        .map(|(kind, continued, line)| {
            let body: Vec<Span<'static>> = line.spans;
            crate::ui::transcript::decorate(&app.theme, kind, continued, body, frame_width)
        })
        .collect();

    let para = Paragraph::new(rows);
    f.render_widget(para, area);
}

// The tests live beside this file: `app/tests.rs` and
// `app/composer_viewport_tests.rs`, pulled in below. They stay part of this
// module so `super::*` still reaches private items.
/// Command palette overlay (M16c, opencode command-palette pattern): fuzzy-filtered
/// list of all commands with keybinding hints.
pub(super) fn draw_palette(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    let entries = crate::picker::filter_palette(&app.input, &app.keybindings);
    let width = Width::Fixed(64).resolve(outer);
    let height = 14u16.min(outer.height.saturating_sub(2)).max(4);
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned rather than centred: the palette opens where the eye
        // already is (the composer), and a centred panel would jump as the
        // filter list changes height.
        area: Rect {
            y: outer.y + 2,
            ..overlay::centered(outer, width, height)
        },
        title: "commands",
        focused: true,
        border: None,
    };
    // Clear the panel's row band across the full width, not just the box: at 60
    // columns the palette is nearly the whole frame, and the transcript row that
    // happens to sit on the box's bottom-rail row showed through the margin —
    // `▎╰────╯` read as a broken rail. The tree overlay already clears before it
    // draws for the same reason; a modal that owns the keys must not float over
    // a sliver of the page it is covering.
    let band = Rect {
        x: outer.x,
        y: panel.area.y,
        width: outer.width,
        height: panel.area.height,
    };
    f.render_widget(ratatui::widgets::Clear, band);
    let inner = panel.open(f);

    let capacity = inner.height.saturating_sub(2).max(1) as usize;
    let start = crate::scroll::scroll_start(entries.len(), app.picker_index, capacity);
    let mut lines: Vec<Line<'static>> = Vec::new();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matching commands",
            app.theme.dim(),
        )));
    }
    for (index, entry) in entries.iter().enumerate().skip(start).take(capacity) {
        // `as_deref()` on a borrow, because the entry is behind `&` from `entries.iter()`
        // and the hint is now an owned `String` rather than a `&'static str`.
        let binding = entry.binding.as_deref().map(|b| format!("[{b}]"));
        lines.push(overlay::list_row(
            &app.theme,
            index == app.picker_index,
            &entry.title,
            binding.as_deref(),
            inner.width as usize,
        ));
    }
    // **The filter stays on the composer's line, and that is a correction (M312).** The comment
    // here used to read "The filter itself lives in the title bar, so the body stays a pure
    // list" — and the title is `"commands"`, with no filter in it. What the frame showed was the
    // composer row *and* the box's top border sharing one line, with the filter text reading as
    // part of the border. **The claim was true of the design and false of the code**, which is
    // the shape of a comment that has drifted. The filter is what the reader typed and where they
    // expect to see it; the box keeps a pure list.
    lines.push(Line::from(""));
    let arrows = overlay::up_down(app.theme.glyphs.up, app.theme.glyphs.down);
    lines.push(overlay::hint_row(
        &app.theme,
        &[
            ("type", "filter"),
            (&arrows, "move"),
            ("enter", "run"),
            ("esc", "close"),
        ],
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

// (M17b: slash registry moved to `picker::SLASH_ITEMS`/`picker::slash_items` —
// descriptions + selection live there now.)

/// Popup listing matching slash commands directly above the input box
/// (M17b: pi SelectList rendering — highlighted selection + dim descriptions,
/// arrow-key navigable via `slash_index`).
pub(super) fn draw_at_file(f: &mut Frame<'_>, app: &App, transcript_area: ratatui::layout::Rect) {
    if !app.at_open {
        return;
    }
    let rows = app.at_rows.as_deref().unwrap_or(&[]);
    if rows.is_empty() {
        // **The empty state, drawn rather than skipped.** This used to return
        // here, so an unmatched `@fragment` closed the popup and the screen said
        // nothing: the reader typed a path, saw nothing change, and had no way
        // to tell "no such file" from "the popup is broken". The palette names
        // its own empty row (`no matching commands`), and this is that row.
        let height = 4u16.min(transcript_area.height);
        if height == 0 {
            return;
        }
        let y = transcript_area
            .bottom()
            .saturating_sub(height)
            .saturating_sub(1);
        let area = ratatui::layout::Rect {
            x: transcript_area.x,
            y,
            width: transcript_area.width.min(64),
            height,
        };
        let panel = crate::ui::overlay::Panel {
            theme: &app.theme,
            area,
            title: "files",
            focused: true,
            border: None,
        };
        let inner = panel.open(f);
        let mut lines: Vec<ratatui::text::Line<'static>> = vec![
            ratatui::text::Line::from(ratatui::text::Span::styled(
                format!("no matching files for '{}'", app.at_fragment),
                app.theme.dim(),
            )),
            ratatui::text::Line::from(""),
            crate::ui::overlay::hint_row(&app.theme, &[("backspace", "widen"), ("esc", "close")]),
        ];
        // Pad so the frame's bottom rule is drawn at the same row every time:
        // a popup that changes height as the filter changes is a layout that
        // moves under the reader's hands.
        while lines.len() < inner.height as usize {
            lines.push(ratatui::text::Line::from(""));
        }
        f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
        return;
    }
    let capacity = 8usize;
    let start = crate::scroll::scroll_start(rows.len(), app.at_index, capacity);
    let end = (start + capacity).min(rows.len());
    let height = (end - start) as u16 + 1;
    let y = transcript_area
        .bottom()
        .saturating_sub(height)
        .saturating_sub(1);
    let area = ratatui::layout::Rect {
        x: transcript_area.x,
        y,
        width: transcript_area.width.min(64),
        height,
    };
    let panel = crate::ui::overlay::Panel {
        theme: &app.theme,
        area,
        title: "files",
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    let theme = &app.theme;
    let mut lines: Vec<ratatui::text::Line<'static>> = rows[start..end]
        .iter()
        .enumerate()
        .map(|(index, path)| {
            // Trim by characters, never by bytes: a path with one non-ASCII
            // component would otherwise panic on a boundary that happens to land
            // inside a multi-byte character.
            let shown = crate::ui::format::format_path(
                std::path::Path::new(path),
                None,
                inner.width.saturating_sub(6) as usize,
                theme.glyphs.ellipsis,
            );
            crate::ui::overlay::list_row(
                theme,
                start + index == app.at_index,
                // A workspace path can carry a display-steering character, and this is the
                // one place a reader chooses a file by **reading it**. See `safe_to_show`.
                &safe_to_show(&shown),
                None,
                inner.width as usize,
            )
        })
        .collect();
    lines.push(crate::ui::overlay::hint_row(
        theme,
        &[
            ("tab", "insert"),
            ("enter", "insert + space"),
            ("esc", "close"),
        ],
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}

pub(super) fn draw_timeline(f: &mut Frame<'_>, app: &mut App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    if !app.timeline_open || app.timeline_entries.is_empty() {
        return;
    }
    let total = app.timeline_entries.len();
    let capacity = 12usize.min(outer.height.saturating_sub(6).max(1) as usize);
    // P-306: the page step is this measured screenful. Recorded before the
    // immutable borrows below, like the session browser's own capacity.
    app.timeline_capacity = capacity;
    let start = crate::scroll::scroll_start(total, app.timeline_index, capacity);
    let end = (start + capacity).min(total);
    // The timeline is a list panel like the others, so it takes the panel width;
    // the height still follows the visible row count.
    let width = Width::PANEL.resolve(outer);
    // +6 rather than +4: the border takes two rows, and the search field plus
    // its match count take two more when a search is active. Sizing for the
    // worst case keeps the panel from resizing under the reader's fingers.
    let height = (capacity as u16 + 6).min(outer.height.saturating_sub(2));
    // The active filter is named in the title, because a picker that silently
    // hides entries is the "silent no-op" this repo treats as worse than a
    // missing feature. `f` cycles it.
    // P-074: fork mode names itself — the same list forks instead of jumping.
    let title = if app.timeline_fork_refill.is_some() {
        "fork · pick a message".to_string()
    } else {
        format!("timeline \u{00b7} {}", app.tree_filter.label())
    };
    let panel = Panel {
        theme: &app.theme,
        area: overlay::centered(outer, width, height),
        title: &title,
        focused: true,
        border: None,
    };
    let inner = panel.open(f);

    // A row is truncated to what the frame can actually show: the selection bar
    // and its gutter take two columns of the inner width.
    let budget = (inner.width as usize).saturating_sub(2);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, row) in app
        .timeline_entries
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        // M196: depth and the off-lane marker. Without them a branch the user
        // rewound away from looked exactly like the conversation they are in,
        // which is the specific confusion this view exists to remove.
        // M196: a label is drawn on its row. A label the store holds but the
        // picker does not show is a label the user cannot find, which makes the
        // feature worse than useless -- they would re-create it.
        let labelled = match row.label.as_deref() {
            Some(l) => format!(" {} {l}", app.theme.glyphs.folder),
            None => String::new(),
        };
        // A folded row says so. Without a marker the subtree simply vanishes
        // and the reader cannot tell "collapsed" from "never had children" --
        // which is the same silent-loss problem the off-lane marker addresses.
        // Two DIFFERENT glyphs, because the two states are different facts:
        // closed means "children are hidden, press Ctrl+Right", open means
        // "children are shown, press Ctrl+Left". The same glyph for both would
        // make a collapsed branch indistinguishable from an expanded one -- the
        // reader would have to press the key to find out, which is the
        // guessing this repo avoids.
        //
        // **From the repertoire table (A43).** These were literals, so on the ASCII set --
        // which is the *default* on Windows without a UTF-8 host, i.e. most of the people
        // who filed the ASCII reports -- both rendered as boxes. The fold feature's entire
        // purpose is carried by the difference between these two characters, and that
        // difference is exactly what a box destroys. The CP437 frame test never drove this
        // panel, which is why the M219 sweep missed them.
        let fold_mark = if app.folded.contains(&row.id) {
            format!(" {}", app.theme.glyphs.fold_closed)
        } else if super::commands::has_children(row, &app.timeline_all) {
            format!(" {}", app.theme.glyphs.fold_open)
        } else {
            String::new()
        };
        let raw = format!(
            "{}{}#{seq} [{tag}]{fold_mark}{labelled} {snippet}",
            "  ".repeat(row.depth),
            // A `·` for "not the lane you are on". The alternative -- leaving it
            // blank -- makes an abandoned branch indistinguishable from the live
            // one, which is the thing a reader cannot afford to get wrong.
            if row.on_active { " " } else { "\u{00b7} " },
            seq = row.seq,
            tag = row.kind,
            fold_mark = fold_mark,
            labelled = labelled,
            snippet = row.snippet,
        );
        // **Truncated in COLUMNS, by the one helper that knows a column is not a byte.**
        //
        // This was `if raw.len() > budget { &raw[..budget - 3] }` -- and that panics. `budget`
        // is columns and `raw[..n]` is bytes, so the slice index is wrong by however much the
        // prefix and the snippet are non-ASCII. The prefix guarantees the skew: `📁` is four
        // bytes, `▾`/`▸` three. Snippets are produced up to 100 columns wide
        // (`PREVIEW_COLUMNS`), so at an 80-column terminal this branch is the **normal** path,
        // not an edge case, and any em dash, curly quote or emoji within three bytes of the
        // cut ends the process with `byte index N is not a char boundary`.
        //
        // The CJK test that does exist passed by arithmetic luck: its prefix is exactly 11
        // bytes and the cut lands 60 bytes in, which is exactly 20 three-byte characters.
        // A boundary, by coincidence. It exercised truncation and missed the panic, which is
        // the project's own "a control that passes is a control you have not proven" lesson
        // landing in a new place.
        let display = crate::state::truncate_public(&raw, budget, app.theme.glyphs.ellipsis);
        lines.push(overlay::list_row(
            &app.theme,
            i == app.timeline_index,
            &display,
            None,
            inner.width as usize,
        ));
    }
    // M196: the search field, drawn INSIDE the panel above the rows. A search
    // that is not on screen while it is being typed cannot be confirmed: the
    // reader has no way to tell filtering from keys going somewhere.
    //
    // It was first added to the MODEL picker's renderer, where it never had any
    // business being -- and no test noticed for a full round, because every test
    // in this file asserted on state and none rendered this panel. The test that
    // now exists is `the_tree_panel_draws_its_filter_search_field_and_fold_glyphs`.
    if app.tree_searching || !app.tree_query.is_empty() {
        // The budget is in COLUMNS (it subtracts the `/` prefix and the cursor block),
        // so it must be spent in columns: taking `budget` *characters* of a CJK query
        // draws a row twice as wide as the panel it is typed into.
        let budget = (inner.width as usize).saturating_sub(6);
        let shown = crate::wrap::fit_width(&app.tree_query, budget);
        lines.push(Line::from(vec![
            Span::styled("/", app.theme.accent()),
            Span::styled(shown, app.theme.assistant()),
            Span::styled("\u{2588}", app.theme.accent()),
        ]));
    }
    if !app.tree_query.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "{} of {} match",
                app.timeline_entries.len(),
                app.timeline_all.len()
            ),
            app.theme.system(),
        )));
    }
    lines.push(Line::from(""));
    // The position counter and the key hints share the footer line: a `hint_row`
    // carries the hints, and the counter stays where the reader looks for it.
    let counter = format!("{}/{}", app.timeline_index + 1, total);
    let mut footer = vec![Span::styled(counter.clone(), app.theme.dim())];
    footer.push(Span::styled(" · ", app.theme.faint()));
    let arrows = overlay::up_down(app.theme.glyphs.up, app.theme.glyphs.down);
    let shift_l = format!("{}L", app.theme.glyphs.shift);
    // **Budgeted against this panel's own width.** Seven hints are about 74 columns and the
    // inner width is 60 at an 80-column terminal, so the row wrapped and took a line from the
    // list above it. The counter's own width comes out of the same budget, because it is on
    // the same line — which is the whole reason the hint row and the counter share a footer.
    let budget =
        (inner.width as usize).saturating_sub(crate::ui::layout::display_width(&counter) + 3);
    footer.extend(
        overlay::hint_row_in(
            &app.theme,
            // M196: the tree's own keys, all of which exist. A hint row that
            // lists fewer keys than the panel binds is the same defect as an
            // unbound key with no response -- the capability is there and
            // undiscoverable.
            &[
                (&arrows, "move"),
                // P-306: the page step is named where the keys are — a
                // screenful step with no hint is undiscoverable at exactly
                // the branch sizes that need it. Early in the list so the
                // budget drops later hints first, not this one.
                ("left/right", "page"),
                // P-074: fork mode forks on Enter instead of jumping — the
                // hint names the action the key takes, not the other mode's.
                (
                    "enter",
                    if app.timeline_fork_refill.is_some() {
                        "fork"
                    } else {
                        "jump"
                    },
                ),
                ("f", "filter"),
                ("/", "search"),
                // From the repertoire table: `⇧` is not in CP437, and this is the ONLY
                // place the Shift+L chord is advertised, so on a legacy console the one
                // undiscoverable binding was the one that rendered as a box (A43).
                (&shift_l, "label"),
                ("k", "fork"),
                ("esc", "close"),
            ],
            budget,
        )
        .spans,
    );
    lines.push(Line::from(footer));
    f.render_widget(Paragraph::new(lines), inner);
}

/// The slash-command popup, anchored directly above the composer.
///
/// Anchored rather than centred on purpose: it is a completion list for what the
/// user is *currently typing*, so it belongs next to the text, and a centred
/// panel would pull the eye away from the caret on every keystroke.
pub(super) fn draw_slash_autocomplete(f: &mut Frame<'_>, app: &App, transcript_area: Rect) {
    use crate::ui::overlay::{self, Panel};

    let candidates = app.popup_candidates();
    if !app.slash_open || candidates.is_empty() {
        return;
    }
    let total = candidates.len();
    // Eight visible rows, then the list scrolls: taller than that and the popup
    // covers the conversation it is helping with.
    let capacity = 8usize.min(total);
    let start = crate::scroll::scroll_start(total, app.slash_index, capacity);
    let window = &candidates[start..(start + capacity).min(total)];

    // **The box takes the width the terminal offers.** It was capped at 56
    // columns, and at 80 that wasted twenty-four of them on the transcript while
    // the description column had thirty-seven to work with — which is why
    // `browse session files in this workspace` arrived as `browse session files
    // in this workspac…`. The description is the part a user reads to choose, so
    // it gets the room, and the cap only bites on a genuinely narrow terminal.
    let width = transcript_area.width.clamp(40, 80);
    let inner_width = (width as usize).saturating_sub(4);

    // The label column is **measured from the rows on screen**, not fixed: `/models`
    // is seven columns and `/scoped-models` is fourteen, and a shared column lets
    // the eye find the description by position rather than by scanning. The gap
    // after it is the same `MIN_META_GAP` the shared row renderer reserves, so a
    // slash row and a picker row use one rhythm.
    const MIN_META_GAP: usize = 2;
    let label_col = window
        .iter()
        .map(|item| crate::ui::layout::display_width(item.value.as_str()))
        .max()
        .unwrap_or(0)
        + MIN_META_GAP;

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(window.len() * 2);
    for (offset, item) in window.iter().enumerate() {
        let selected = start + offset == app.slash_index;
        // The bar marks selection for a monochrome terminal, and the accent
        // carries it for the rest: two statements of the same fact, neither of
        // which is the only one.
        let bar = if selected {
            app.theme.glyphs.cursor_bar
        } else {
            " "
        };
        let label_style = if selected {
            app.theme.selected()
        } else {
            app.theme.text()
        };
        let value = safe_to_show(&item.value);
        let label_width = crate::ui::layout::display_width(&value);
        let mut row = vec![
            Span::styled(
                format!("{bar}{}", " ".repeat(overlay::GUTTER_AFTER_BAR)),
                app.theme.accent(),
            ),
            Span::styled(value.clone(), label_style),
        ];
        // A description is wrapped onto continuation lines rather than
        // truncated when it will not fit beside its command. Truncating said
        // `show or switch the permission preset (A…` — worse than saying nothing,
        // because it looks like the sentence ended there. The continuation is
        // indented under the description column, so the second line reads as
        // more of the same thought and not as a new row.
        let desc_budget = inner_width.saturating_sub(1 + 1 + label_col).max(1);
        let description = safe_to_show(&item.description);
        if description.is_empty() {
            row.push(Span::raw(" ".repeat(desc_budget)));
            lines.push(Line::from(row));
            continue;
        }
        let pad = label_col.saturating_sub(label_width);
        row.push(Span::raw(" ".repeat(pad)));
        row.push(Span::styled(description.clone(), app.theme.dim()));
        lines.push(Line::from(row));
        // Wrap what did not fit beside the command onto continuation lines rather
        // than cutting it: a truncated description used to read
        // `show or switch the permission preset (A…`, which is worse than useless
        // because it looks as though the sentence ended there. `wrap_line` is
        // measured in columns from a span list, and the continuation is padded to
        // the column the description began, so a second line reads as more of the
        // same thought rather than as a new row.
        //
        // **The first line's cut is on a word boundary, not on a column count.**
        // `fit_width` cuts wherever the budget runs out, which leaves
        // `...underscore spell` + `g)` on its own line — a one-word orphan that
        // reads as a mistake. Breaking at the last space before the budget means
        // the remainder is a whole phrase, which is what wrapping is for.
        //
        // **Split on a byte boundary that a grapheme respects.** The cut index
        // comes from a column budget, so it can land inside a multi-byte
        // sequence; slicing by it would panic or split a grapheme. Searching
        // backwards for the nearest ASCII space lands on a char boundary by
        // construction — a space is one byte and cannot be part of a sequence.
        let head = crate::wrap::fit_width(description.as_str(), desc_budget);
        let cut = if head.len() < description.len() {
            let candidate = description[..head.len()].rfind(' ').unwrap_or(head.len());
            description[..candidate].trim_end().len()
        } else {
            description.len()
        };
        if cut < description.len() {
            let continuation = description[cut..].trim_start();
            for wrapped in crate::wrap::wrap_line(&[(continuation, app.theme.dim())], desc_budget) {
                let mut line = vec![Span::raw(" ".repeat(1 + 1 + label_col))];
                line.extend(
                    wrapped
                        .into_iter()
                        .map(|(text, style)| Span::styled(text, style)),
                );
                lines.push(Line::from(line));
            }
        }
    }
    lines.push(Line::from(""));
    let arrows = overlay::up_down(app.theme.glyphs.up, app.theme.glyphs.down);
    lines.push(overlay::hint_row(
        &app.theme,
        &[(&arrows, "pick"), ("tab", "apply"), ("esc", "dismiss")],
    ));
    let capacity = lines.len().min(20);
    let height = capacity as u16 + 2;
    let area = Rect {
        x: transcript_area.x,
        y: transcript_area.bottom().saturating_sub(height),
        width,
        height,
    };
    let panel = Panel {
        theme: &app.theme,
        area,
        title: "commands",
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    f.render_widget(Paragraph::new(lines), inner);
}

/// The permission dialog.
///
/// This is the most important screen in the harness: it is the moment the user
/// decides whether the agent may do something. It is drawn with the *danger*
/// accent rather than the ordinary focus accent, because "this owned the
/// keyboard" and "this can authorize a write" are different statements, and
/// telling them apart at a glance is the point. That is now true of the code as
/// well as of this comment: the frame's colour is passed in
/// ([`crate::ui::overlay::Panel::border`]) instead of being inherited from
/// `focused: true`, which yielded the same accent every other overlay uses.
///
/// **Six rows, not eleven** — for an ask with nothing to preview. The dialog is
/// a yes/no with one line of detail, and a tall box for a small question reads
/// as more consequential than it is — which is the opposite of what the danger
/// colour is for. The skill's own rule is 5–7 rows for a confirm prompt
/// (`references/layout.md`); the body keeps the capability, the one line that
/// says what will actually happen, and the four decision keys, and drops the
/// blank separator and the `pattern` field, which restated what the detail line
/// already shows.
///
/// **An ask that carries a preview grows, and only then.** `write` and `edit`
/// merge a diff of the change into the ask's detail (epsilon-core's `ask_detail`),
/// and a reader approving a write having seen neither the old nor the new bytes
/// is the gap that was carrying. So the `what` line keeps its place, a `diff`
/// summary row joins it, and the diff follows — as many rows as the terminal
/// can give, with a count for the rest, which is opencode's expanded permission
/// prompt (`references/opencode-1.18.34/packages/tui/src/routes/session/permission.tsx:47-87`:
/// a scrollbox for the diff, and `No diff provided` rather than omitting it).
pub(super) fn draw_dialog(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    /// Interior rows the body needs: capability, detail, and the two key lines.
    ///
    /// Stated once and used for the height, so the box cannot be given a size the
    /// body does not fill — an oversized dialog is how the eleven-row version
    /// happened.
    const BODY_ROWS: u16 = 4;
    const CHROME_ROWS: u16 = 2;
    /// Interior rows the diff costs before its own: the `diff +3 -1` summary row.
    const DIFF_SUMMARY_ROWS: u16 = 1;

    let theme = &app.theme;
    let width = Width::PROMPT.resolve(outer);
    // The change this call would make, when the tool previewed one. `None` for a
    // shell command, a read, a search — every ask that existed before the preview
    // seam — and the body drawn for those is the one it has always been.
    let preview = app
        .pending_ask
        .as_ref()
        .and_then(|ask| ask_preview(&ask.request));
    // Measured against the panel's own interior, which is what the rows must fit.
    let inner_width = width.saturating_sub(4) as usize;
    let mut diff_rows: Vec<Line<'static>> = preview
        .as_ref()
        .map(|diff| diff_body_rows(diff, inner_width, theme))
        .unwrap_or_default();
    // How many of them fit. The dialog may grow to the whole screen, and does —
    // that is the only way a large change is fully visible — but it may not pass
    // it: `centered` clamps, and a clipped edge is the frame's own bottom rule.
    let room = outer
        .height
        .saturating_sub(CHROME_ROWS + BODY_ROWS + DIFF_SUMMARY_ROWS);
    let shown = if diff_rows.is_empty() || room == 0 {
        diff_rows.clear();
        Vec::new()
    } else if diff_rows.len() as u16 <= room {
        diff_rows
    } else {
        // A head-kept cut with a count, which is the affordance a transcript card
        // prints for the same situation: the reader is told there is more of the
        // change than is on the screen rather than being left to assume this is all
        // of it. A diff reads from its first hunk down, so the head is the half
        // worth keeping (`ui::card::body_keeps_tail`).
        let keep = room.saturating_sub(1) as usize;
        let hidden = diff_rows.len() - keep;
        let mut shown: Vec<Line<'static>> = diff_rows.into_iter().take(keep).collect();
        shown.push(Line::from(Span::styled(
            crate::ui::card::more_lines(hidden, None, true),
            theme.faint(),
        )));
        shown
    };
    let diff_needed = if preview.is_some() && !shown.is_empty() {
        DIFF_SUMMARY_ROWS + shown.len() as u16
    } else {
        0
    };
    let height = (BODY_ROWS + CHROME_ROWS + diff_needed).min(outer.height);
    let panel = Panel {
        theme: &app.theme,
        area: overlay::centered(outer, width, height),
        title: "permission required",
        focused: true,
        // The one overlay that is not merely asking a question.
        border: Some(app.theme.danger()),
    };
    let inner = panel.open(f);

    // The permission dialog is the worst place in this product for a command to be
    // displayed as something it is not: it is the one screen where a reader decides
    // whether work runs, and the text they decide on comes from the **agent**, not from the
    // user. So both fields are sanitised, the dialog says when it did, and
    // `handle_approval_key` refuses the approve keys for an ask that needed it. See
    // `crate::text_safety`.
    let body: Vec<Line<'static>> = match app.pending_ask.as_ref() {
        Some(ask) => {
            let req = &ask.request;
            // A labelled field, then its value: the label is dim so a reader
            // scanning for the answer skips it, and the value is not. The label
            // column is five wide — wide enough for `cap`, narrow enough that the
            // decision keys below keep their descriptions inside the panel.
            let field = |label: &str, value: String| {
                Line::from(vec![
                    Span::styled(format!("{label:<5}"), theme.faint()),
                    Span::styled(value, theme.text()),
                ])
            };
            // The detail is the line the skill asks for: what will actually
            // happen. For a shell call that is the command, not a JSON object
            // with the command buried in it — see [`summarize_detail`].
            let detail = summarize_detail(&req.detail);
            let (cap, cap_hit) = crate::text_safety::display_safe(&req.capability);
            let (what, what_hit) = crate::text_safety::display_safe(&detail);
            let mut lines = vec![
                field("cap", cap.into_owned()),
                field("what", what.into_owned()),
            ];
            // The change itself, between the one-line answer and the decision keys.
            //
            // **The `what` line comes first and is unchanged.** A reader skimming still
            // gets the path; the diff is what a reader who wants the bytes reads. The
            // label carries `+a -b` because that is the size of the change, which is the
            // first question after "what file".
            if preview.is_some() && !shown.is_empty() {
                let (added, removed) = preview.as_ref().map(|d| d.counts()).unwrap_or((0, 0));
                lines.push(field("diff", format!("{added} added, {removed} removed")));
                lines.extend(shown);
            }
            // **The mode replaces the answer keys; it does not join them (A42).**
            //
            // `guidance_mode` was set at `keys.rs` and read nowhere, so this block kept
            // advertising `y allow once` while `y` typed a letter into the composer — the
            // screen said one thing and the keyboard did another. That is M261's finding
            // reintroduced one screen over, and the fix is M261's: render the state where
            // the reader is deciding.
            //
            // **Replaced, not appended.** Printing the guidance line *underneath* the keys
            // would still leave the screen advertising two different keyboards at once,
            // which is the same lie with an extra sentence. And the guidance line has to
            // say what guidance is, because `e` moves the keyboard somewhere with no prompt
            // of its own — a reader who pressed it has no evidence they are anywhere but
            // the ask.
            if app.guidance_mode {
                lines.push(Line::from(Span::styled(
                    "guidance — the keyboard is here now",
                    theme.warn(),
                )));
                lines.push(Line::from(vec![
                    Span::styled("enter", theme.danger()),
                    Span::styled(" deny with this", theme.dim()),
                    Span::styled("        esc", theme.danger()),
                    Span::styled(" back to the ask", theme.dim()),
                ]));
            } else if cap_hit.any() || what_hit.any() {
                // Named, never silent. A permission prompt that quietly dropped a
                // character is worse than one that says so, because the reader has no
                // other way to learn the text was not what it claimed.
                lines.push(Line::from(Span::styled(
                    format!(
                        "this request carries {} display-steering character(s), removed for display",
                        cap_hit.total() + what_hit.total()
                    ),
                    theme.danger(),
                )));
                lines.push(Line::from(Span::styled(
                    "it cannot be approved as shown",
                    theme.danger(),
                )));
                lines.push(Line::from(vec![
                    Span::styled("n", theme.danger()),
                    Span::styled(" deny", theme.dim()),
                    Span::styled("      e", theme.danger()),
                    Span::styled(" deny + guidance", theme.dim()),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("y", theme.success()),
                    Span::styled(" allow once", theme.dim()),
                    Span::styled("   a", theme.success()),
                    Span::styled(" always (session)", theme.dim()),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("n", theme.danger()),
                    Span::styled(" deny", theme.dim()),
                    Span::styled("         e", theme.danger()),
                    Span::styled(" deny + guidance", theme.dim()),
                ]));
            }
            lines
        }
        // Reached only when the ask was taken between the draw call and here,
        // which the caller's `is_some` check makes a one-frame race.
        None => vec![Line::from(Span::styled("resolving…", theme.dim()))],
    };

    f.render_widget(Paragraph::new(body), inner);
}

/// The diff an ask carries, when it carries one that can be shown.
///
/// `None` for a shell command, a read, a search — every ask that existed before the preview
/// seam — which is why the dialog for those is the one it always was.
///
/// **A preview that is not display-safe is withheld, not stripped.** The refusal rule for an
/// ask whose own text carries display-steering characters is
/// `epsilon_runtime::ask_is_display_honest` (M261), and it reads the capability and the
/// one-line `what`. A diff is a surface that predicate does not cover, so the same rule is
/// applied here in the half that applies: a diff whose lines were quietly rewritten would have
/// a reader approving bytes they never saw, which is exactly what M261 refuses. So it is not
/// drawn, and the dialog keeps the body it had. (It is *not* folded into the refusal row
/// either: that row's keys are refused by `handle_approval_key` from M261's own predicate, and
/// advertising a refusal the keyboard does not honour is the defect M261 was.)
fn ask_preview(
    request: &epsilon_contracts::agent_event::PermissionRequest,
) -> Option<crate::state::Diff> {
    let text = request.detail.get("preview")?.as_str()?;
    let (safe, stripped) = crate::text_safety::display_safe(text);
    if stripped.any() {
        return None;
    }
    let diff = crate::state::parse_diff(&safe);
    (!diff.lines.is_empty()).then_some(diff)
}

/// The diff's rows for the dialog: sign, line number, then the text cut to fit.
///
/// The shape a tool result's diff already uses — the same signs, the same number column, the
/// same dim context, the same success/danger pair (`ui::card`'s unified view) — because a
/// reader must not have to learn a second notation for the screen that authorises the change.
/// This is the same *shape* in the dialog's own types, not a call into that renderer: the
/// card's row builder is private to it and returns `CardRow`, while this is a `Paragraph` of
/// `Line`s. If a second panel needs these rows, the move is to widen that one, not to copy
/// this a third time.
fn diff_body_rows(
    diff: &crate::state::Diff,
    width: usize,
    theme: &crate::ui::theme::Theme,
) -> Vec<Line<'static>> {
    // The numbers are the first thing to go when the box is narrow: the sign is what colour
    // stands in for, and a clipped number is worse than none (`Diff::unified_numbers_fit`).
    let numbers = diff.unified_numbers_fit(width);
    diff.lines
        .iter()
        .map(|line| {
            use crate::state::DiffKind;
            let base = match line.kind {
                DiffKind::Add => theme.success(),
                DiffKind::Remove => theme.danger(),
                DiffKind::Hunk => theme.accent(),
                _ => theme.dim_strong(),
            };
            let sign = match line.kind {
                DiffKind::Add => theme.glyphs.plus,
                DiffKind::Remove => theme.glyphs.minus,
                _ => " ",
            };
            let number = match (numbers, line.number) {
                (true, Some(n)) => format!("{n:>width$} ", width = diff.number_width),
                _ => format!("{:width$} ", "", width = diff.number_width),
            };
            let head = format!("{sign} ");
            let used =
                crate::ui::layout::display_width(&head) + crate::ui::layout::display_width(&number);
            Line::from(vec![
                Span::styled(head, base),
                Span::styled(number, theme.faint()),
                Span::styled(
                    crate::wrap::fit_width(&line.text, width.saturating_sub(used)),
                    base,
                ),
            ])
        })
        .collect()
}

/// M32: masked API-key prompt (opencode DialogPrompt parity). The real key
/// lives in `app.auth_key` but rendering shows one dot per character so the
/// secret never appears in the transcript, in a screenshot, or in a scrollback
/// buffer.
pub(super) fn draw_label_prompt(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    if !app.label_prompt_open {
        return;
    }
    let width = Width::PROMPT.resolve(outer);
    let height = 7u16.min(outer.height.saturating_sub(2)).max(5);
    let panel = Panel {
        theme: &app.theme,
        area: overlay::centered(outer, width, height),
        title: "label this entry",
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    let theme = &app.theme;
    let mut lines: Vec<Line<'static>> = Vec::new();

    let target = app.label_prompt_target.as_deref().unwrap_or("?");
    let shown = crate::wrap::fit_width(&app.label_prompt_text, 60);
    lines.push(Line::from(Span::styled(
        format!("entry {target}"),
        theme.system(),
    )));
    // The caret sits where the next character will land, so a reader can see
    // where they are without the field having to be a real widget.
    let caret = "█";
    let (before, after) = if app.label_prompt_cursor >= app.label_prompt_text.chars().count() {
        (shown.clone(), String::new())
    } else {
        let at = app
            .label_prompt_text
            .chars()
            .take(app.label_prompt_cursor)
            .collect::<String>();
        let rest = app
            .label_prompt_text
            .chars()
            .skip(app.label_prompt_cursor)
            .collect::<String>();
        (at, rest)
    };
    lines.push(Line::from(vec![
        Span::styled("> ", theme.accent()),
        Span::styled(before, theme.assistant()),
        Span::styled(caret, theme.accent()),
        Span::styled(after, theme.assistant()),
    ]));
    lines.push(Line::from(Span::styled(
        if app.label_prompt_text.trim().is_empty() {
            "Enter clears the label \u{00b7} Esc cancels"
        } else {
            "Enter saves \u{00b7} Esc cancels"
        },
        theme.system(),
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_auth_prompt(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    let width = Width::PROMPT.resolve(outer);
    let height = 8u16.min(outer.height.saturating_sub(2)).max(5);
    let oauth_mode = app.oauth_code_tx.is_some();
    let title = if oauth_mode {
        format!("OAuth login · {}", app.auth_provider)
    } else {
        format!("API key · {}", app.auth_provider)
    };
    let panel = Panel {
        theme: &app.theme,
        area: overlay::centered(outer, width, height),
        title: &title,
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    let theme = &app.theme;

    let masked_len = app.auth_key.chars().count();
    let masked: String = theme.glyphs.bullet.repeat(masked_len);
    let hint = match (oauth_mode, masked_len) {
        (true, 0) => "paste the redirect URL or the bare code — Ctrl+V works, Enter sends",
        (true, _) => "Enter sends · Esc cancels",
        (false, 0) => "paste your key — it stays masked, Enter saves",
        (false, _) => "Enter saves · Esc cancels",
    };

    let body = vec![
        Line::from(Span::styled(
            if oauth_mode {
                "hidden while typing"
            } else {
                "the key is never shown, here or in the transcript"
            },
            theme.faint(),
        )),
        Line::from(Span::styled(masked, theme.warn())),
        Line::from(""),
        Line::from(Span::styled(hint, theme.dim())),
    ];
    f.render_widget(Paragraph::new(body), inner);

    // The caret is placed inside the masked field so a paste lands visibly.
    // Column arithmetic is in display cells, and the field is the second row of
    // the body (the first is the explanatory line).
    let cursor_col = app
        .auth_key_cursor
        .min(masked_len)
        .min(inner.width.saturating_sub(1) as usize);
    let x = inner.x + cursor_col as u16;
    let y = inner.y + 1;
    if x < inner.x + inner.width && y < inner.y + inner.height {
        f.set_cursor_position((x, y));
    }
}

/// Level picker for bare `/think` (P-048): the provider-picker shape with
/// one row per level and the current one marked in words, not only colour.
pub(super) fn draw_think_picker(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};
    let levels = crate::picker::match_think_levels(&app.input);
    let width = Width::PANEL.resolve(outer);
    let height = 12u16.min(outer.height.saturating_sub(2));
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned like the model picker: the two are the same list at
        // different moments, and a panel that moves between them is disorienting.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: "thinking — pick a level",
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    let theme = &app.theme;
    let capacity = inner.height.saturating_sub(2).max(1) as usize;
    let start = crate::scroll::scroll_start(levels.len(), app.think_pick_index, capacity);
    let end = (start + capacity).min(levels.len());
    let mut lines: Vec<Line<'static>> = Vec::new();
    if levels.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("no level matches '{}' — clear the filter", app.input),
            theme.dim(),
        )));
    }
    for (index, level) in levels.iter().enumerate().skip(start).take(end - start) {
        // The current level is a word, not only a colour: consoles that
        // cannot render the highlight still read which level is active.
        let current = if *level == app.thinking {
            Some("current")
        } else {
            None
        };
        lines.push(overlay::list_row(
            theme,
            index == app.think_pick_index,
            level.as_str(),
            current,
            inner.width as usize,
        ));
    }
    lines.push(Line::from(""));
    let arrows = crate::ui::overlay::up_down(theme.glyphs.up, theme.glyphs.down);
    lines.push(overlay::hint_row(
        theme,
        &[
            ("type", "filter"),
            (&arrows, "move"),
            ("enter", "apply"),
            ("esc", "keep current"),
        ],
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}

/// The model-question overlay (criterion 4): one parked `question` tool call.
///
/// The think-picker chrome (`Panel`, `list_row`, `hint_row`, measured
/// capacity) over the ask snapshot: tab headers when several questions are
/// queued, the current question, its option rows with `picked` marks (words,
/// not only colour), the custom-answer row and input, or the review list on
/// the confirm stop. Nothing here answers: keys do that in `ask.rs`.
pub(super) fn draw_ask(f: &mut Frame<'_>, app: &mut App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};
    let Some(frame) = super::ask::draw_snapshot(app) else {
        return;
    };
    super::ask::note_painted(app, frame.id.clone());
    let width = Width::PANEL.resolve(outer);
    let theme = &app.theme;
    let inner_width = width.saturating_sub(4) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    if frame.show_tabs {
        let mut tabs: Vec<Span<'static>> = Vec::new();
        for (i, (header, active, answered)) in frame.tabs.iter().enumerate() {
            if i > 0 {
                tabs.push(Span::styled(" · ".to_string(), theme.dim()));
            }
            let style = if *active {
                theme.selected()
            } else if *answered {
                theme.text()
            } else {
                theme.dim()
            };
            tabs.push(Span::styled(header.clone(), style));
        }
        lines.push(Line::from(tabs));
    }
    let mut head = frame.question.clone();
    if frame.multi {
        head.push_str(" (select all that apply)");
    }
    lines.push(Line::from(Span::styled(head, theme.text())));
    if frame.at_review {
        for (header, value) in &frame.review {
            lines.push(Line::from(vec![
                Span::styled(format!("{header}: "), theme.dim()),
                Span::styled(value.clone(), theme.text()),
            ]));
        }
    }
    for (index, row) in frame.rows.iter().enumerate() {
        let picked = if row.picked { Some("picked") } else { None };
        lines.push(overlay::list_row(
            theme,
            index == frame.selected,
            &format!("{}. {}", index + 1, row.label),
            picked,
            inner_width,
        ));
        if !row.description.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("    {}", row.description),
                theme.dim(),
            )));
        }
    }
    if frame.custom_open {
        lines.push(Line::from(vec![
            Span::styled("> ".to_string(), theme.accent()),
            Span::styled(frame.custom_text.clone(), theme.text()),
        ]));
    }
    lines.push(Line::from(""));
    let arrows = overlay::up_down(theme.glyphs.up, theme.glyphs.down);
    let hints: Vec<(&str, &str)> = if frame.custom_open {
        vec![("type", "answer"), ("enter", "add"), ("esc", "back")]
    } else if frame.at_review {
        vec![("enter", "submit"), ("tab", "back"), ("esc", "dismiss")]
    } else {
        vec![
            (&arrows, "move"),
            ("enter", if frame.multi { "toggle" } else { "answer" }),
            ("tab", "next"),
            ("esc", "dismiss"),
        ]
    };
    lines.push(overlay::hint_row(theme, &hints));
    let height = (lines.len() + 2).min(outer.height.saturating_sub(2) as usize) as u16;
    let panel = Panel {
        theme,
        // Top-aligned like the model picker: the overlay opens mid-turn, and
        // a panel that moves between openings is disorienting.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height.max(5))
        },
        title: "question for you",
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}

/// P-047: `/scoped-models` checklist over the F2 cycle scope.
///
/// The think-picker chrome (Panel, `list_row`, `hint_row`, measured
/// capacity) over the flat catalog rows, with staged membership as the
/// checkbox: `[x]` is in the scope F2 cycles, `[ ]` is not. Words, not only
/// colour — the same console rule as the think picker's `current` mark.
pub(super) fn draw_scope_picker(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};
    let rows = crate::picker::build_rows_with(
        &app.input,
        &app.model_id,
        &[],
        &app.favorites,
        None,
        &app.live_cache,
        &app.customs,
    );
    let width = Width::PANEL.resolve(outer);
    let height = 16u16.min(outer.height.saturating_sub(2));
    let panel = Panel {
        theme: &app.theme,
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: &format!(
            "scoped models · {} of {}",
            app.favorites.len(),
            epsilon_core::config::FAVORITE_MAX
        ),
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    let theme = &app.theme;
    let capacity = inner.height.saturating_sub(2).max(1) as usize;
    let start = crate::scroll::scroll_start(rows.len(), app.scope_index, capacity);
    let end = (start + capacity).min(rows.len());
    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("no model matches '{}' — clear the filter", app.input),
            theme.dim(),
        )));
    }
    for (index, row) in rows.iter().enumerate().skip(start).take(end - start) {
        let mark = if row.is_favorite { "[x]" } else { "[ ]" };
        lines.push(overlay::list_row(
            theme,
            index == app.scope_index,
            &format!("{mark} {} · {}", row.label, row.provider),
            if row.is_favorite {
                Some("in scope")
            } else {
                None
            },
            inner.width as usize,
        ));
    }
    lines.push(Line::from(""));
    let arrows = crate::ui::overlay::up_down(theme.glyphs.up, theme.glyphs.down);
    lines.push(overlay::hint_row(
        theme,
        &[
            ("type", "filter"),
            (&arrows, "move"),
            ("space", "toggle"),
            ("^S", "save"),
            ("esc", "discard"),
        ],
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}

/// M32b: provider picker for bare `/login` — opencode DialogProvider parity.
///
/// Renders the provider list filtered by the current `input`. Selecting one
/// opens the masked API-key prompt; Esc closes. Filter + list reuse the
/// existing `/models` provider rows, with auth badge and active mark so a
/// first-run user can see at a glance what is wired up.
pub(super) fn draw_auth_picker(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};
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
    let width = Width::PANEL.resolve(outer);
    let height = 15u16.min(outer.height.saturating_sub(2));
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned like the model picker: the two are the same list at
        // different moments, and a panel that moves between them is disorienting.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: "connect — pick a provider",
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    let theme = &app.theme;
    let capacity = inner.height.saturating_sub(2).max(1) as usize;
    let start = crate::scroll::scroll_start(filtered.len(), app.auth_pick_index, capacity);
    let end = (start + capacity).min(filtered.len());
    let mut lines: Vec<Line<'static>> = Vec::new();
    if filtered.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("no match for '{}' — type another provider id", app.input),
            theme.dim(),
        )));
    }
    for (index, (_, prow)) in filtered.iter().enumerate().skip(start).take(end - start) {
        // Auth state is a word, not only a colour: a key glyph is invisible on a
        // console that cannot render it, and "ready to use" is not a detail to
        // lose to a font.
        let auth = if prow.auth_ready {
            // Name the variable when the environment is the credential (P-313):
            // "key (env: NAME)". Stored entries read plain "key"; the lookup is
            // the same stored-wins precedence the resolver uses.
            match crate::picker::provider_env_key(&prow.id, &app.customs)
                .and_then(|key| epsilon_llm::catalog::env_credential_name_live(&prow.id, &key))
            {
                Some(name) => format!("key (env: {name})"),
                None => "key".to_string(),
            }
        } else {
            "—".to_string()
        };
        let oauth = if epsilon_llm::oauth_login::login_flow(&prow.id).is_some() {
            "oauth"
        } else {
            ""
        };
        // Warn on a stored OAuth login nothing will ever read (P-313): dead
        // configuration that reads as "logged in". The helper owns the
        // flow/endpoint rule; the draw only words it.
        let warn = if epsilon_llm::catalog::oauth_credential_unused(
            epsilon_llm::auth::global_dir().as_deref(),
            &prow.id,
        ) {
            " · ! oauth login unused"
        } else {
            ""
        };
        let meta = format!("{auth} · {} models {oauth}{warn}", prow.model_count)
            .trim_end()
            .to_string();
        let label = if prow.is_active {
            format!("{} (active)", prow.id)
        } else {
            prow.id.clone()
        };
        lines.push(overlay::list_row(
            theme,
            index == app.auth_pick_index,
            &label,
            Some(&meta),
            inner.width as usize,
        ));
    }
    lines.push(Line::from(""));
    let arrows = crate::ui::overlay::up_down(theme.glyphs.up, theme.glyphs.down);
    lines.push(overlay::hint_row(
        theme,
        &[
            ("type", "filter"),
            (&arrows, "move"),
            ("enter", "paste key"),
            ("esc", "cancel"),
        ],
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

/// `/settings`: the in-TUI chooser over the five live rows.
///
/// Rows are computed per frame from [`App::setting_rows`], never stored, so
/// the value a row shows is the value the next Enter will change. The panel
/// owns the keyboard while open (see the `settings_open` arm in `keys.rs`);
/// the hint row names exactly the keys that arm handles.
pub(super) fn draw_settings(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};
    let rows = app.setting_rows();
    let width = Width::PANEL.resolve(outer);
    let height = 12u16.min(outer.height.saturating_sub(2));
    let panel = Panel {
        theme: &app.theme,
        // Top-aligned like the model picker: the two are the same list at
        // different moments, and a panel that moves between them is disorienting.
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: "settings",
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    let theme = &app.theme;
    let capacity = inner.height.saturating_sub(2).max(1) as usize;
    let start = crate::scroll::scroll_start(rows.len(), app.settings_index, capacity);
    let end = (start + capacity).min(rows.len());
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (index, (name, value)) in rows.iter().enumerate().skip(start).take(end - start) {
        lines.push(overlay::list_row(
            theme,
            index == app.settings_index,
            name,
            Some(value.as_str()),
            inner.width as usize,
        ));
    }
    lines.push(Line::from(""));
    let arrows = crate::ui::overlay::up_down(theme.glyphs.up, theme.glyphs.down);
    lines.push(overlay::hint_row(
        theme,
        &[(&arrows, "move"), ("enter", "change"), ("esc", "close")],
    ));
    // Where the rest lives: this panel holds five rows of ~fourteen persisted
    // things, and a settings screen that hides the others without naming them
    // is a discoverability dead end (the settings audit). One more hint row,
    // not more rows — a row without a writer would be the no-op AGENTS.md §2.7
    // forbids, so the panel points instead of padding.
    lines.push(overlay::hint_row(
        theme,
        &[("more", "/scoped-models /skill /login /editor")],
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

/// `/changes`: the checkpoint list for this session (or the workspace, on `all`).
///
/// Rows are `(path, tool)` pairs the command captured when the panel opened;
/// Enter previews the highlighted file in the viewer, Esc closes. An empty
/// list OPENS with its reason, the way the session browser does (P-243): a
/// refusal would hide exactly the message that teaches it.
pub(super) fn draw_changes(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};
    let width = Width::PANEL.resolve(outer);
    let height = 16u16.min(outer.height.saturating_sub(2));
    let panel = Panel {
        theme: &app.theme,
        area: Rect {
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: &format!("changes — {}", app.changes_scope),
        focused: true,
        border: None,
    };
    let inner = panel.open(f);
    let theme = &app.theme;
    let capacity = inner.height.saturating_sub(2).max(1) as usize;
    let start = crate::scroll::scroll_start(app.changes_rows.len(), app.changes_index, capacity);
    let end = (start + capacity).min(app.changes_rows.len());
    let mut lines: Vec<Line<'static>> = Vec::new();
    if app.changes_rows.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "no checkpoints {} — files the agent writes appear here",
                app.changes_scope
            ),
            theme.dim(),
        )));
    }
    for (index, (path, tool)) in app
        .changes_rows
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        lines.push(overlay::list_row(
            theme,
            index == app.changes_index,
            &safe_to_show(path),
            Some(tool.as_str()),
            inner.width as usize,
        ));
    }
    lines.push(Line::from(""));
    let arrows = crate::ui::overlay::up_down(theme.glyphs.up, theme.glyphs.down);
    lines.push(overlay::hint_row(
        theme,
        &[(&arrows, "move"), ("enter", "preview"), ("esc", "close")],
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod safe_to_show_tests {
    use super::safe_to_show;

    /// **The strip itself, asserted on text rather than on pixels.**
    ///
    /// It has to be here, not only on a frame: `a_frame_cannot_see_a_display_steering_character`
    /// records that ratatui's buffer does not carry `U+202E`, so a frame assertion passes
    /// whether or not the strip ran. This test is what actually fails when it is removed —
    /// verified, with `safe_to_show` stubbed to a no-op it fails on
    /// `left: "claude\u{202e}-3-opus"`.
    #[test]
    fn an_override_never_survives_into_a_row() {
        assert_eq!(safe_to_show("claude\u{202E}-3-opus"), "claude-3-opus");
        assert_eq!(safe_to_show("src/\u{2066}secrets.rs"), "src/secrets.rs");
        // Ordinary text is untouched, and an ordinary row must never be altered for this.
        assert_eq!(
            safe_to_show("cargo test -p epsilon-tui"),
            "cargo test -p epsilon-tui"
        );
    }
}
