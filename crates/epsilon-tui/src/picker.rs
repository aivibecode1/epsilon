//! Model-picker rows and the slash autocomplete that ranks them.
//!
//! **The command half of the registry is not written here any more.** `Command`,
//! `SlashItem`, `SLASH_ITEMS`, `COMMAND_HEADS`, `ALIAS_HEADS` and `parse_command` moved
//! down to `epsilon-contracts` (`crates/epsilon-contracts/src/command.rs`), which is the
//! only crate below both front ends. The `pub use` below is what keeps that move invisible:
//! `epsilon_tui::picker::Command` and `epsilon_tui::picker::parse_command` still resolve,
//! so **no call site in this workspace changed**.
//!
//! It was this crate's own edge that made the move worth making.
//! `crates/epsilon-desktop/src/commands.rs:79` reads `parse_command` and
//! `COMMAND_HEADS` from here — a native IDE importing a terminal crate to learn what a
//! `/word` means, in the crate whose manifest says it "draws no terminals and depends on
//! no terminal behaviour". The registry is data both front ends need, and `AGENTS.md` §2.4
//! says it is declared once. It was declared once, in the wrong crate.
//!
//! **What stays is the picker half, and it stays for one reason:** it needs
//! `epsilon_llm::catalog`, and `epsilon-contracts` has zero internal dependencies
//! (`AGENTS.md` §2). `slash_items` and `suggest_slash` stay for the same reason — they are
//! fuzzy ranking over `SLASH_ITEMS` and the ranking needs `crate::fuzzy`.
//!
//! Rendering lives in `app.rs`; everything testable without a terminal lives here.
//! Rows come from the bundled catalog (`epsilon_llm::catalog`) plus persisted
//! recents, session-fetched live listings, and a synthetic "custom id" row whenever
//! the filter text is not an exact known id — pi's escape hatch plus opencode's
//! favorites/recents convention, minus the parts that need a credential store.

// The re-export that makes the move a no-op for every caller. `epsilon-tui` already
// depended on `epsilon-contracts` directly, so this adds no Cargo edge anywhere.
pub use epsilon_contracts::command::{
    ALIAS_HEADS, COMMAND_HEADS, Command, SLASH_ITEMS, SlashItem, parse_command,
};

use epsilon_llm::catalog;
use epsilon_llm::custom::CustomProvider;

use crate::fuzzy;

/// One selectable line in the `/models` dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    /// Model id applied on selection (the filter text for the custom row).
    pub id: String,
    /// Display label (catalog label or the raw id).
    pub label: String,
    /// Provider grouping key ("recent"/"live"/"custom" for synthetic rows).
    pub provider: String,
    /// Credential env-var status for the auth-hint column.
    pub auth_ready: bool,
    /// True for the synthetic "use this exact id" row.
    pub is_custom: bool,
    /// True for rows fetched live from the provider API.
    pub is_live: bool,
    /// True for rows sourced from the persisted recently-used list.
    pub is_recent: bool,
    /// True when the id is starred in the persisted favorites list (M23).
    pub is_favorite: bool,
}

fn row_from_info(info: &epsilon_llm::ModelInfo) -> PickerRow {
    PickerRow {
        id: info.id.to_string(),
        label: info.label.to_string(),
        provider: info.provider.to_string(),
        // M22: stored credentials (auth.json) count, not just env vars.
        auth_ready: catalog::credentials_ready(info.id),
        is_custom: false,
        is_live: false,
        is_recent: false,
        is_favorite: false,
    }
}

fn synthetic_row(
    id: &str,
    provider: &str,
    is_custom: bool,
    is_live: bool,
    is_recent: bool,
) -> PickerRow {
    PickerRow {
        id: id.to_string(),
        label: catalog::display_label(id).to_string(),
        provider: provider.to_string(),
        auth_ready: catalog::credentials_ready(id),
        is_custom,
        is_live,
        is_recent,
        is_favorite: false,
    }
}

/// Build the picker rows for a filter string.
///
/// Order (empty filter): active model first, then FAVORITES, then recents, then
/// the bundled catalog (opencode's model-dialog section order). With a filter:
/// case-insensitive substring matches across id/label/provider from all
/// sources; when the filter is not an exact known id, a trailing custom row
/// offers it verbatim.
pub fn build_rows(
    filter: &str,
    current: &str,
    recents: &[String],
    live: &[(String, Vec<String>)],
) -> Vec<PickerRow> {
    build_rows_with(filter, current, recents, &[], live, &[])
}

