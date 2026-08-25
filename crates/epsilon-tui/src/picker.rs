//! Pure model-picker logic: row construction and slash-command parsing.
//!
//! Rendering lives in `app.rs`; everything testable without a terminal lives here.
//! Rows come from the bundled catalog (`epsilon_llm::catalog`) plus persisted
//! recents, session-fetched live listings, and a synthetic "custom id" row whenever
//! the filter text is not an exact known id — pi's escape hatch plus opencode's
//! favorites/recents convention, minus the parts that need a credential store.

use epsilon_llm::catalog;

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
    }
}

/// Build the picker rows for a filter string.
///
/// Order (empty filter): active model first, then recents, then the bundled catalog.
/// With a filter: case-insensitive substring matches across id/label/provider from all
/// sources; when the filter is not an exact known id, a trailing custom row offers it
/// verbatim.
pub fn build_rows(
    filter: &str,
    current: &str,
    recents: &[String],
    live: &[(String, Vec<String>)],
) -> Vec<PickerRow> {
    let needle = filter.trim().to_ascii_lowercase();
    let matches_filter =
        |hay: &str| needle.is_empty() || hay.to_ascii_lowercase().contains(&needle);

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
        && matches_filter(&format!("{} {} {}", info.id, info.label, info.provider))
    {
        push_unique(row_from_info(info), &mut rows, &mut seen);
    }

    for id in recents {
        if matches_filter(id)
            && let Some(info) = catalog::lookup_model(id)
        {
            let mut row = row_from_info(info);
            row.is_recent = true;
            row.provider = "recent".into();
            push_unique(row, &mut rows, &mut seen);
        } else if matches_filter(id) {
            push_unique(
                synthetic_row(id, "recent", false, false, true),
                &mut rows,
                &mut seen,
            );
        }
    }

    // Live listings (M20): ids fetched from each provider's own API that are NOT
    // already in the bundled catalog surface under their provider's group.
    for (provider, ids) in live {
        for id in ids {
            if matches_filter(id) && catalog::lookup_model(id).is_none() {
                push_unique(
                    synthetic_row(id, provider, false, true, false),
                    &mut rows,
                    &mut seen,
                );
            }
        }
    }

    for info in catalog::curated() {
        if matches_filter(&format!("{} {} {}", info.id, info.label, info.provider)) {
            push_unique(row_from_info(info), &mut rows, &mut seen);
        }
    }

    let trimmed = filter.trim();
    if !trimmed.is_empty() && !rows.iter().any(|r| r.id.eq_ignore_ascii_case(trimmed)) {
        rows.push(synthetic_row(trimmed, "custom", true, false, false));
    }
    rows
}

// ---- M17b: slash autocomplete with descriptions + selection (pi pattern) --------

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
        value: "/skills",
        description: "list discovered skills",
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
        value: "/quit",
        description: "exit the session",
    },
];

/// Candidates whose head matches the typed `/prefix` (input has no space yet),
/// in registry order — pi returns fuzzy-filtered registry order too.
pub fn slash_items(input: &str) -> Vec<&'static SlashItem> {
    if !input.starts_with('/') || input.contains(' ') {
        return Vec::new();
    }
    SLASH_ITEMS
        .iter()
        .filter(|item| item.value.starts_with(input))
        .collect()
}

/// A parsed editor command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Open the model picker.
    Models,
    /// Set the model id directly (everything after `/model `).
    Model(String),
    /// Set thinking effort (level after `/think `; bare `/think` shows current).
    Think(Option<String>),
    /// Show the command reference.
    Help,
    /// Clear the transcript pane.
    Clear,
    /// List discovered skills.
    Skills,
    /// Search the session log (`/search <query>`); empty tail opens with guidance.
    Search(String),
    /// Exit the session.
    Quit,
    /// A `/word` we do not know — surfaced as an error, never sent to the model.
    Unknown(String),
}

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
        "model" => Some(Command::Model(tail.to_string())),
        "think" => Some(Command::Think(if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        })),
        "help" | "?" => Some(Command::Help),
        "clear" => Some(Command::Clear),
        "skills" => Some(Command::Skills),
        "search" => Some(Command::Search(tail.to_string())),
        "quit" | "q" | "exit" => Some(Command::Quit),
        other => Some(Command::Unknown(other.to_string())),
    }
}

// ---- M16c: two-stage picker (opencode dialog-provider → dialog-model) -----------

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
}

/// Stage-1 rows: one per distinct provider in the bundled catalog, ordered by
/// (auth-ready desc, id asc) — ready providers first, alphabetical within groups.
pub fn build_provider_rows(current: &str) -> Vec<ProviderRow> {
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
            }),
        }
    }
    rows.sort_by(|a, b| b.auth_ready.cmp(&a.auth_ready).then(a.id.cmp(&b.id)));
    rows
}

