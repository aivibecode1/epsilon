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
//! The removed desktop window (ADR-039) used to read `parse_command` and
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

use crate::app::LiveCacheEntry;
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
    /// True when the id is the saved default model (P-249): ordered second,
    /// marked `· default`.
    pub is_default: bool,
    /// True when the id is not in the catalog and was not live-listed (P-260).
    ///
    /// Config stores raw unvalidated ids, so a model saved once and later
    /// retired from the catalog is a real state — and the reader deserves to
    /// *see* what is configured rather than have it silently dropped. Pi keeps
    /// the row, strikes its id, and badges it ` [unavailable]`
    /// (`scoped-models-selector.ts:252-257`). A struck row here is
    /// `Modifier::CROSSED_OUT`, which the terminal draw already uses elsewhere
    /// (`markdown.rs`), and the badge is a word rather than a glyph so it
    /// survives `NO_COLOR` (`AGENTS.md` §8).
    ///
    /// Only a **bare or self-prefixed** unknown id carries this: a
    /// `groq/foo-x` cannot be attributed to `deepseek`, because its provider
    /// prefix no longer identifies a live provider.
    pub is_unavailable: bool,
    /// True when the id is in [`catalog::RETIRED_IDS`]: dropped from the
    /// bundle as deprecated (criterion 6). A retired row stays struck like
    /// every unavailable one, but badges `retired` rather than
    /// `unavailable` — "we used to list this" is a different fact from
    /// "we never heard of this", and only the first tells the reader their
    /// saved model aged out.
    pub is_retired: bool,
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
        is_default: false,
        is_unavailable: false,
        is_retired: false,
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
        is_default: false,
        is_unavailable: false,
        is_retired: false,
    }
}

/// Mark a synthetic row that the runtime cannot route.
///
/// Every source of ids — favourites, recents, a live `/models` page, a typed
/// filter — can produce an id with no catalog row. Drawing those as ordinary
/// selectable rows is how the picker came to write a model that made the next
/// launch fail. A declared custom provider is the one exception: its prefix is a
/// real route even though no catalog row exists.
fn mark_unavailable(row: &mut PickerRow, customs: &[CustomProvider]) {
    if catalog::lookup_model(&row.id).is_some()
        || customs.iter().any(|custom| custom.matches_id(&row.id))
    {
        return;
    }
    row.is_unavailable = true;
    row.is_retired = catalog::is_retired_model(&row.id, &row.provider);
}