/// [`build_rows`] with an explicit favorites list (M23; kept as a separate
/// function so existing call sites/tests keep their shape).
pub fn build_rows_with(
    filter: &str,
    current: &str,
    recents: &[String],
    favorites: &[String],
    live: &[(String, Vec<String>)],
    customs: &[CustomProvider],
) -> Vec<PickerRow> {
    let needle = filter.trim();
    // pi parity (tui/fuzzy.ts:fuzzyFilter): token-split + total-score ordering.
    // Collect candidates first so we can sort by fuzzy score (best first), then
    // emit in section order with per-section sort — preserves favorites/recents
    // grouping while ranking best matches top within each section.
    let total_score = |hay: &str| -> Option<i64> {
        if needle.is_empty() {
            return Some(0);
        }
        let tokens: Vec<&str> = needle
            .split(|c: char| c.is_whitespace() || c == '/')
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.is_empty() {
            return Some(0);
        }
        let mut total: i64 = 0;
        for tok in &tokens {
            let m = crate::fuzzy::fuzzy_match(tok, hay);
            if !m.matches {
                return None;
            }
            total += m.score;
        }
        Some(total)
    };

    // Helper: collect then sort by score; deduped via push_unique shape kept
    // in the per-section loops below (we score then emit).
    let mut rows: Vec<PickerRow> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    let push_unique = |row: PickerRow, rows: &mut Vec<PickerRow>, seen: &mut Vec<String>| {
        if !seen.iter().any(|id| id.eq_ignore_ascii_case(&row.id)) {
            seen.push(row.id.clone());
            rows.push(row);
        }
    };

    // Active model floats to top so it is always reachable/visible.
    if let Some(info) = catalog::lookup_model(current)
        && total_score(&format!("{} {} {}", info.id, info.label, info.provider)).is_some()
    {
        push_unique(row_from_info(info), &mut rows, &mut seen);
    }

    // Favorites (M23, opencode "Favorites" section): starred ids first — sorted by score.
    {
        let mut fav_cands: Vec<(i64, PickerRow)> = Vec::new();
        for id in favorites {
            if let Some(sc) = total_score(id) {
                if let Some(info) = catalog::lookup_model(id) {
                    let mut row = row_from_info(info);
                    row.is_favorite = true;
                    row.provider = "favorite".into();
                    fav_cands.push((sc, row));
                } else {
                    fav_cands.push((sc, synthetic_row(id, "favorite", false, false, false)));
                }
            }
        }
        fav_cands.sort_by_key(|(sc, _)| *sc);
        for (_, row) in fav_cands {
            push_unique(row, &mut rows, &mut seen);
        }
    }

    // Recents exclude favorites (opencode filters them out of "Recent") — sorted by score.
    {
        let mut rec_cands: Vec<(i64, PickerRow)> = Vec::new();
        for id in recents
            .iter()
            .filter(|id| !favorites.iter().any(|f| f.eq_ignore_ascii_case(id)))
        {
            if let Some(sc) = total_score(id) {
                if let Some(info) = catalog::lookup_model(id) {
                    let mut row = row_from_info(info);
                    row.is_recent = true;
                    row.provider = "recent".into();
                    rec_cands.push((sc, row));
                } else {
                    rec_cands.push((sc, synthetic_row(id, "recent", false, false, true)));
                }
            }
        }
        rec_cands.sort_by_key(|(sc, _)| *sc);
        for (_, row) in rec_cands {
            push_unique(row, &mut rows, &mut seen);
        }
    }

    // Live listings (M20): ids fetched from each provider's own API that are NOT
    // already in the bundled catalog surface under their provider's group.
    // Sorted by score so the best live hit surfaces first within the live block.
    {
        let mut live_cands: Vec<(i64, String, String)> = Vec::new();
        for (provider, ids) in live {
            for id in ids {
                if catalog::lookup_model(id).is_none()
                    && let Some(sc) = total_score(id)
                {
                    live_cands.push((sc, provider.clone(), id.clone()));
                }
            }
        }
        live_cands.sort_by_key(|(sc, _, _)| *sc);
        for (_, provider, id) in live_cands {
            push_unique(
                synthetic_row(&id, &provider, false, true, false),
                &mut rows,
                &mut seen,
            );
        }
    }

    {
        let mut cat_cands: Vec<(i64, PickerRow)> = Vec::new();
        for info in catalog::curated() {
            if let Some(sc) = total_score(&format!("{} {} {}", info.id, info.label, info.provider))
            {
                let mut row = row_from_info(info);
                row.is_favorite = favorites.iter().any(|f| f.eq_ignore_ascii_case(&row.id));
                cat_cands.push((sc, row));
            }
        }
        cat_cands.sort_by_key(|(sc, _)| *sc);
        for (_, row) in cat_cands {
            push_unique(row, &mut rows, &mut seen);
        }
    }

    // M28: declared custom providers surface one guidance row each, so a
    // configured endpoint is discoverable without reading docs. Selecting it
    // stages `<name>/` in the input for completing with the model name.
    {
        let mut cust_cands: Vec<(i64, String)> = Vec::new();
        for c in customs {
            let hint = format!("{}/", c.name);
            if let Some(sc) = total_score(&hint) {
                cust_cands.push((sc, hint));
            }
        }
        cust_cands.sort_by_key(|(sc, _)| *sc);
        for (_, hint) in cust_cands {
            push_unique(
                synthetic_row(&hint, "custom provider", true, false, false),
                &mut rows,
                &mut seen,
            );
        }
    }

    let trimmed = filter.trim();
    if !trimmed.is_empty() && !rows.iter().any(|r| r.id.eq_ignore_ascii_case(trimmed)) {
        rows.push(synthetic_row(trimmed, "custom", true, false, false));
    }
    rows
}

// ---- M17b: slash autocomplete with descriptions + selection (pi pattern) --------

