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
    let title = format!("sessions — filter '{}'", app.input);
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
        lines.push(Line::from(Span::styled(
            "no matches — Backspace to widen, Esc closes",
            app.theme.dim(),
        )));
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
    footer.push(Span::styled(" · ", app.theme.faint()));
    footer.extend(overlay::hint_row(&app.theme, &[("enter", "resumes"), ("esc", "closes")]).spans);
    lines.push(Line::from(footer));
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
        lines.push(overlay::list_row(
            &app.theme,
            selected,
            // A **live** model's label and id come from a third-party endpoint, so they are
            // remote text in a row the reader chooses by reading. See `safe_to_show`.
            &safe_to_show(&format!("{marker}{auth} {star} {}", row.label)),
            Some(safe_to_show(row.id.as_str()).as_str()),
            inner.width as usize,
        ));
    }

    lines.push(Line::from(""));
    lines.push(overlay::hint_row(
        &app.theme,
        &[
            ("type", "filters (fuzzy)"),
            ("enter", "apply"),
            ("backspace/esc", "back"),
        ],
    ));
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
    let mut tail_rows: Vec<(LineKind, Line<'static>)> = Vec::new();
    for (text, style, kind) in [
        (
            app.ui.streaming_thinking(),
            app.theme.thinking(),
            LineKind::Thinking,
        ),
        (
            app.ui.streaming_text(),
            app.theme.assistant(),
            LineKind::Assistant,
        ),
    ] {
        if !text.is_empty() {
            for line in text.split('\n') {
                for wrapped in crate::wrap::wrap_line(&[(line, style)], body_width) {
                    tail_rows.push((
                        kind,
                        Line::from(
                            wrapped
                                .into_iter()
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
    let mut rows = app.render_cache.visible_rows_with_kinds(skip, visible);
    let remaining = visible.saturating_sub(rows.len());
    rows.extend(
        tail_rows
            .into_iter()
            .skip(skip.saturating_sub(total))
            .take(remaining),
    );

    // visible_rows already applied the viewport offset; decoration runs on the
    // rows that survived, and pads to the FRAME width, not the body width, so a
    // banded row reaches the terminal edge.
    let rows: Vec<Line<'static>> = rows
        .into_iter()
        .map(|(kind, line)| {
            let body: Vec<Span<'static>> = line.spans;
            crate::ui::transcript::decorate(&app.theme, kind, body, frame_width)
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
            y: outer.y + 1,
            ..overlay::centered(outer, width, height)
        },
        title: "commands",
        focused: true,
        border: None,
    };
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
    // The filter itself lives in the title bar, so the body stays a pure list.
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
    let Some(rows) = &app.at_rows else {
        return;
    };
    if rows.is_empty() {
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

pub(super) fn draw_timeline(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    if !app.timeline_open || app.timeline_entries.is_empty() {
        return;
    }
    let total = app.timeline_entries.len();
    let capacity = 12usize.min(outer.height.saturating_sub(6).max(1) as usize);
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
    let title = format!("timeline \u{00b7} {}", app.tree_filter.label());
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
                ("enter", "jump"),
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

    let candidates = crate::picker::slash_items(&app.input);
    if !app.slash_open || candidates.is_empty() {
        return;
    }
    let total = candidates.len();
    // Eight visible rows, then the list scrolls: taller than that and the popup
    // covers the conversation it is helping with.
    let capacity = 8usize.min(total);
    let start = crate::scroll::scroll_start(total, app.slash_index, capacity);
    let height = capacity as u16 + 1 + 2; // rows + hint + frame
    let area = Rect {
        x: transcript_area.x,
        y: transcript_area.bottom().saturating_sub(height),
        width: transcript_area.width.min(56),
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

    let mut lines: Vec<Line<'static>> = candidates[start..(start + capacity).min(total)]
        .iter()
        .enumerate()
        .map(|(offset, item)| {
            overlay::list_row(
                &app.theme,
                start + offset == app.slash_index,
                item.value,
                // The description is the hint column; `list_row` right-aligns it,
                // and an empty one means "no hint" rather than a blank column.
                Some(item.description).filter(|d| !d.is_empty()),
                inner.width as usize,
            )
        })
        .collect();
    lines.push(Line::from(""));
    let arrows = overlay::up_down(app.theme.glyphs.up, app.theme.glyphs.down);
    lines.push(overlay::hint_row(
        &app.theme,
        &[(&arrows, "pick"), ("tab", "apply"), ("esc", "dismiss")],
    ));
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
/// **Six rows, not eleven.** The dialog is a yes/no with one line of detail, and
/// a tall box for a small question reads as more consequential than it is —
/// which is the opposite of what the danger colour is for. The skill's own rule
/// is 5–7 rows for a confirm prompt (`references/layout.md`); the body keeps the
/// capability, the one line that says what will actually happen, and the four
/// decision keys, and drops the blank separator and the `pattern` field, which
/// restated what the detail line already shows.
pub(super) fn draw_dialog(f: &mut Frame<'_>, app: &App, outer: Rect) {
    use crate::ui::overlay::{self, Panel, Width};

    /// Interior rows the body needs: capability, detail, and the two key lines.
    ///
    /// Stated once and used for the height, so the box cannot be given a size the
    /// body does not fill — an oversized dialog is how the eleven-row version
    /// happened.
    const BODY_ROWS: u16 = 4;
    const CHROME_ROWS: u16 = 2;

    let width = Width::PROMPT.resolve(outer);
    let height = (BODY_ROWS + CHROME_ROWS).min(outer.height);
    let panel = Panel {
        theme: &app.theme,
        area: overlay::centered(outer, width, height),
        title: "permission required",
        focused: true,
        // The one overlay that is not merely asking a question.
        border: Some(app.theme.danger()),
    };
    let inner = panel.open(f);
    let theme = &app.theme;

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
        let auth = if prow.auth_ready { "key" } else { "—" };
        let oauth = if epsilon_llm::oauth_login::login_flow(&prow.id).is_some() {
            "oauth"
        } else {
            ""
        };
        let meta = format!("{auth} · {} models {oauth}", prow.model_count)
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