/// Whether an unknown id can honestly be shown under `provider`.
///
/// Bare ids can: a catalog entry with no `/` is this provider's, which is how
/// `deepseek-chat` reads. Ids prefixed with this provider's own name can:
/// `deepseek/deepseek-chat` is the same row spelled the long way. Anything else
/// — `groq/foo-x` under `deepseek`, or a bare id belonging to a *different*
/// provider — cannot, because the prefix is the only evidence there is and it
/// points elsewhere.
///
/// Without this, an unknown id would surface under every provider it was
/// searched with, and the reader would be shown a model attributed to a
/// provider that never published it.
fn id_belongs_to_provider(id: &str, provider: &str) -> bool {
    match id.split_once('/') {
        None => true,
        Some((prefix, _)) => prefix.eq_ignore_ascii_case(provider),
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
    live: &[LiveCacheEntry],
) -> Vec<PickerRow> {
    build_rows_with(filter, current, recents, &[], None, live, &[])
}

/// [`build_rows`] with an explicit favorites list (M23; kept as a separate
/// function so existing call sites/tests keep their shape).
pub fn build_rows_with(
    filter: &str,
    current: &str,
    recents: &[String],
    favorites: &[String],
    default: Option<&str>,
    live: &[LiveCacheEntry],
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

    // P-250: pi's `/model` selector searches a **provider-first** haystack and
    // states the reason in its own doc comment — *"keep the bare model ID out of
    // the leading position"* so a provider-prefixed query ranks above proxy ids
    // like `openrouter/openai/gpt-5`
    // (`references/pi-1.3.0/packages/coding-agent/src/modes/interactive/model-search.ts:13-21`).
    // Its scoped overlay uses the opposite order deliberately (`:7-11`), which is
    // why the reference has two functions; Epsilon has one picker over one
    // haystack, so it takes the selector's order.
    //
    // The defect this fixes was measured, not reasoned: with an id-first haystack
    // the query `openai` ranks `groq`'s `openai/gpt-oss-120b` above every OpenAI
    // row, because a proxy provider names its upstream **inside its id**. The
    // reader asked about a provider and was shown another one's models first.
    //
    // Each term appears once, and that is a measured constraint rather than a
    // preference: a longer haystack gives fuzzy subsequence matching more
    // subsequences to find, so repeating a term widens what a miss test matches.
    let hay = |info: &epsilon_llm::ModelInfo| {
        format!(
            "{} {}/{} {} {}",
            info.provider, info.provider, info.id, info.id, info.label
        )
    };

    let push_unique = |row: PickerRow, rows: &mut Vec<PickerRow>, seen: &mut Vec<String>| {
        if !seen.iter().any(|id| id.eq_ignore_ascii_case(&row.id)) {
            seen.push(row.id.clone());
            rows.push(row);
        }
    };

    // Active model floats to top so it is always reachable/visible.
    // It carries the same flags as every other row: without them a staged
    // current model shows unchecked in the `/scoped-models` checklist — the
    // favorites section dedupes it away (`push_unique`), so no row is left
    // to carry the mark (read on a real frame, not reasoned).
    if let Some(info) = catalog::lookup_model(current)
        && total_score(&hay(info)).is_some()
    {
        let mut row = row_from_info(info);
        row.is_favorite = favorites.iter().any(|f| f.eq_ignore_ascii_case(&row.id));
        row.is_default = default.is_some_and(|d| d.eq_ignore_ascii_case(&row.id));
        push_unique(row, &mut rows, &mut seen);
    }

    // Favorites (M23, opencode "Favorites" section): starred ids first — sorted by score.
    {
        let mut fav_cands: Vec<(i64, PickerRow)> = Vec::new();
        for id in favorites {
            if let Some(sc) = total_score(id) {
                if let Some(info) = catalog::lookup_model(id) {
                    let mut row = row_from_info(info);
                    row.is_favorite = true;
                    row.is_default = default.is_some_and(|d| d.eq_ignore_ascii_case(&row.id));
                    row.provider = "favorite".into();
                    fav_cands.push((sc, row));
                } else {
                    let mut row = synthetic_row(id, "favorite", false, false, false);
                    mark_unavailable(&mut row, customs);
                    fav_cands.push((sc, row));
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
                    let mut row = synthetic_row(id, "recent", false, false, true);
                    mark_unavailable(&mut row, customs);
                    rec_cands.push((sc, row));
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
        for entry in live {
            for id in &entry.ids {
                if catalog::lookup_model(id).is_none()
                    && let Some(sc) = total_score(id)
                {
                    live_cands.push((sc, entry.provider.clone(), id.clone()));
                }
            }
        }
        live_cands.sort_by_key(|(sc, _, _)| *sc);
        for (_, provider, id) in live_cands {
            let mut row = synthetic_row(&id, &provider, false, true, false);
            mark_unavailable(&mut row, customs);
            push_unique(row, &mut rows, &mut seen);
        }
    }

    {
        let mut cat_cands: Vec<(i64, PickerRow)> = Vec::new();
        for info in catalog::curated() {
            if let Some(sc) = total_score(&hay(info)) {
                let mut row = row_from_info(info);
                row.is_favorite = favorites.iter().any(|f| f.eq_ignore_ascii_case(&row.id));
                row.is_default = default.is_some_and(|d| d.eq_ignore_ascii_case(&row.id));
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

/// One merged slash-popup row: a built-in head, a `/skill <name>` entry, or
/// a `/template-name` entry. Owned because skills and templates are
/// discovered per workspace — a `&'static` row for each would need a leak
/// per discovery, which this repository reverted on sight (round 11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCandidate {
    /// Text inserted when the row is chosen.
    pub value: String,
    /// Description rendered dim beside the label.
    pub description: String,
}

/// Candidates that fuzzy-match the typed `/prefix` (input has no space yet),
/// merging the static registry with per-workspace `extra` rows. A later row
/// replaces an earlier row with the same value, so a caller that lists
/// templates after built-ins lets a shadowing template carry its own
/// description — the same precedence the expander gives it.
pub fn slash_candidates(input: &str, extra: &[(String, String)]) -> Vec<SlashCandidate> {
    if !input.starts_with('/') || input.contains(' ') {
        return Vec::new();
    }
    let needle = input.trim_start_matches('/').trim();
    let mut rows: Vec<SlashCandidate> = SLASH_ITEMS
        .iter()
        .map(|item| SlashCandidate {
            value: item.value.to_string(),
            description: item.description.to_string(),
        })
        .collect();
    for (value, description) in extra {
        if let Some(pos) = rows.iter().position(|row| row.value == *value) {
            rows[pos].description = description.clone();
        } else {
            rows.push(SlashCandidate {
                value: value.clone(),
                description: description.clone(),
            });
        }
    }
    if needle.is_empty() {
        return rows;
    }
    rank_candidates(needle, rows)
}

/// Levels matching a picker filter query, in ladder order (P-048).
/// The think picker (keys) and its draw read this one function, the way
/// the provider picker shares `build_provider_rows` — an empty query lists
/// all seven; otherwise a level matches when its name holds the query.
pub fn match_think_levels(query: &str) -> Vec<epsilon_contracts::ThinkingLevel> {
    let query = query.trim().to_ascii_lowercase();
    epsilon_contracts::ThinkingLevel::ALL
        .into_iter()
        .filter(|level| query.is_empty() || level.as_str().contains(query.as_str()))
        .collect()
}

/// Fuzzy-rank owned popup rows against a needle, best first; an empty
/// needle keeps insertion order. Shared by the head-mode and argument-mode
/// popups so both rank the same way.
pub fn rank_candidates(needle: &str, rows: Vec<SlashCandidate>) -> Vec<SlashCandidate> {
    rank_candidates_by(needle, rows, |row| row.value.trim_start_matches('/').trim())
}

/// [`rank_candidates`] with the haystack spelled out: the argument popup
/// ranks the fragment against the argument alone, not the whole
/// `"/head arg"` value — every row shares the head, so ranking the full
/// value ties on the head's own letters and the order degrades to
/// insertion order (Tab then applies whichever the caller listed first).
pub fn rank_candidates_by(
    needle: &str,
    rows: Vec<SlashCandidate>,
    hay: impl for<'a> Fn(&'a SlashCandidate) -> &'a str,
) -> Vec<SlashCandidate> {
    let needle = needle.trim();
    if needle.is_empty() {
        return rows;
    }
    let mut scored: Vec<(i64, SlashCandidate)> = rows
        .into_iter()
        .filter_map(|row| {
            let hay = hay(&row);
            let m = crate::fuzzy::fuzzy_match(needle, hay);
            m.matches.then_some((m.score, row))
        })
        .collect();
    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().map(|(_, row)| row).collect()
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

/// What a successful `/login` save implies for the running model (P-063).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostLoginAction {
    /// The current model already runs on a credential — save notice only.
    KeepCurrent,
    /// The current model cannot answer — switch to this bundled model id.
    Select(String),
    /// The provider has no bundled rows — save notice plus `/model` guidance.
    NoBundledModels,
}

/// Pi selects the provider's default model after login, but only from an
/// unknown model (`completeProviderAuthentication`'s `isUnknownModel` gate).
/// The equivalent here: a `faux` fallback id or a model with no credential
/// cannot answer, so the just-saved provider's first bundled row takes over
/// (`catalog::first_model_for_provider` — picker order, never a guess); a
/// running model is never yanked out from under its session.
pub fn post_login_action(current_model_id: &str, provider: &str) -> PostLoginAction {
    if !current_model_id.starts_with("faux") && catalog::credentials_ready(current_model_id) {
        return PostLoginAction::KeepCurrent;
    }
    match catalog::first_model_for_provider(provider) {
        Some(info) => PostLoginAction::Select(info.id.to_string()),
        None => PostLoginAction::NoBundledModels,
    }
}

/// Whether a just-saved key needs the subscription-billing warning (P-061:
/// pi's `isAnthropicSubscriptionAuthKey` — an `sk-ant-oat` infix — scoped to
/// the provider it prices).
pub fn needs_subscription_warning(provider: &str, key: &str) -> bool {
    provider.eq_ignore_ascii_case("anthropic")
        && epsilon_llm::providers::anthropic::is_oauth_token(key)
}

/// P-061: the once-per-session subscription-billing warning. Pi's words minus
/// the settings opt-out this tree has no toggle for — stated, not silent.
pub fn subscription_billing_warning() -> &'static str {
    "Anthropic subscription auth is active \u{2014} harness use draws from extra usage and is billed per token, not your Claude plan limits"
}

/// Stage-2 rows: the models of ONE provider, reusing [`build_rows`] ordering rules
/// via fuzzy filter across id + label. Live-listed ids for this provider that are
/// missing from the bundled catalog are merged on top (M20 auto-fetch).
/// M28: for a declared custom provider, one guidance row (`<name>/`) is offered —
/// select it, append the model name, Enter applies the full id.
///
/// `favorites` marks starred rows and floats them under the current model
/// (empty: no marking, and the comparator falls through to the old order —
/// so every existing caller passing `&[]` sees byte-identical rows).
pub fn build_models_for_provider(
    provider: &str,
    filter: &str,
    current: &str,
    favorites: &[String],
    default: Option<&str>,
    live: &[LiveCacheEntry],
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
        // P-250: provider-first, as in `build_rows` — and here it matters twice
        // over, because this haystack used to be `id label` with the provider
        // missing entirely, so filtering stage 2 on its own provider's name
        // matched nothing and drew "no matches — Backspace to widen". Each term
        // once, for the reason `build_rows`'s `hay` gives.
        let hay = format!(
            "{} {}/{} {} {}",
            info.provider, info.provider, info.id, info.id, info.label
        );
        if let Some(sc) = score_hay(&hay) {
            scored.push((sc, row_from_info(info)));
        }
    }
    if let Some(entry) = live.iter().find(|e| e.provider == provider) {
        for id in &entry.ids {
            let known = scored.iter().any(|(_, r)| r.id.eq_ignore_ascii_case(id));
            if !known && let Some(sc) = score_hay(id) {
                let mut row = synthetic_row(id, provider, false, true, false);
                mark_unavailable(&mut row, customs);
                scored.push((sc, row));
            }
        }
    }
    // P-260: a configured id the catalog does not know stays offered, marked
    // unavailable. This is the row a reader most needs when it appears — the
    // model that is actually configured is one the picker used to hide, so the
    // running state was invisible in the surface that changes it.
    //
    // Two guards, both cheap and both necessary. `push_unique` dedupes
    // case-insensitively, so a synthesized row can never shadow a catalog or
    // live one. And the id must be bare or prefixed with *this* provider: a
    // `groq/foo-x` is not attributable to `deepseek`, and inventing an
    // attribution would show another provider's model under the wrong name.
    for id in [Some(current), default.unwrap_or_default().into()]
        .into_iter()
        .flatten()
        .filter(|id| !id.is_empty())
    {
        if catalog::lookup_model(id).is_none()
            && !scored.iter().any(|(_, r)| r.id.eq_ignore_ascii_case(id))
            && id_belongs_to_provider(id, provider)
        {
            let mut row = synthetic_row(id, provider, false, false, false);
            row.is_unavailable = true;
            // Criterion 6: a bundle-retired id stays struck (it cannot be
            // selected) but badges `retired` — the reader's saved model aged
            // out, which is a different fact from "never heard of it".
            // Spelling-tolerant: the configured id and the retired entry may
            // each be bare or provider-scoped.
            row.is_retired = catalog::is_retired_model(id, provider);
            scored.push((i64::MAX, row));
        }
    }
    // Starred rows carry the flag the pickers draw, whichever front end
    // asked. Empty favorites: no flag ever set, and the comparator below
    // falls through to the old order — byte-identical rows. The default
    // rides the same loop (one pass, two flags).
    for (_, row) in scored.iter_mut() {
        row.is_favorite = favorites.iter().any(|f| f.eq_ignore_ascii_case(&row.id));
        row.is_default = default.is_some_and(|d| d.eq_ignore_ascii_case(&row.id));
    }
    if needle.is_empty() {
        // No filter: current floats to top, then the default, then
        // favorites (stable, pi's sortModels semantics with the starred
        // set next). `None` default: the middle term never fires, and the
        // order is the old one — byte-identical rows.
        //
        // **Unavailable rows sort last, and this is a measured constraint, not
        // a preference.** The first version let the comparator's existing terms
        // place them, and a synthesized row for the *current* model then floated
        // `current` to the top — above a starred model, which broke
        // `starred_models_float_first_and_read_as_favorites` in the desktop
        // suite. An unavailable row is a **report** that something is
        // configured, not a candidate to select: it must never displace a model
        // that can actually be applied.
        scored.sort_by(|a, b| {
            let a_un = a.1.is_unavailable;
            let b_un = b.1.is_unavailable;
            let a_cur = a.1.id == current;
            let b_cur = b.1.id == current;
            let a_def = a.1.is_default;
            let b_def = b.1.is_default;
            let a_fav = a.1.is_favorite;
            let b_fav = b.1.is_favorite;
            a_un.cmp(&b_un)
                .then(b_cur.cmp(&a_cur))
                .then(b_def.cmp(&a_def))
                .then(b_fav.cmp(&a_fav))
                .then(a.0.cmp(&b.0))
                .then(a.1.id.cmp(&b.1.id))
        });
    } else {
        // Under a filter, score wins — and an unavailable row still sorts last,
        // because `i64::MAX` alone would put it behind every match but a filter
        // that matches nothing but the unavailable id would promote it above a
        // real match's neighbours. Same rule, both branches.
        scored.sort_by(|a, b| {
            a.1.is_unavailable
                .cmp(&b.1.is_unavailable)
                .then(a.0.cmp(&b.0))
                .then(a.1.id.cmp(&b.1.id))
        });
    }
    scored.into_iter().map(|(_, r)| r).collect()
}

/// The badge word for a picker row (criterion 6): `retired` for ids the
/// bundle dropped as deprecated, `unavailable` for any other unknown id,
/// nothing for a row that can actually be applied.
///
/// A word rather than a glyph so it survives `NO_COLOR` (`AGENTS.md` §8) —
/// the same reason the draw strikes the row beside it.
pub fn model_badge(row: &PickerRow) -> Option<&'static str> {
    if row.is_retired {
        Some("[retired]")
    } else if row.is_unavailable {
        Some("[unavailable]")
    } else {
        None
    }
}

/// One live-status line for `provider` (criterion 6), shared by the picker
/// footer and `/catalog` so the two cannot disagree:
///
/// - `live: deepseek (12 models · refreshed 5m ago (this session))` — fetched this session;
/// - `live: deepseek (12 models · from startup cache · 2h ago · auto-refreshes on open)` — loaded at startup;
/// - `live: deepseek (not fetched yet · opening refreshes)` — nothing cached yet.
///
/// The age is [`age_label`]: std-only formatting, deterministic in tests via
/// the injected `now_ms`. `age_label` already carries `ago`/`just now`, so the
/// verbs reuse it verbatim rather than adding a second `ago`.
///
/// The separator is `·`, the row's own separator everywhere else in the UI:
/// `,` and `—` here were a second notation for the same join, and a second
/// notation is how two screens disagree about one fact.
pub fn live_status(provider: &str, entry: Option<&LiveCacheEntry>, now_ms: u64) -> String {
    match entry {
        None => format!("live: {provider} (not fetched yet · opening refreshes)"),
        Some(e) => {
            let age = age_label(now_ms, e.fetched_at_ms);
            let n = e.ids.len();
            let noun = if n == 1 { "model" } else { "models" };
            if e.from_cache {
                format!(
                    "live: {provider} ({n} {noun} · from startup cache · {age} · auto-refreshes on open)"
                )
            } else {
                format!("live: {provider} ({n} {noun} · refreshed {age} (this session))")
            }
        }
    }
}

// ---- M16c: command palette -----------------------------------------------------

/// One palette entry (opencode command-palette pattern): a named action with an
/// optional keybinding hint and slash alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    /// Display title.
    pub title: String,
    /// Command executed when selected (slash-style parse input).
    ///
    /// Empty for an entry that carries a direct [`Self::action`] instead: there
    /// is no command to parse, and inventing a spelling the parser would refuse
    /// is how a palette entry becomes a silent no-op (M194).
    pub command: String,
    /// Keybinding this action runs, resolved through `KeybindingsManager::key_text`.
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
    /// What choosing this entry does, when it is not a slash command.
    ///
    /// The workspace file tree is the case: `app.tree.toggle` is a chord in the
    /// terminal table with **no command head** (`/tree` is the *session* tree, a
    /// different feature), so a palette entry that names it needs one action the
    /// palette can run directly. It is the same call F6 makes — the entry names
    /// the key rather than being a second way in.
    pub action: Option<PaletteAction>,
}

/// An action a palette entry runs directly, with no slash command behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteAction {
    /// Open or close the workspace file tree (`app.tree.toggle`, F6).
    ToggleFileTree,
}

impl PaletteEntry {
    /// The label column: the command value, or `Files` for the direct tree action.
    ///
    /// Split from the description so the row can budget the two separately: a
    /// combined title lets a long description push the label off the row with
    /// no ellipsis, which reads as complete when it was cut.
    pub fn label(&self) -> &str {
        if self.command.is_empty() {
            // The direct tree action: the title leads with `Files`.
            "Files"
        } else {
            self.command.trim()
        }
    }

    /// The description column: the registry words after the `·`, without the label.
    pub fn description(&self) -> &str {
        self.title
            .split_once('·')
            .map(|(_, rest)| rest.trim())
            .unwrap_or("")
    }
}

/// All palette commands (slash commands + direct actions), in display order.
///
/// Derived from [`COMMAND_HEADS`] — one row per head, with the display name
/// and description taken from the [`SLASH_ITEMS`] row the slash popup reads,
/// so the two surfaces cannot drift apart (AGENTS.md §2.4: palette entries
/// are derived from the registry, not a second hand-maintained list). A head
/// with no `SLASH_ITEMS` row fails loud here rather than shipping a row the
/// popup never names. The trailing workspace-tree row keeps its direct action:
/// it has no command head (`/tree` is the *session* tree, a different
/// feature), and inventing a spelling the parser would refuse is what would
/// make choosing it a silent no-op.
///
/// `kb` is read for every hint, so a user who rebinds a key sees the new chord here and
/// an entry whose action has no binding shows none — which is the only honest thing to
/// print, and what ``KeybindingsManager::key_text`` already promises when it returns
/// `None`.
pub fn palette_entries(kb: &crate::keybindings::KeybindingsManager) -> Vec<PaletteEntry> {
    let mut entries: Vec<PaletteEntry> = COMMAND_HEADS
        .iter()
        .map(|head| {
            let item = SLASH_ITEMS
                .iter()
                .find(|item| item.value.trim() == format!("/{head}"))
                .unwrap_or_else(|| {
                    panic!(
                        "COMMAND_HEADS entry /{head} has no SLASH_ITEMS row, \
                         so the palette cannot name it the way the popup does"
                    )
                });
            PaletteEntry {
                // Words from the registry, punctuation from the repertoire.
                // Six descriptions carry an em dash, which is not in CP437 —
                // a literal here drew as a box on a legacy console (the M219
                // shape; `transcript::welcome` takes its dash from the glyph
                // table for the same reason, and the ASCII golden gate fails
                // any frame that draws one). The middle dot is in the CP437
                // repertoire, so the title keeps every word and changes only
                // the punctuation a legacy console cannot draw.
                title: format!(
                    "{} · {}",
                    item.value.trim(),
                    item.description.replace('—', "·")
                ),
                command: item.value.to_string(),
                key_id: palette_key_id(head),
                binding: None,
                action: None,
            }
        })
        .collect();
    entries.push(PaletteEntry {
        // **Invariant 7, in both directions.** F6 has opened the workspace
        // tree since Phase 3 and no screen named it: not the first screen,
        // not the palette, not the footer, not `/help`. A key nobody can
        // find is a key nobody presses, and the entry below is the fix —
        // the chord is resolved from the table, so a rebinding moves the
        // hint with it.
        title: "Files · workspace tree".into(),
        // No command head: `/tree` is the *session* tree, and pointing at
        // it would run a different feature than the entry names.
        command: String::new(),
        key_id: Some("app.tree.toggle"),
        binding: None,
        action: Some(PaletteAction::ToggleFileTree),
    });
    entries
        .into_iter()
        .map(|mut e| {
            e.binding = e.key_id.and_then(|id| kb.key_text(id));
            e
        })
        .collect()
}

/// Keybinding hints for the derived palette rows that have bindings.
///
/// Hint data only, never a command list: every head named here is a row the
/// derivation above already produced, so this can neither add nor drop a
/// command — it only says which chord the key column prints, resolved through
/// the table rather than typed (which is how `/login` once sat in the key
/// column for an action that has no key at all).
fn palette_key_id(head: &str) -> Option<&'static str> {
    match head {
        // Ctrl+M opens the model picker, which is what a bare `/model` runs.
        "model" => Some("app.model.select"),
        // Ctrl+X expands tool output, which is what `/expand` toggles.
        "expand" => Some("app.tools.expand"),
        _ => None,
    }
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
    if then_ms == 0 {
        return "just now".to_string();
    }
    // A future stamp is clock skew or a hand-edited cache, not a fresh fetch.
    // The refresh rule already treats it as stale; the label must not say
    // "just now" about the one state that is about to be refetched.
    if then_ms > now_ms {
        return "timestamp in the future".to_string();
    }
    let secs = now_ms.saturating_sub(then_ms) / 1000;
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
    scope: epsilon_session::SessionScope,
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
    let sessions = match scope {
        epsilon_session::SessionScope::Workspace => {
            epsilon_session::list_sessions_for_cwd(dir, cwd)
        }
        epsilon_session::SessionScope::All => epsilon_session::list_sessions_all(dir),
    };
    for session in sessions {
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
        // All-scope rows name the workspace they belong to (P-230): without
        // it, sessions from elsewhere are indistinguishable, which is the
        // whole reason to leave the workspace scope.
        let ws = if scope == epsilon_session::SessionScope::All && !session.cwd.is_empty() {
            format!(" \u{b7} {}", workspace_label(&session.cwd))
        } else {
            String::new()
        };
        scored.push((
            score,
            SessionRow {
                path: session.path.clone(),
                title,
                meta: format!("{meta_head}{ws} · {file}"),
                is_current,
            },
        ));
    }
    if !tokens.is_empty() {
        scored.sort_by_key(|(s, _)| *s);
    }
    scored.into_iter().map(|(_, r)| r).collect()
}