/// Candidates that fuzzy-match the typed `/prefix` (input has no space yet),
/// sorted by match quality — pi's `fuzzyFilter` over `AutocompleteItem.name`.
pub fn slash_items(input: &str) -> Vec<&'static SlashItem> {
    if !input.starts_with('/') || input.contains(' ') {
        return Vec::new();
    }
    let needle = input.trim_start_matches('/').trim();
    if needle.is_empty() {
        return SLASH_ITEMS.iter().collect();
    }
    let mut scored: Vec<(i64, &'static SlashItem)> = SLASH_ITEMS
        .iter()
        .filter_map(|item| {
            let hay = item.value.trim_start_matches('/').trim();
            let m = crate::fuzzy::fuzzy_match(needle, hay);
            if m.matches {
                Some((m.score, item))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().map(|(_, it)| it).collect()
}

/// Best fuzzy guess for an unknown slash word (e.g. `/modl` → `/model`).
/// Returns the top-scoring [`SlashItem`] whose name fuzzy-matches `word`.
pub fn suggest_slash(word: &str) -> Option<&'static SlashItem> {
    let needle = word.trim().trim_start_matches('/').trim();
    if needle.is_empty() {
        return None;
    }
    let mut best: Option<(i64, &'static SlashItem)> = None;
    for item in SLASH_ITEMS {
        let hay = item.value.trim_start_matches('/').trim();
        let m = crate::fuzzy::fuzzy_match(needle, hay);
        if m.matches {
            match best {
                None => best = Some((m.score, item)),
                Some((best_score, _)) if m.score < best_score => best = Some((m.score, item)),
                _ => {}
            }
        }
    }
    best.map(|(_, it)| it)
}

// ---- M16c: two-stage picker (opencode dialog-provider → dialog-model) -----------

/// Provider priority for display ordering (opencode `PROVIDER_PRIORITY` parity —
/// popular providers surface first so first-run users see the obvious choices).
const PROVIDER_PRIORITY: &[(&str, usize)] = &[
    ("opencode", 0),
    ("opencode-go", 1),
    ("openai", 2),
    ("github-copilot", 3),
    ("anthropic", 4),
    ("google", 5),
];

fn provider_priority(id: &str) -> usize {
    PROVIDER_PRIORITY
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, p)| *p)
        .unwrap_or(99)
}

fn provider_description(id: &str) -> Option<&'static str> {
    match id {
        "opencode" => Some("(Recommended)"),
        "anthropic" => Some("(API key)"),
        "openai" => Some("(ChatGPT Plus/Pro or API key)"),
        "opencode-go" => Some("Low cost subscription for everyone"),
        _ => None,
    }
}

/// One provider entry in stage 1 of the model picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRow {
    /// Provider key (matches [`epsilon_llm::ModelInfo::provider`]).
    pub id: String,
    /// Number of catalog models under this provider.
    pub model_count: usize,
    /// True when any model of this provider has its credential env var set.
    pub auth_ready: bool,
    /// True when the ACTIVE model belongs to this provider.
    pub is_active: bool,
    /// Short hint shown dim beside the id (opencode's description column).
    pub description: Option<&'static str>,
    /// Section heading this row belongs under (opencode "Popular" vs "Providers").
    pub category: &'static str,
}

/// Stage-1 rows: one per distinct provider in the bundled catalog, ordered
/// Popular-first then auth-ready then alphabetical (opencode's model-dialog
/// section order). With M28 customs appended when they do not shadow a bundle.
pub fn build_provider_rows(current: &str, customs: &[CustomProvider]) -> Vec<ProviderRow> {
    let mut rows: Vec<ProviderRow> = Vec::new();
    for info in catalog::curated() {
        let active = current.starts_with(&format!("{}/", info.provider)) || info.id == current;
        let is_active_provider = catalog::lookup_model(current)
            .map(|m| m.provider == info.provider)
            .unwrap_or(false);
        match rows.iter_mut().find(|r| r.id == info.provider) {
            Some(row) => {
                row.model_count += 1;
                row.auth_ready |= catalog::credentials_ready(info.id);
                row.is_active |= is_active_provider || (active && info.id == current);
            }
            None => rows.push(ProviderRow {
                id: info.provider.to_string(),
                model_count: 1,
                auth_ready: catalog::credentials_ready(info.id),
                is_active: is_active_provider,
                description: provider_description(info.provider),
                category: if provider_priority(info.provider) < 99 {
                    "Popular"
                } else {
                    "Providers"
                },
            }),
        }
    }
    // Customs never shadow a bundled provider (routing guarantees the same);
    // auth_ready reflects whether a stored key or declared env var exists.
    for c in customs {
        if rows.iter().any(|r| r.id.eq_ignore_ascii_case(&c.name)) {
            continue;
        }
        let env_present =
            !c.env_key.is_empty() && std::env::var_os(&c.env_key).is_some_and(|v| !v.is_empty());
        rows.push(ProviderRow {
            id: c.name.clone(),
            model_count: 0, // endpoint-declared; model names come from the server
            auth_ready: env_present || epsilon_llm::auth::load_credentials().contains_key(&c.name),
            is_active: current.starts_with(&format!("{}/", c.name)),
            description: Some("Custom provider"),
            category: "Providers",
        });
    }
    rows.sort_by(|a, b| {
        provider_priority(&a.id)
            .cmp(&provider_priority(&b.id))
            .then(b.auth_ready.cmp(&a.auth_ready))
            .then(a.id.cmp(&b.id))
    });
    rows
}

