//! Pure model-picker logic: row construction and slash-command parsing.
//!
//! Rendering lives in `app.rs`; everything testable without a terminal lives here.
//! Rows come from the bundled catalog (`epsilon_llm::catalog`) plus persisted
//! recents, session-fetched live listings, and a synthetic "custom id" row whenever
//! the filter text is not an exact known id — pi's escape hatch plus opencode's
//! favorites/recents convention, minus the parts that need a credential store.

use epsilon_llm::catalog;

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
        auth_ready: info.env_key.is_empty() || std::env::var_os(info.env_key).is_some(),
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
    live: &[String],
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

    for id in live {
        if matches_filter(id) && !catalog::lookup_model(id).is_some() {
            // Live ids not in the bundled catalog surface under their own group;
            // known ids already appear via the catalog section below.
            push_unique(
                synthetic_row(id, "live", false, true, false),
                &mut rows,
                &mut seen,
            );
        } else if matches_filter(id) && catalog::lookup_model(id).is_none() {
            unreachable!()
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

/// A parsed editor command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Open the model picker.
    Models,
    /// Set the model id directly (everything after `/model `).
    Model(String),
    /// Show the command reference.
    Help,
    /// Clear the transcript pane.
    Clear,
    /// List discovered skills.
    Skills,
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
        "help" | "?" => Some(Command::Help),
        "clear" => Some(Command::Clear),
        "skills" => Some(Command::Skills),
        "quit" | "q" | "exit" => Some(Command::Quit),
        other => Some(Command::Unknown(other.to_string())),
    }
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
            &[
                "brand-new-model".to_string(),
                "deepseek-chat".to_string(), // known: deduped into the catalog section
            ],
        );
        let live = rows.iter().filter(|r| r.is_live).collect::<Vec<_>>();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, "brand-new-model");
        assert!(rows.iter().any(|r| r.id == "deepseek-chat" && !r.is_live));
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