/// Stage-2 rows: the models of ONE provider, reusing [`build_rows`] ordering rules
/// via fuzzy filter across id + label. Live-listed ids for this provider that are
/// missing from the bundled catalog are merged on top (M20 auto-fetch).
pub fn build_models_for_provider(
    provider: &str,
    filter: &str,
    current: &str,
    live: &[(String, Vec<String>)],
) -> Vec<PickerRow> {
    let needle = filter.trim();
    let mut rows: Vec<PickerRow> = catalog::curated()
        .iter()
        .filter(|i| i.provider == provider)
        .map(row_from_info)
        .filter(|row| {
            needle.is_empty()
                || fuzzy::fuzzy_match(needle, &row.id).matches
                || fuzzy::fuzzy_match(needle, &row.label).matches
        })
        .collect();
    if let Some((_, ids)) = live.iter().find(|(p, _)| p == provider) {
        for id in ids {
            let known = rows.iter().any(|r| r.id.eq_ignore_ascii_case(id));
            if !known && (needle.is_empty() || fuzzy::fuzzy_match(needle, id).matches) {
                rows.push(synthetic_row(id, provider, false, true, false));
            }
        }
    }
    // Active model floats to top.
    rows.sort_by_key(|r| r.id != current);
    rows
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
    /// Key hint shown on the right (e.g. "Ctrl+M").
    pub binding: Option<&'static str>,
}

/// All palette commands (slash commands + direct actions), in display order.
pub fn palette_entries() -> Vec<PaletteEntry> {
    vec![
        PaletteEntry {
            title: "Models — switch model".into(),
            command: "/models".into(),
            binding: Some("Ctrl+M"),
        },
        PaletteEntry {
            title: "Skills — list discovered skills".into(),
            command: "/skills".into(),
            binding: None,
        },
        PaletteEntry {
            title: "Clear transcript".into(),
            command: "/clear".into(),
            binding: None,
        },
        PaletteEntry {
            title: "Help — command reference".into(),
            command: "/help".into(),
            binding: None,
        },
        PaletteEntry {
            title: "Quit session".into(),
            command: "/quit".into(),
            binding: None,
        },
        PaletteEntry {
            title: "Toggle tool output expand".into(),
            command: "\u{1}expand".into(),
            binding: Some("x"),
        },
    ]
}

/// Fuzzy-filtered palette entries; empty query returns everything.
pub fn filter_palette(query: &str) -> Vec<PaletteEntry> {
    let q = query.trim();
    if q.is_empty() {
        return palette_entries();
    }
    let mut scored: Vec<(i64, PaletteEntry)> = palette_entries()
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

#[cfg(test)]
mod tests {
    use super::*;

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
        for q in ["deepseek", "DEEPSEEK", "reasoner"] {
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
            build_rows("deepseek-chat", "", &[], &[])
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
                    "deepseek-chat".to_string(), // known: deduped into the catalog section
                ],
            )],
        );
        let live = rows.iter().filter(|r| r.is_live).collect::<Vec<_>>();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, "brand-new-model");
        assert_eq!(live[0].provider, "deepseek", "grouped under its provider");
        assert!(rows.iter().any(|r| r.id == "deepseek-chat" && !r.is_live));
    }

    /// M20: stage-2 merges live-listed ids for its provider on top of the
    /// bundled catalog rows (this is the path that makes fresh models appear).
    #[test]
    fn stage2_merges_live_ids_for_its_provider_only() {
        let live = vec![(
            "deepseek".to_string(),
            vec!["deepseek-chat".to_string(), "brand-new-model".to_string()],
        )];
        let rows = build_models_for_provider("deepseek", "", "", &live);
        assert!(
            rows.iter().any(|r| r.id == "brand-new-model" && r.is_live),
            "unknown live id must surface in stage 2"
        );
        // Known id appears once, from the catalog (not duplicated as a live row).
        assert_eq!(
            rows.iter().filter(|r| r.id == "deepseek-chat").count(),
            1,
            "no duplicate rows for known ids"
        );
        // A different provider's live listing must NOT leak in.
        let other = build_models_for_provider(
            "anthropic",
            "",
            "",
            &[("openai".to_string(), vec!["gpt-fake-live".to_string()])],
        );
        assert!(
            !other.iter().any(|r| r.is_live),
            "stage 2 must not show other providers' live ids"
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
        assert_eq!(
            parse_command("/bogus"),
            Some(Command::Unknown("bogus".into())),
            "unknown slash words error out — never sent to the model"
        );
        assert_eq!(parse_command("fix the bug"), None);
    }
}