/// The env var that authenticates `provider`, for display ("env: NAME").
///
/// Proven unique per provider across the bundled catalog, so the first match
/// is the rule rather than a guess; customs carry their own. `None` means
/// local (no variable exists) — never "unknown".
pub fn provider_env_key(provider: &str, customs: &[CustomProvider]) -> Option<String> {
    if let Some(c) = customs.iter().find(|c| c.name == provider) {
        return if c.env_key.is_empty() {
            None
        } else {
            Some(c.env_key.clone())
        };
    }
    catalog::curated()
        .iter()
        .find(|m| m.provider == provider)
        .and_then(|m| {
            if m.env_key.is_empty() {
                None
            } else {
                Some(m.env_key.to_string())
            }
        })
}

/// Stage-2 rows: the models of ONE provider, reusing [`build_rows`] ordering rules
/// via fuzzy filter across id + label. Live-listed ids for this provider that are
/// missing from the bundled catalog are merged on top (M20 auto-fetch).
/// M28: for a declared custom provider, one guidance row (`<name>/`) is offered —
/// select it, append the model name, Enter applies the full id.
pub fn build_models_for_provider(
    provider: &str,
    filter: &str,
    current: &str,
    live: &[(String, Vec<String>)],
    customs: &[CustomProvider],
) -> Vec<PickerRow> {
    let needle = filter.trim();
    if let Some(c) = customs.iter().find(|c| c.name == provider) {
        let hint = format!("{}/", c.name);
        let mut rows = vec![synthetic_row(&hint, "custom provider", true, false, false)];
        // A typed filter past the prefix becomes a directly applicable id row.
        let candidate = filter.trim();
        if !candidate.is_empty() && candidate.starts_with(&hint) && candidate.len() > hint.len() {
            rows.insert(0, synthetic_row(candidate, &c.name, true, false, false));
        }
        return rows;
    }
    // pi parity: score + sort like fuzzyFilter — best (lowest score) first.
    // When the filter is empty, preserve the "current floats to top" order;
    // when filtering, fuzzy ranking wins (pi's filterModels sets selectedIndex=0
    // on the scored list, not on a current-first list).
    let score_hay = |hay: &str| -> Option<i64> {
        if needle.is_empty() {
            return Some(0);
        }
        let tokens: Vec<&str> = needle
            .split(|c: char| c.is_whitespace() || c == '/')
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.is_empty() {
            return Some(0);
        }
        let mut total: i64 = 0;
        for tok in &tokens {
            let m = fuzzy::fuzzy_match(tok, hay);
            if !m.matches {
                return None;
            }
            total += m.score;
        }
        Some(total)
    };
    let mut scored: Vec<(i64, PickerRow)> = Vec::new();
    for info in catalog::curated().iter().filter(|i| i.provider == provider) {
        let hay = format!("{} {}", info.id, info.label);
        if let Some(sc) = score_hay(&hay) {
            scored.push((sc, row_from_info(info)));
        }
    }
    if let Some((_, ids)) = live.iter().find(|(p, _)| p == provider) {
        for id in ids {
            let known = scored.iter().any(|(_, r)| r.id.eq_ignore_ascii_case(id));
            if !known && let Some(sc) = score_hay(id) {
                scored.push((sc, synthetic_row(id, provider, false, true, false)));
            }
        }
    }
    if needle.is_empty() {
        // No filter: current floats to top (stable, pi's sortModels semantics).
        scored.sort_by(|a, b| {
            let a_cur = a.1.id == current;
            let b_cur = b.1.id == current;
            b_cur
                .cmp(&a_cur)
                .then(a.0.cmp(&b.0))
                .then(a.1.id.cmp(&b.1.id))
        });
    } else {
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
    }
    scored.into_iter().map(|(_, r)| r).collect()
}

// ---- M16c: command palette -----------------------------------------------------

/// One palette entry (opencode command-palette pattern): a named action with an
/// optional keybinding hint and slash alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    /// Display title.
    pub title: String,
    /// Command executed when selected (slash-style parse input).
    pub command: String,
    /// Keybinding this action runs, resolved through [`KeybindingsManager::key_text`].
    ///
    /// Held as data rather than used directly so a hint can only ever be a chord the
    /// key handler will actually accept. This column used to be `Option<&'static str>`
    /// holding hand-typed literals, which is how one of them came to be `"/login"` — a
    /// **slash command advertised as a keyboard shortcut** — beside `"Ctrl+M"`, which is
    /// not the spelling `key_label` prints.
    pub key_id: Option<&'static str>,
    /// The chord shown on the right, or `None` when the action has no binding.
    ///
    /// Resolved, never written: see [`Self::key_id`].
    pub binding: Option<String>,
}