/// Last path segment of a session's recorded cwd for all-scope row labels.
/// Falls back to the raw string when it has no file name (a filesystem root).
fn workspace_label(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string())
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
        let rows = session_rows(
            dir,
            dir,
            epsilon_session::SessionScope::Workspace,
            "",
            None,
            now,
        );
        assert_eq!(rows.len(), 2, "only session files: {rows:?}");
        assert_eq!(rows[0].title, "second task", "newest first");
        assert!(rows[0].meta.contains("new.jsonl"));
        assert!(!rows[0].is_current);

        // The current file is marked, and the filter matches title, file, or cwd.
        let rows = session_rows(
            dir,
            dir,
            epsilon_session::SessionScope::Workspace,
            "",
            Some(&dir.join("new.jsonl")),
            now,
        );
        assert!(rows[0].is_current, "the attached session is marked");
        assert!(!rows[1].is_current);
        assert_eq!(
            session_rows(
                dir,
                dir,
                epsilon_session::SessionScope::Workspace,
                "old",
                None,
                now
            )
            .len(),
            1
        );
        assert_eq!(
            session_rows(
                dir,
                dir,
                epsilon_session::SessionScope::Workspace,
                "second",
                None,
                now
            )
            .len(),
            1,
            "title matches"
        );
        assert!(
            session_rows(
                dir,
                dir,
                epsilon_session::SessionScope::Workspace,
                "zzz-nothing",
                None,
                now
            )
            .is_empty()
        );
    }

    #[test]
    fn post_login_selects_a_runnable_model_only_from_an_unrunnable_one() {
        // P-063: pi's `isUnknownModel` gate — a running model is never yanked.
        assert_eq!(
            post_login_action("llama3.1:8b", "anthropic"),
            PostLoginAction::KeepCurrent,
            "a credential-free local model already runs: no switch"
        );
        // A faux fallback id cannot answer: the provider's first bundled row
        // takes over, named explicitly so `/model` can move it.
        let first = epsilon_llm::catalog::first_model_for_provider("anthropic")
            .expect("bundled anthropic rows exist")
            .id
            .to_string();
        assert_eq!(
            post_login_action("faux-1", "anthropic"),
            PostLoginAction::Select(first),
            "faux after login resolves to the provider's first bundled row"
        );
        // Case-insensitive provider, like the rest of the ladder.
        assert!(
            matches!(
                post_login_action("faux-1", "Anthropic"),
                PostLoginAction::Select(_)
            ),
            "provider matching ignores case"
        );
        // No bundled rows (customs, unknown ids): guidance, not a guess.
        assert_eq!(
            post_login_action("faux-1", "definitely-not-a-provider"),
            PostLoginAction::NoBundledModels,
            "nothing bundled means no selection to make"
        );
    }

    #[test]
    fn subscription_warning_fires_only_for_anthropic_oauth_shaped_keys() {
        // P-061: pi's `isAnthropicSubscriptionAuthKey` predicate, same shape.
        assert!(
            needs_subscription_warning("anthropic", "sk-ant-oat-abc123"),
            "an OAuth-shaped anthropic key warns"
        );
        assert!(
            !needs_subscription_warning("anthropic", "sk-ant-abc123"),
            "a plain anthropic API key is not subscription auth"
        );
        assert!(
            !needs_subscription_warning("openai", "sk-ant-oat-abc123"),
            "the predicate is provider-scoped"
        );
        assert!(
            subscription_billing_warning().contains("per token"),
            "the warning names the billing surprise, not just the state"
        );
    }

    #[test]
    fn all_scope_lists_foreign_sessions_with_their_workspace_named() {
        // P-230: the All scope sees every workspace; rows from elsewhere name
        // their workspace, or the toggle answers nothing.
        use epsilon_session::SessionScope;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let header = |id: &str, cwd: &str| {
            format!(
                "{}\n",
                serde_json::json!({
                    "type": "session", "version": 3, "id": id,
                    "timestamp": "2026-09-14T10:00:00Z", "cwd": cwd,
                })
            )
        };
        let home = epsilon_session::time::session_dir_for(dir, std::path::Path::new("/w"));
        let away = epsilon_session::time::session_dir_for(dir, std::path::Path::new("/elsewhere"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&away).unwrap();
        std::fs::write(home.join("a.jsonl"), header("ses_home", "/w")).unwrap();
        std::fs::write(away.join("b.jsonl"), header("ses_away", "/elsewhere")).unwrap();

        let now = 10_000_000_000u64;
        let ws = session_rows(
            dir,
            std::path::Path::new("/w"),
            SessionScope::Workspace,
            "",
            None,
            now,
        );
        assert_eq!(
            ws.len(),
            1,
            "workspace scope hides other workspaces: {ws:?}"
        );
        let all = session_rows(
            dir,
            std::path::Path::new("/w"),
            SessionScope::All,
            "",
            None,
            now,
        );
        assert_eq!(all.len(), 2, "all scope sees both workspaces: {all:?}");
        let foreign = all
            .iter()
            .find(|r| r.path == away.join("b.jsonl"))
            .expect("the foreign session is listed");
        assert!(
            foreign.meta.contains("elsewhere"),
            "the foreign row names its workspace: {}",
            foreign.meta
        );
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

        let rows = session_rows(
            dir,
            dir,
            epsilon_session::SessionScope::Workspace,
            "",
            None,
            1_000_000,
        );
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
        let rows = session_rows(
            dir.path(),
            dir.path(),
            epsilon_session::SessionScope::Workspace,
            "",
            None,
            0,
        );
        let row = rows.first().expect("the session is listed");
        assert!(row.meta.contains("in use"), "{}", row.meta);
    }

    #[test]
    fn age_and_size_labels_are_compact_and_deterministic() {
        let now = 1_000_000_000_000u64;
        assert_eq!(age_label(now, now), "just now");
        assert_eq!(age_label(now, now - 30_000), "just now");
        assert_eq!(
            age_label(now, now + 60_000),
            "timestamp in the future",
            "clock skew is not a fresh fetch"
        );
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
        // M55: the third query is a LABEL-only term — `v4.1` appears in
        // `deepseek-flash`'s label ("DeepSeek V4.1 Flash") and nowhere in its
        // id, so the label path and the case-folding both stay covered.
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

    /// A provider-name query ranks that provider's own models first (P-250).
    ///
    /// Pi's `/model` selector searches a **provider-first** haystack and says why
    /// in its own doc comment: *"keep the bare model ID out of the leading
    /// position"* so that a provider-prefixed query ranks above proxy ids like
    /// `openrouter/openai/gpt-5`
    /// (`references/pi-1.3.0/packages/coding-agent/src/modes/interactive/model-search.ts:13-21`).
    /// Its scoped overlay deliberately uses the opposite order, id first
    /// (`:7-11`), which is why the reference has two functions and Epsilon picks
    /// the selector's.
    ///
    /// RED before the fix, and printed verbatim: `got openai/gpt-oss-120b`,
    /// `left: "groq"`, `right: "openai"` — a proxy provider naming its upstream
    /// inside its own id outranked the provider being asked about.
    /// A configured id absent from the catalog renders `[unavailable]` (P-260).
    ///
    /// Config stores raw unvalidated ids, so a model that was saved once and
    /// later retired from the catalog is representable — and Pi's answer is to
    /// **keep** the row, strike it, and badge it ` [unavailable]`
    /// (`scoped-models-selector.ts:252-257`, with a regression test pinning the
    /// whole feature at `6949-unavailable-scoped-model.test.ts`). Epsilon's stage
    /// 2 dropped those ids entirely, so the running model vanished from the
    /// picker and the reader had no way to see what was actually configured.
    ///
    /// Only a bare or self-prefixed id can be attributed: an unknown
    /// `groq/foo-x` cannot be shown under `deepseek`, because the provider came
    /// from a prefix that no longer identifies a live provider.
    #[test]
    fn an_uncatalogued_configured_model_marked_unavailable() {
        let gone = "deepseek-v4-pro-vanished";
        let rows = build_models_for_provider("deepseek", "", gone, &[], None, &[], &[]);
        let row = rows
            .iter()
            .find(|r| r.id.eq_ignore_ascii_case(gone))
            .expect("a configured id absent from the catalog must still be offered");
        assert!(
            row.is_unavailable,
            "and marked unavailable, so the reader is told it cannot be selected: {row:?}"
        );
        // A real catalog row is never marked — the control.
        let live =
            build_models_for_provider("deepseek", "", "deepseek-v4-pro", &[], None, &[], &[]);
        let known = live
            .iter()
            .find(|r| r.id == "deepseek-v4-pro")
            .expect("a catalog row is offered");
        assert!(
            !known.is_unavailable,
            "a model the catalog knows is available: {known:?}"
        );
        // A live-listed id with no catalog row has **no wire**: it is shown so
        // the reader can see the provider still publishes it, but it is marked
        // unavailable because selecting it would make the next launch fail.
        let fetched_live = vec![LiveCacheEntry {
            provider: "deepseek".to_string(),
            ids: vec!["deepseek-new-thing".to_string()],
            fetched_at_ms: 1,
            from_cache: false,
        }];
        let fetched = build_models_for_provider("deepseek", "", "", &[], None, &fetched_live, &[]);
        let listed = fetched
            .iter()
            .find(|r| r.id == "deepseek-new-thing")
            .expect("a live id is offered");
        assert!(
            listed.is_unavailable,
            "a live id with no catalog row cannot be routed: {listed:?}"
        );
    }

    /// The control is really the control: an ordinary filtered stage-2 call
    /// gains no unavailable rows, because every row it lists is known.
    /// An unavailable row sorts **last**, and that is what keeps favourites intact.
    ///
    /// The desktop suite caught this one: the first version let the existing
    /// comparator place the synthesized row, so a row for the *current* model
    /// floated `current` to the top — above a starred model, which broke
    /// `starred_models_float_first_and_read_as_favorites`. An unavailable row is
    /// a report that something is configured, not a candidate to select, so it
    /// must never displace a model that can actually be applied.
    #[test]
    fn an_unavailable_row_sorts_below_every_available_one() {
        let gone = "deepseek-v4-pro-vanished";
        let rows = build_models_for_provider(
            "deepseek",
            "",
            gone,
            &["deepseek-v4-pro".to_string()],
            None,
            &[],
            &[],
        );
        // The starred model is first, exactly as before this feature existed.
        assert_eq!(
            rows.first().map(|r| r.id.as_str()),
            Some("deepseek-v4-pro"),
            "a favourite must still lead: {:?}",
            rows.iter()
                .take(4)
                .map(|r| r.id.clone())
                .collect::<Vec<_>>()
        );
        // And the unavailable row is at the end.
        let position = rows
            .iter()
            .position(|r| r.is_unavailable)
            .expect("the unavailable row is offered");
        assert_eq!(
            position,
            rows.len() - 1,
            "an unavailable row must sort last, not displace a real model"
        );
    }

    #[test]
    fn an_ordinary_stage_two_call_has_no_unavailable_rows() {
        let rows = build_models_for_provider("deepseek", "pro", "", &[], None, &[], &[]);
        assert!(!rows.is_empty(), "the filter matched something");
        assert!(
            rows.iter().all(|r| !r.is_unavailable),
            "nothing here is unknown to the catalog: {:?}",
            rows.iter().filter(|r| r.is_unavailable).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_provider_query_ranks_that_providers_own_models_first() {
        let rows = build_rows("openai", "", &[], &[]);
        let top = rows
            .iter()
            .find(|r| !r.is_custom)
            .expect("a non-guidance row matches");
        assert_eq!(
            top.provider, "openai",
            "a provider-name query must rank that provider's own rows first, got {}",
            top.id
        );
    }

    /// Stage 2 answers its own provider's name (P-250's sharper half).
    ///
    /// The stage-2 haystack was `id label` with the provider **absent**, so
    /// typing a provider's own name into stage 2 matched nothing at all and drew
    /// the widen hint. Descending into `groq` and typing `groq` must list groq's
    /// models.
    #[test]
    fn stage_two_answers_its_own_providers_name() {
        let rows = build_models_for_provider("groq", "groq", "", &[], None, &[], &[]);
        assert!(
            !rows.is_empty(),
            "filtering on the provider's own name must list its models"
        );
        assert!(
            rows.iter().all(|r| r.provider == "groq"),
            "and nothing else's: {:?}",
            rows.iter().map(|r| r.provider.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn live_rows_surface_only_unknown_ids_under_their_own_group() {
        let live = vec![LiveCacheEntry {
            provider: "deepseek".to_string(),
            ids: vec![
                "brand-new-model".to_string(),
                "deepseek-v4-pro".to_string(), // known: deduped into the catalog section
            ],
            fetched_at_ms: 1,
            from_cache: false,
        }];
        let rows = build_rows("", "", &[], &live);
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
        let live = vec![LiveCacheEntry {
            provider: "deepseek".to_string(),
            ids: vec!["deepseek-v4-pro".to_string(), "brand-new-model".to_string()],
            fetched_at_ms: 1,
            from_cache: false,
        }];
        let rows = build_models_for_provider("deepseek", "", "", &[], None, &live, &[]);
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
        let other_live = vec![LiveCacheEntry {
            provider: "openai".to_string(),
            ids: vec!["gpt-fake-live".to_string()],
            fetched_at_ms: 1,
            from_cache: false,
        }];
        let other = build_models_for_provider("anthropic", "", "", &[], None, &other_live, &[]);
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

        let stage2 = build_models_for_provider("lmstudio", "", "", &[], None, &[], &customs);
        assert!(
            stage2
                .iter()
                .any(|r| r.id == "lmstudio/" && r.provider == "custom provider")
        );

        // Typing past the prefix offers the full id directly.
        let typed =
            build_models_for_provider("lmstudio", "lmstudio/qwen", "", &[], None, &[], &customs);
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
        // stage2 does.
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

        let s1 = build_models_for_provider("deepseek", "pro", "", &[], None, &[], &[]);
        let s2 = build_models_for_provider("deepseek", "pro", "", &[], None, &[], &[]);
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
        let s2 = build_models_for_provider("deepseek", "deep pro", "", &[], None, &[], &[]);
        assert!(
            s2.iter().any(|r| r.id == "deepseek-v4-pro"),
            "stage2 multi-token must find deepseek-v4-pro"
        );
        // Non-matching second token must NOT match. The token is `qzx` rather
        // than a hyphenated word because fuzzy subsequence matching is exactly
        // that — *subsequence*, not substring: `zzz-nope` is a subsequence of
        // `amazon-bedrock/…nova-premier…`, so with a longer haystack it
        // legitimately matches and the old "miss" premise quietly became false.
        // A three-letter cluster found in no catalog id, label or provider is
        // the honest form of this assertion; it is the same property, stated so
        // it stays true as the haystack changes.
        let miss = build_rows("deep qzx", "", &[], &[]);
        assert!(
            miss.iter().all(|r| r.is_custom),
            "second token miss must yield only custom row: {:?}",
            miss.iter().map(|r| r.id.clone()).collect::<Vec<_>>()
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
        let s2 = build_models_for_provider("deepseek", "pro", "", &[], None, &[], &[]);
        assert_eq!(
            s2.first().map(|r| r.id.as_str()),
            Some("deepseek-v4-pro"),
            "stage2 ranking: pro top"
        );
        // Empty stage2 still puts current first (stable when no query).
        let s2_empty_a =
            build_models_for_provider("deepseek", "", "deepseek-v4-pro", &[], None, &[], &[]);
        let s2_empty_b =
            build_models_for_provider("deepseek", "", "deepseek-v4-pro", &[], None, &[], &[]);
        assert_eq!(s2_empty_a, s2_empty_b);
        assert_eq!(
            s2_empty_a.first().map(|r| r.id.as_str()),
            Some("deepseek-v4-pro")
        );
    }

    /// The saved default orders second and is flagged (P-249).
    ///
    /// Current first, default second, then the rest — pi's order with the
    /// default where pi puts it. `None` default keeps the old order
    /// (byte-identical rows, asserted by every existing caller passing it).
    /// Live rows stand in for catalog siblings (deterministic ids, same
    /// comparator). The `deepseek-a` decoy sorts before `deepseek-b` by id,
    /// so second place proves the default term fired — without it the decoy
    /// wins second.
    #[test]
    fn saved_default_orders_second_and_is_flagged() {
        // Catalog rows, not synthetic live ids: only a catalog row is
        // selectable, and an unavailable row deliberately sorts last so it can
        // never displace a real candidate.
        let rows = build_models_for_provider(
            "deepseek",
            "",
            "deepseek-v4-pro",
            &[],
            Some("deepseek-flash"),
            &[],
            &[],
        );
        assert_eq!(
            rows.first().map(|r| r.id.as_str()),
            Some("deepseek-v4-pro"),
            "current stays first"
        );
        let pos = rows
            .iter()
            .position(|r| r.id == "deepseek-flash")
            .expect("the default is listed");
        assert_eq!(pos, 1, "default second, not scored: {rows:?}");
        assert!(rows[pos].is_default, "and flagged for the mark: {rows:?}");
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

    /// P-043 guards (added with the merge; the key-path RED is
    /// `slash_popup_offers_discovered_skills/templates`): extra rows join
    /// the registry, fuzzy applies to them, and a later same-value row
    /// replaces the earlier one so a shadowing template carries its own
    /// description.
    #[test]
    fn slash_candidates_merges_extra_and_shadows_by_value() {
        let extra = vec![
            ("/skill commit".to_string(), "Record changes".to_string()),
            ("/review".to_string(), "Project code review".to_string()),
        ];
        let rows = slash_candidates("/skil", &extra);
        assert!(
            rows.iter()
                .any(|r| r.value == "/skill commit" && r.description == "Record changes"),
            "skill row merged: {rows:?}"
        );
        let rows = slash_candidates("/rev", &extra);
        assert!(
            rows.iter().any(|r| r.value == "/review"),
            "template row merged: {rows:?}"
        );
        // Shadowing: a template named like a head replaces the description.
        let shadow = vec![("/help".to_string(), "Custom help".to_string())];
        let rows = slash_candidates("/help", &shadow);
        let help: Vec<&SlashCandidate> = rows.iter().filter(|r| r.value == "/help").collect();
        assert_eq!(help.len(), 1, "one /help row, not two: {rows:?}");
        assert_eq!(help[0].description, "Custom help");
        // Gates: no slash, or a space, means no popup.
        assert!(slash_candidates("review", &extra).is_empty());
        assert!(slash_candidates("/skill commit", &extra).is_empty());
        // Empty needle lists everything, statics first.
        let all = slash_candidates("/", &extra);
        assert!(all.len() > SLASH_ITEMS.len());
        assert_eq!(all[0].value, SLASH_ITEMS[0].value);
    }

    /// P-044 guard (added with the argument popup; key-path RED is
    /// `slash_popup_completes_*`): ranking is shared, so the second word
    /// ranks like the first — best fuzzy first, empty needle in order.
    #[test]
    fn rank_candidates_orders_second_words_like_firsts() {
        let rows = vec![
            SlashCandidate {
                value: "/think high".into(),
                description: "thinking level".into(),
            },
            SlashCandidate {
                value: "/think xhigh".into(),
                description: "thinking level".into(),
            },
        ];
        let ranked = rank_candidates("xh", rows.clone());
        assert_eq!(ranked.len(), 1, "only /think xhigh holds an x: {ranked:?}");
        assert_eq!(ranked[0].value, "/think xhigh");
        let ranked = rank_candidates("high", rows.clone());
        assert_eq!(ranked.len(), 2, "both levels hold high: {ranked:?}");
        // Argument-mode ranks the fragment against the argument alone: with
        // the shared head scored, "rea" ties on the head's own letters and
        // insertion order would apply the wrong row.
        let preset_rows = ["auto", "read-only"].iter().map(|name| SlashCandidate {
            value: format!("/preset {name}"),
            description: "permission preset".into(),
        });
        let ranked = rank_candidates_by("rea", preset_rows.collect(), |row| {
            row.value
                .split_once(' ')
                .map(|(_, arg)| arg.trim())
                .unwrap_or("")
        });
        assert_eq!(ranked.len(), 1, "only read-only holds rea: {ranked:?}");
        assert_eq!(ranked[0].value, "/preset read-only");
        let ranked = rank_candidates("", rows.clone());
        assert_eq!(ranked, rows, "empty needle keeps insertion order");
        assert!(rank_candidates("zzz", rows).is_empty());
    }

    /// P-048 guard (key-path RED is `bare_think_opens_...`): the level
    /// filter lists all seven on an empty query and narrows by name.
    #[test]
    fn match_think_levels_lists_all_then_narrows() {
        assert_eq!(match_think_levels("").len(), 7);
        let names: Vec<&str> = match_think_levels("h")
            .iter()
            .map(|level| level.as_str())
            .collect();
        assert_eq!(names, vec!["high", "xhigh"]);
    }

    /// Filtered sessions rank by fuzzy score, not recency — and ties keep
    /// newest-first (P-242): the sort is stable over the newest-first base, so
    /// equal scores never reorder what the directory already ordered.
    #[test]
    fn session_filter_ranks_by_score_with_newest_first_ties() {
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
        // Better match is OLDER: score must beat recency, or this proves nothing.
        std::fs::write(
            dir.join("old.jsonl"),
            format!("{}{}", header("ses_old"), user("task")),
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(
            dir.join("new.jsonl"),
            format!("{}{}", header("ses_new"), user("mytask")),
        )
        .unwrap();
        let now = 10_000_000_000u64;
        let rows = session_rows(
            dir,
            dir,
            epsilon_session::SessionScope::Workspace,
            "task",
            None,
            now,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].title,
            "task",
            "the better match ranks first even though it is older: {:?}",
            rows.iter().map(|r| &r.title).collect::<Vec<_>>()
        );
        // Equal titles score equally: newest first stands.
        std::fs::write(
            dir.join("old.jsonl"),
            format!("{}{}", header("ses_old"), user("same words")),
        )
        .unwrap();
        // Rewrite order sets the age: the tie half needs old older than new,
        // and the earlier half already touched old.jsonl last.
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(
            dir.join("new.jsonl"),
            format!("{}{}", header("ses_new"), user("same words")),
        )
        .unwrap();
        let rows = session_rows(
            dir,
            dir,
            epsilon_session::SessionScope::Workspace,
            "same",
            None,
            now,
        );
        assert_eq!(rows.len(), 2);
        assert!(
            rows[0].meta.contains("new.jsonl"),
            "tied scores keep newest-first: {:?}",
            rows.iter().map(|r| &r.meta).collect::<Vec<_>>()
        );
    }

    /// Criterion 6: a bundle-retired id badges `retired`; any other unknown
    /// id stays `unavailable` (the existing struck row).
    #[test]
    fn retired_ids_badge_retired_others_stay_unavailable() {
        // `gpt-4` is in RETIRED_IDS (dropped from the bundle as deprecated
        // upstream) and absent from it as a bare id: the fixture is real,
        // not invented.
        let retired = "gpt-4";
        assert!(catalog::lookup_model(retired).is_none());
        let rows = build_models_for_provider("openai", "", retired, &[], None, &[], &[]);
        let row = rows
            .iter()
            .find(|r| r.id.eq_ignore_ascii_case(retired))
            .expect("a retired configured id must still be offered");
        assert!(row.is_retired, "retired ids badge retired: {row:?}");
        assert!(
            row.is_unavailable,
            "a retired row stays struck like every unavailable one: {row:?}"
        );
        assert_eq!(model_badge(row), Some("[retired]"));

        // The control: an unknown id nobody retired.
        let gone = "deepseek-v4-pro-vanished";
        let rows = build_models_for_provider("deepseek", "", gone, &[], None, &[], &[]);
        let row = rows
            .iter()
            .find(|r| r.id.eq_ignore_ascii_case(gone))
            .expect("offered");
        assert!(!row.is_retired, "nothing retired this id: {row:?}");
        assert!(row.is_unavailable);
        assert_eq!(model_badge(row), Some("[unavailable]"));

        // And a catalog row carries no badge at all.
        let rows =
            build_models_for_provider("deepseek", "", "deepseek-v4-pro", &[], None, &[], &[]);
        let known = rows
            .iter()
            .find(|r| r.id == "deepseek-v4-pro")
            .expect("a catalog row is offered");
        assert_eq!(model_badge(known), None);
    }

    /// Every synthetic source carries the availability flags, not just the
    /// configured-current row. A retired id in recents or favourites, or a
    /// live-listed id with no catalog wire, must look as unselectable as it is.
    #[test]
    fn retired_and_live_only_rows_are_marked_in_every_source() {
        let live = vec![crate::app::LiveCacheEntry {
            provider: "deepseek".into(),
            ids: vec!["deepseek/live-only".into()],
            fetched_at_ms: 1,
            from_cache: true,
        }];
        let rows = build_rows_with(
            "",
            "faux-1",
            &["deepseek-flash".to_string(), "local/qwen".to_string()],
            &["gpt-4".to_string(), "local/llama".to_string()],
            None,
            &live,
            &[CustomProvider {
                name: "local".into(),
                base_url: "http://127.0.0.1:1234/v1".into(),
                env_key: String::new(),
            }],
        );
        let find = |id: &str| {
            rows.iter()
                .find(|r| r.id.eq_ignore_ascii_case(id))
                .unwrap_or_else(|| panic!("{id} missing: {rows:#?}"))
        };
        let retired = find("gpt-4");
        assert!(
            retired.is_unavailable && retired.is_retired,
            "a retired favourite stays marked: {retired:?}"
        );
        let live_only = find("deepseek/live-only");
        assert!(
            live_only.is_unavailable && !live_only.is_retired,
            "a live id with no catalog row cannot be applied: {live_only:?}"
        );
        let custom = find("local/qwen");
        assert!(
            !custom.is_unavailable,
            "a declared custom provider prefix is a real route: {custom:?}"
        );
        assert!(
            !find("deepseek-flash").is_unavailable,
            "a catalog row stays selectable"
        );
    }

    /// Criterion 6: the live-status line names freshness — refreshed (this
    /// session) vs startup cache vs never fetched — with a std-only age.
    /// Singular `1 model` is pinned here (the plural bug read `1 models`).
    #[test]
    fn live_status_names_freshness() {
        let now = 10_000_000_000u64;
        let fresh = crate::app::LiveCacheEntry {
            provider: "deepseek".to_string(),
            ids: vec!["a".to_string(), "b".to_string()],
            fetched_at_ms: now,
            from_cache: false,
        };
        assert_eq!(
            live_status("deepseek", Some(&fresh), now),
            "live: deepseek (2 models · refreshed just now (this session))"
        );
        let cached = crate::app::LiveCacheEntry {
            from_cache: true,
            fetched_at_ms: now - 5 * 60 * 1000,
            ..fresh.clone()
        };
        assert_eq!(
            live_status("deepseek", Some(&cached), now),
            "live: deepseek (2 models · from startup cache · 5m ago · auto-refreshes on open)"
        );
        assert_eq!(
            live_status("deepseek", None, now),
            "live: deepseek (not fetched yet · opening refreshes)"
        );
        // Singular: one id reads `1 model`, never `1 models`.
        let single = crate::app::LiveCacheEntry {
            provider: "deepseek".to_string(),
            ids: vec!["only".to_string()],
            fetched_at_ms: now,
            from_cache: false,
        };
        assert_eq!(
            live_status("deepseek", Some(&single), now),
            "live: deepseek (1 model · refreshed just now (this session))"
        );
        let single_cached = crate::app::LiveCacheEntry {
            from_cache: true,
            fetched_at_ms: now - 5 * 60 * 1000,
            ..single.clone()
        };
        assert_eq!(
            live_status("deepseek", Some(&single_cached), now),
            "live: deepseek (1 model · from startup cache · 5m ago · auto-refreshes on open)"
        );
    }

    /// Criterion 6: struct entries merge exactly like the old tuples — only
    /// ids missing from the bundle surface, under their own provider.
    #[test]
    fn live_struct_entries_merge_like_tuples() {
        let live = vec![crate::app::LiveCacheEntry {
            provider: "deepseek".to_string(),
            ids: vec![
                "brand-new-model".to_string(),
                "deepseek-v4-pro".to_string(), // known: deduped into the catalog section
            ],
            fetched_at_ms: 1,
            from_cache: false,
        }];
        let rows = build_rows("", "", &[], &live);
        let surfaced = rows.iter().filter(|r| r.is_live).collect::<Vec<_>>();
        assert_eq!(surfaced.len(), 1);
        assert_eq!(surfaced[0].id, "brand-new-model");
        assert_eq!(surfaced[0].provider, "deepseek");
    }

    /// Every palette row is unique by command and by visible label.
    ///
    /// The registry already has aliases (`/tree` for `/branch`, `/models` for
    /// `/model`), but aliases are parser inputs, not palette rows. One row per
    /// canonical command is what makes the menu scannable; a duplicate row here
    /// is two entries that run the same action and teach nothing.
    #[test]
    fn palette_entries_are_unique_by_command_and_label() {
        let kb = crate::keybindings::KeybindingsManager::from_map(std::collections::HashMap::new());
        let entries = palette_entries(&kb);
        let mut commands = std::collections::HashSet::new();
        let mut labels = std::collections::HashSet::new();
        for entry in &entries {
            if !entry.command.is_empty() {
                assert!(
                    commands.insert(entry.command.trim()),
                    "duplicate command row: {entry:?}"
                );
            }
            assert!(
                labels.insert(entry.label()),
                "duplicate visible label: {entry:?}"
            );
        }
    }

    /// AGENTS.md §2.4 drift guard: every `COMMAND_HEADS` entry gets exactly
    /// one palette row, with name + description from the `SLASH_ITEMS` row the
    /// slash popup reads — one source, not a second list.
    ///
    /// RED: the hand-written palette names a handful of commands while the
    /// registry holds ~40, so most heads fail the coverage half below.
    #[test]
    fn palette_covers_every_command_head_from_the_registry() {
        let kb = crate::keybindings::KeybindingsManager::from_map(std::collections::HashMap::new());
        let entries = palette_entries(&kb);
        // One row per head, plus the workspace-tree action that has no head
        // (`/tree` is the *session* tree, a different feature).
        let slash_rows: Vec<&PaletteEntry> =
            entries.iter().filter(|e| e.action.is_none()).collect();
        assert_eq!(
            slash_rows.len(),
            COMMAND_HEADS.len(),
            "one palette row per command head, no more, no fewer: {:?}",
            entries
                .iter()
                .map(|e| e.command.clone())
                .collect::<Vec<_>>()
        );
        for head in COMMAND_HEADS {
            let item = SLASH_ITEMS
                .iter()
                .find(|item| item.value.trim() == format!("/{head}"))
                .unwrap_or_else(|| panic!("/{head} has no SLASH_ITEMS row"));
            let rows: Vec<&&PaletteEntry> = slash_rows
                .iter()
                .filter(|e| {
                    e.command
                        .trim()
                        .strip_prefix('/')
                        .and_then(|s| s.split([' ', ':']).next())
                        == Some(*head)
                })
                .collect();
            assert_eq!(
                rows.len(),
                1,
                "/{head} must have exactly one palette row: {:?}",
                rows.iter().map(|e| &e.command).collect::<Vec<_>>()
            );
            // The same source the popup reads: the row carries the registry's
            // own spelling and every word of its description, not a second
            // copy of them. The one exception is punctuation: an em dash is
            // not in CP437 (see the `palette_entries` note), so the title
            // carries the repertoire-safe rendering of the same words.
            assert_eq!(
                rows[0].command, item.value,
                "/{head} carries a reworded spelling"
            );
            assert!(
                rows[0].title.contains(item.value.trim())
                    && rows[0].title.contains(&item.description.replace('—', "·")),
                "/{head} names the registry row: {:?}",
                rows[0].title
            );
        }
        // Repertoire guard at the derivation point: a registry edit that adds
        // a character no legacy console draws must fail here, not as a box on
        // the frame (M219; the ASCII golden gate holds the drawn rows, this
        // holds every row). Titles are ASCII plus the middle dot, by
        // construction above.
        for entry in &entries {
            assert!(
                entry.title.chars().all(|c| c.is_ascii() || c == '·'),
                "palette title draws outside the repertoire: {:?}",
                entry.title
            );
        }
    }

    /// `filter_palette` keeps its behavior over the derived rows: an empty
    /// query lists everything, a narrowing query returns a strict subset that
    /// still names the match, and nonsense matches nothing.
    #[test]
    fn filter_palette_narrows_the_derived_rows() {
        let kb = crate::keybindings::KeybindingsManager::from_map(std::collections::HashMap::new());
        let all = filter_palette("", &kb);
        assert_eq!(
            all.len(),
            COMMAND_HEADS.len() + 1,
            "empty query lists every head plus the tree action"
        );
        let narrowed = filter_palette("hotkeys", &kb);
        assert!(
            !narrowed.is_empty() && narrowed.len() < all.len(),
            "a narrowing query returns a strict subset: {narrowed:?}"
        );
        assert!(
            narrowed.iter().any(|e| e.command.trim() == "/hotkeys"),
            "the subset still names the match: {narrowed:?}"
        );
        assert!(
            filter_palette("zzz-no-such-command", &kb).is_empty(),
            "nonsense matches nothing"
        );
    }

    /// D3: the palette entry splits into a label and a description column.
    #[test]
    fn d3_palette_entry_splits_label_and_description() {
        let kb = crate::keybindings::KeybindingsManager::from_map(std::collections::HashMap::new());
        let entries = palette_entries(&kb);
        let model = entries
            .iter()
            .find(|e| e.command.trim() == "/model")
            .expect("one /model row");
        assert_eq!(model.label(), "/model");
        assert_eq!(
            model.description(),
            "Switch or pick a model",
            "the description keeps the registry words: {:?}",
            model.title
        );
        let tree = entries
            .iter()
            .find(|e| e.action.is_some())
            .expect("the tree action");
        assert_eq!(tree.label(), "Files");
        assert!(!tree.description().is_empty());
    }

    /// D8: the live-status line joins with `·` and pluralizes the count.
    #[test]
    fn d8_live_status_uses_middle_dot_and_pluralizes() {
        let now = 10_000_000_000u64;
        let two = crate::app::LiveCacheEntry {
            provider: "deepseek".to_string(),
            ids: vec!["a".to_string(), "b".to_string()],
            fetched_at_ms: now,
            from_cache: false,
        };
        let line = live_status("deepseek", Some(&two), now);
        assert!(
            line.contains('·'),
            "the join is the row's own separator: {line:?}"
        );
        assert!(
            !line.contains(','),
            "no second notation for the same join: {line:?}"
        );
        assert!(line.contains("2 models"), "{line:?}");
        let one = crate::app::LiveCacheEntry {
            provider: "deepseek".to_string(),
            ids: vec!["only".to_string()],
            fetched_at_ms: now,
            from_cache: false,
        };
        let line = live_status("deepseek", Some(&one), now);
        assert!(line.contains("1 model"), "{line:?}");
        assert!(!line.contains("1 models"), "{line:?}");
    }
}