/// All palette commands (slash commands + direct actions), in display order.
///
/// `kb` is read for every hint, so a user who rebinds a key sees the new chord here and
/// an entry whose action has no binding shows none — which is the only honest thing to
/// print, and what [`KeybindingsManager::key_text`] already promises when it returns
/// `None`.
pub fn palette_entries(kb: &crate::keybindings::KeybindingsManager) -> Vec<PaletteEntry> {
    vec![
        PaletteEntry {
            title: "Models — switch model".into(),
            command: "/models".into(),
            key_id: Some("app.model.select"),
            binding: None,
        },
        PaletteEntry {
            title: "Connect provider — set API key (paste, masked)".into(),
            command: "/login ".into(),
            // **No binding, and now it says so.** This used to advertise `Some("/login")`
            // in the key column: a slash command in the place a reader looks for a chord,
            // for an action that has no key at all. Pressing it did nothing.
            key_id: None,
            binding: None,
        },
        PaletteEntry {
            title: "Skills — list discovered skills".into(),
            command: "/skills".into(),
            key_id: None,
            binding: None,
        },
        PaletteEntry {
            title: "Clear transcript".into(),
            command: "/clear".into(),
            key_id: None,
            binding: None,
        },
        PaletteEntry {
            title: "Help — command reference".into(),
            command: "/help".into(),
            key_id: None,
            binding: None,
        },
        PaletteEntry {
            title: "Quit session".into(),
            command: "/quit".into(),
            key_id: None,
            binding: None,
        },
        PaletteEntry {
            title: "Toggle tool output expand".into(),
            // A real command. The old `\u{1}expand` was not a spelling this parser
            // accepts (it requires a leading `/`), so choosing this entry closed the
            // palette, cleared the composer, and ran nothing.
            command: "/expand".into(),
            // The chord is `ctrl+x`, not a bare `x`. A bare `x` is the binding the
            // editor deliberately removed because it made the letter untypable, and
            // the palette was advertising it anyway.
            key_id: Some("app.tools.expand"),
            binding: None,
        },
    ]
    .into_iter()
    .map(|mut e| {
        e.binding = e.key_id.and_then(|id| kb.key_text(id));
        e
    })
    .collect()
}

/// Fuzzy-filtered palette entries; empty query returns everything.
pub fn filter_palette(
    query: &str,
    kb: &crate::keybindings::KeybindingsManager,
) -> Vec<PaletteEntry> {
    let q = query.trim();
    if q.is_empty() {
        return palette_entries(kb);
    }
    let mut scored: Vec<(i64, PaletteEntry)> = palette_entries(kb)
        .into_iter()
        .filter_map(|e| {
            let m = fuzzy::fuzzy_match(q, &e.title);
            let m2 = fuzzy::fuzzy_match(q, &e.command);
            if m.matches || m2.matches {
                Some((m.score.min(m2.score), e))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by_key(|(s, _)| *s);
    scored.into_iter().map(|(_, e)| e).collect()
}

// ---- M57: session browser rows --------------------------------------------------

/// One row in the `/sessions` browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// File to resume.
    pub path: std::path::PathBuf,
    /// Display title (display name → first user message → file stem).
    pub title: String,
    /// Right-hand meta: age · size · what the title did NOT say.
    pub meta: String,
    /// True for the session this app is attached to right now.
    pub is_current: bool,
}

/// Compact byte size for the browser (`0 B`, `812 B`, `12 KB`, `1.4 MB`).
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{kb:.0} KB");
    }
    format!("{:.1} MB", kb / 1024.0)
}

/// Age label from two epoch-millis stamps (`just now`, `5m`, `3h`, `2d`).
/// Injected `now_ms` keeps this deterministic in tests.
pub fn age_label(now_ms: u64, then_ms: u64) -> String {
    if then_ms == 0 || now_ms <= then_ms {
        return "just now".to_string();
    }
    let secs = (now_ms - then_ms) / 1000;
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", hours / 24)
}

/// Rows for `/sessions`: newest first, fuzzy-filtered over title, file name and
/// the session's own cwd. `current` marks the attached session; `now_ms` is
/// injected for deterministic ages.
///
/// **`dir` is the sessions ROOT and `cwd` is the workspace.** Since M217 a launch writes into
/// `dir/--<cwd>--/` and sessions written before that are still in `dir` itself, so the list is
/// `list_sessions_for_cwd` — the same union `--continue` resolves through. Passing only `dir` would
/// show a user an empty browser in a workspace where every session is in the new subdirectory, and
/// passing only the subdirectory would hide every session they already had.
pub fn session_rows(
    dir: &std::path::Path,
    cwd: &std::path::Path,
    filter: &str,
    current: Option<&std::path::Path>,
    now_ms: u64,
) -> Vec<SessionRow> {
    let needle = filter.trim();
    let tokens: Vec<&str> = needle
        .split(|c: char| c.is_whitespace() || c == '/')
        .filter(|t| !t.is_empty())
        .collect();
    let mut scored: Vec<(i64, SessionRow)> = Vec::new();
    for session in epsilon_session::list_sessions_for_cwd(dir, cwd) {
        let title = session.title();
        let file = session
            .path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Meta shows what the title does not: the file name is redundant when
        // the title already IS the stem.
        let meta_head = format!(
            "{} · {}{}{}",
            age_label(now_ms, session.modified_ms),
            format_bytes(session.bytes),
            // M58: a delegated task's transcript is a session too — say so, or
            // the list looks like it grew files nobody asked for.
            if session.parent_session.is_some() {
                " · task child"
            } else {
                ""
            },
            // M61: a session another window is writing — resuming it will be
            // refused, so say so here rather than on the attempt.
            if session.in_use { " · in use" } else { "" }
        );
        // Title and file name match fuzzily; the session's cwd only matches
        // literally. Fuzzy subsequence over a long path is not a filter — `old`
        // is a subsequence of `…\AppData\Local\…` — and a browser that matches
        // everything is a browser that filters nothing.
        let hay = format!("{title} {file}");
        let cwd_lower = session.cwd.to_lowercase();
        let mut score: i64 = 0;
        let mut matched = true;
        for tok in &tokens {
            let m = crate::fuzzy::fuzzy_match(tok, &hay);
            if m.matches {
                score += m.score;
                continue;
            }
            if cwd_lower.contains(&tok.to_lowercase()) {
                continue;
            }
            matched = false;
            break;
        }
        if !matched {
            continue;
        }
        let is_current = current.is_some_and(|c| same_session_file(c, &session.path));
        scored.push((
            score,
            SessionRow {
                path: session.path.clone(),
                title,
                meta: format!("{meta_head} · {file}"),
                is_current,
            },
        ));
    }
    if !tokens.is_empty() {
        scored.sort_by_key(|(s, _)| *s);
    }
    scored.into_iter().map(|(_, r)| r).collect()
}

/// Same file, tolerating relative/absolute and `.`-segment differences between
/// what the app was handed and what the directory listing produced.
fn same_session_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_rows_list_newest_first_and_mark_the_current_file() {
        // M57: the browser reads the same directory `--continue` scans.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let header = |id: &str| {
            format!(
                "{}\n",
                serde_json::json!({
                    "type": "session", "version": 3, "id": id,
                    "timestamp": "2026-09-14T10:00:00Z", "cwd": dir.to_string_lossy(),
                })
            )
        };
        let user = |text: &str| {
            let entry = epsilon_contracts::session_entry::SessionEntry {
                id: "e1".to_string(),
                seq: 1,
                parent_id: None,
                timestamp_ms: 0,
                kind: epsilon_contracts::session_entry::EntryKind::Message {
                    message: epsilon_contracts::message::Message::User(
                        epsilon_contracts::message::UserMessage {
                            id: "m1".to_string(),
                            timestamp_ms: 0,
                            content: vec![epsilon_contracts::content::UserContent::Text {
                                text: text.to_string(),
                            }],
                        },
                    ),
                    api_request_sha256: None,
                },
            };
            format!("{}\n", serde_json::to_string(&entry).unwrap())
        };
        std::fs::write(
            dir.join("old.jsonl"),
            format!("{}{}", header("ses_old"), user("first")),
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(
            dir.join("new.jsonl"),
            format!("{}{}", header("ses_new"), user("second task")),
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();

        let now = 10_000_000_000u64;
        let rows = session_rows(dir, dir, "", None, now);
        assert_eq!(rows.len(), 2, "only session files: {rows:?}");
        assert_eq!(rows[0].title, "second task", "newest first");
        assert!(rows[0].meta.contains("new.jsonl"));
        assert!(!rows[0].is_current);

        // The current file is marked, and the filter matches title, file, or cwd.
        let rows = session_rows(dir, dir, "", Some(&dir.join("new.jsonl")), now);
        assert!(rows[0].is_current, "the attached session is marked");
        assert!(!rows[1].is_current);
        assert_eq!(session_rows(dir, dir, "old", None, now).len(), 1);
        assert_eq!(
            session_rows(dir, dir, "second", None, now).len(),
            1,
            "title matches"
        );
        assert!(session_rows(dir, dir, "zzz-nothing", None, now).is_empty());
    }

    #[test]
    fn session_rows_flag_delegated_task_children() {
        // M58: children carry a parent link in their header; the browser says so.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let parent = dir.join("tui.jsonl");
        let child = dir.join("tui-task_abc.jsonl");
        let _ = epsilon_session::SessionStore::open_in(&parent, dir).unwrap();
        let _ = epsilon_session::SessionStore::open_child_in(&child, dir, "ses_parent").unwrap();

        let rows = session_rows(dir, dir, "", None, 1_000_000);
        assert_eq!(rows.len(), 2);
        let child_row = rows.iter().find(|r| r.path == child).expect("child listed");
        let parent_row = rows
            .iter()
            .find(|r| r.path == parent)
            .expect("parent listed");
        assert!(child_row.meta.contains("task child"), "{}", child_row.meta);
        assert!(
            !parent_row.meta.contains("task child"),
            "{}",
            parent_row.meta
        );
    }

    #[test]
    fn a_leased_session_is_marked_in_use() {
        // M61: the row says why a resume would be refused, before the attempt.
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("ses_busy.jsonl");
        let _store = epsilon_session::SessionStore::open_in(&path, dir.path()).expect("writer");
        let rows = session_rows(dir.path(), dir.path(), "", None, 0);
        let row = rows.first().expect("the session is listed");
        assert!(row.meta.contains("in use"), "{}", row.meta);
    }

    #[test]
    fn age_and_size_labels_are_compact_and_deterministic() {
        let now = 1_000_000_000_000u64;
        assert_eq!(age_label(now, now), "just now");
        assert_eq!(age_label(now, now - 30_000), "just now");
        assert_eq!(age_label(now, now - 5 * 60_000), "5m ago");
        assert_eq!(age_label(now, now - 3 * 3_600_000), "3h ago");
        assert_eq!(age_label(now, now - 50 * 3_600_000), "2d ago");
        // Unknown mtime never renders as a huge age.
        assert_eq!(age_label(now, 0), "just now");

        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(812), "812 B");
        assert_eq!(format_bytes(12 * 1024), "12 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024 / 2), "1.5 MB");
    }

    #[test]
    fn empty_filter_lists_recents_then_catalog_with_current_first() {
        let rows = build_rows("", "faux-1", &["deepseek-chat".into()], &[]);
        assert_eq!(rows[0].id, "faux-1", "current floats above everything");
        assert_eq!(rows[1].id, "deepseek-chat");
        assert!(rows[1].is_recent && rows[1].provider == "recent");
        assert!(rows.iter().any(|r| r.provider == "ollama"));
        assert!(
            !rows
                .iter()
                .any(|r| r.id == "deepseek-chat" && r.provider != "recent")
        );
    }

    #[test]
    fn filter_matches_id_label_or_provider_case_insensitively() {
        // M55: the third query is a LABEL-only term — the registry retired
        // `deepseek-reasoner`, but `v4.1` appears in `deepseek-flash`'s label
        // ("DeepSeek V4.1 Flash") and nowhere in its id, so the label path and
        // the case-folding both stay covered.
        for q in ["deepseek", "DEEPSEEK", "v4.1"] {
            let rows = build_rows(q, "", &[], &[]);
            assert!(
                rows.iter().any(|r| r.provider == "deepseek"),
                "query {q} must find deepseek"
            );
        }
        assert!(
            build_rows("zzz-no-match-zzz", "", &[], &[])
                .iter()
                .all(|r| r.is_custom)
        );
    }

    #[test]
    fn non_exact_filter_offers_custom_row_verbatim() {
        let rows = build_rows("my-fine-tune-v3", "faux-1", &[], &[]);
        let custom = rows.iter().find(|r| r.is_custom).expect("custom row");
        assert_eq!(custom.id, "my-fine-tune-v3");
        assert!(
            build_rows("deepseek-v4-pro", "", &[], &[])
                .iter()
                .all(|r| !r.is_custom)
        );
    }

    #[test]
    fn live_rows_surface_only_unknown_ids_under_their_own_group() {
        let rows = build_rows(
            "",
            "",
            &[],
            &[(
                "deepseek".to_string(),
                vec![
                    "brand-new-model".to_string(),
                    "deepseek-v4-pro".to_string(), // known: deduped into the catalog section
                ],
            )],
        );
        let live = rows.iter().filter(|r| r.is_live).collect::<Vec<_>>();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, "brand-new-model");
        assert_eq!(live[0].provider, "deepseek", "grouped under its provider");
        assert!(rows.iter().any(|r| r.id == "deepseek-v4-pro" && !r.is_live));
    }

    /// M20: stage-2 merges live-listed ids for its provider on top of the
    /// bundled catalog rows (this is the path that makes fresh models appear).
    #[test]
    fn stage2_merges_live_ids_for_its_provider_only() {
        let live = vec![(
            "deepseek".to_string(),
            vec!["deepseek-v4-pro".to_string(), "brand-new-model".to_string()],
        )];
        let rows = build_models_for_provider("deepseek", "", "", &live, &[]);
        assert!(
            rows.iter().any(|r| r.id == "brand-new-model" && r.is_live),
            "unknown live id must surface in stage 2"
        );
        // Known id appears once, from the catalog (not duplicated as a live row).
        assert_eq!(
            rows.iter().filter(|r| r.id == "deepseek-v4-pro").count(),
            1,
            "no duplicate rows for known ids"
        );
        // A different provider's live listing must NOT leak in.
        let other = build_models_for_provider(
            "anthropic",
            "",
            "",
            &[("openai".to_string(), vec!["gpt-fake-live".to_string()])],
            &[],
        );
        assert!(
            !other.iter().any(|r| r.is_live),
            "stage 2 must not show other providers' live ids"
        );
    }

    #[test]
    fn custom_providers_surface_in_stage1_and_guide_stage2() {
        // M28: a declared endpoint appears as a descendable provider row, and
        // its stage 2 guides toward `<name>/<model>` completion.
        let customs =
            vec![CustomProvider::from_config("lmstudio", "http://localhost:1234/v1", "").unwrap()];

        let stage1 = build_provider_rows("faux-1", &customs);
        let lm = stage1.iter().find(|r| r.id == "lmstudio");
        assert!(lm.is_some(), "custom provider visible in stage 1");
        assert_eq!(lm.unwrap().model_count, 0);

        let stage2 = build_models_for_provider("lmstudio", "", "", &[], &customs);
        assert!(
            stage2
                .iter()
                .any(|r| r.id == "lmstudio/" && r.provider == "custom provider")
        );

        // Typing past the prefix offers the full id directly.
        let typed = build_models_for_provider("lmstudio", "lmstudio/qwen", "", &[], &customs);
        assert!(
            typed.iter().any(|r| r.id == "lmstudio/qwen"),
            "filter text becomes an applicable custom id row"
        );

        // Customs never shadow a bundled provider of the same name.
        let shadow = CustomProvider::from_config("deepseek", "http://x/v1", "").unwrap();
        let rows = build_provider_rows("", &[shadow]);
        assert_eq!(rows.iter().filter(|r| r.id == "deepseek").count(), 1);
    }

    #[test]
    fn build_rows_uses_fuzzy_subsequence_not_substring() {
        // pi parity: "dvp" is a fuzzy subsequence of "deepseek-v4-pro", not a
        // substring; stage1's build_rows_with must find it via fuzzy_match like
        // stage2 does. (Pin moved at the M55 refresh: the registry dropped the
        // old `deepseek-chat` row this case used.)
        let rows = build_rows("dvp", "", &[], &[]);
        assert!(
            rows.iter().any(|r| r.id == "deepseek-v4-pro"),
            "fuzzy subsequence dvp must find deepseek-v4-pro via fuzzy_match"
        );
        // "codex52" swapped still finds gpt-5.2-codex via swapped fallback parity
        let rows2 = build_rows("codex52", "", &[], &[]);
        assert!(
            rows2.iter().any(|r| r.id.contains("codex")),
            "swapped codex52 should match a codex model via pi swapped fallback"
        );
    }

    #[test]
    fn deterministic_order_same_filter_same_rows() {
        let a = build_rows("deep", "", &[], &[]);
        let b = build_rows("deep", "", &[], &[]);
        assert_eq!(a, b, "same filter must produce identical row order");

        let s1 = build_models_for_provider("deepseek", "pro", "", &[], &[]);
        let s2 = build_models_for_provider("deepseek", "pro", "", &[], &[]);
        assert_eq!(s1, s2, "stage2 deterministic too");
    }

    #[test]
    fn multi_token_filter_finds_split_words() {
        // pi parity: fuzzyFilter splits on space/slash — every token must match
        for q in ["deep pro", "deep/pro", "  deep   pro  "] {
            let rows = build_rows(q, "", &[], &[]);
            assert!(
                rows.iter().any(|r| r.id == "deepseek-v4-pro"),
                "multi-token {q:?} must find deepseek-v4-pro"
            );
        }
        // Stage2 also token-aware
        let s2 = build_models_for_provider("deepseek", "deep pro", "", &[], &[]);
        assert!(
            s2.iter().any(|r| r.id == "deepseek-v4-pro"),
            "stage2 multi-token must find deepseek-v4-pro"
        );
        // Non-matching second token must NOT match
        let miss = build_rows("deep zzz-nope", "", &[], &[]);
        assert!(
            miss.iter().all(|r| r.is_custom),
            "second token miss must yield only custom row"
        );
    }

    #[test]
    fn filtered_results_are_ranked_by_fuzzy_score_best_first() {
        // pi parity: fuzzyFilter sorts by totalScore. "deep pro" must put
        // deepseek-v4-pro before any other deepseek* that only matches "deep".
        let rows = build_rows("deep pro", "", &[], &[]);
        let pos_pro = rows
            .iter()
            .position(|r| r.id == "deepseek-v4-pro")
            .expect("must find deepseek-v4-pro");
        // Any deepseek row that doesn't contain "pro" must rank after it
        if let Some(pos_other) = rows
            .iter()
            .position(|r| r.provider == "deepseek" && !r.id.contains("pro"))
        {
            assert!(
                pos_pro < pos_other,
                "deepseek-v4-pro (pos {pos_pro}) must rank before non-pro deepseek row (pos {pos_other}) under fuzzy scoring"
            );
        }
        // Empty filter still deterministic (already covered), but stage2 with filter
        // must also be scored — "pro" within deepseek provider should win.
        let s2 = build_models_for_provider("deepseek", "pro", "", &[], &[]);
        assert_eq!(
            s2.first().map(|r| r.id.as_str()),
            Some("deepseek-v4-pro"),
            "stage2 ranking: pro top"
        );
        // Empty stage2 still puts current first (stable when no query).
        let s2_empty_a = build_models_for_provider("deepseek", "", "deepseek-v4-pro", &[], &[]);
        let s2_empty_b = build_models_for_provider("deepseek", "", "deepseek-v4-pro", &[], &[]);
        assert_eq!(s2_empty_a, s2_empty_b);
        assert_eq!(
            s2_empty_a.first().map(|r| r.id.as_str()),
            Some("deepseek-v4-pro")
        );
    }

    #[test]
    fn slash_items_uses_fuzzy_not_only_prefix() {
        // pi parity: slash autocomplete uses fuzzyFilter over command names, not starts_with
        assert!(
            slash_items("/modl")
                .iter()
                .any(|it| it.value.trim() == "/model"),
            "/modl typo must still surface /model via fuzzy"
        );
        assert!(
            slash_items("/sesn")
                .iter()
                .any(|it| it.value.trim() == "/session"),
            "/sesn typo must surface /session via fuzzy"
        );
        // exact prefix still ranks first/best
        let first = slash_items("/mod").first().expect("/mod matches").value;
        assert!(first.trim_start_matches('/').starts_with("mod"));
    }

    #[test]
    fn suggest_slash_finds_best_fuzzy_hint_for_unknown() {
        let hit = suggest_slash("modl").expect("modl suggests something");
        assert!(
            matches!(hit.value.trim(), "/model" | "/models"),
            "modl should hint /model or /models, got {}",
            hit.value
        );
        assert!(suggest_slash("").is_none());
        assert!(suggest_slash("zzzzz-nope-zzz").is_none());
    }
}
