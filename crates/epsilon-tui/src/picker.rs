//! Pure model-picker logic: row construction and slash-command parsing.
//!
//! Rendering lives in `app.rs`; everything testable without a terminal lives here.
//! Rows come from the bundled catalog (`epsilon_llm::catalog`) plus a synthetic
//! "custom id" row whenever the filter text is not an exact catalog id — pi's escape
//! hatch for models we do not know about.

use epsilon_llm::catalog;

/// One selectable line in the `/models` dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    /// Model id applied on selection (the filter text for the custom row).
    pub id: String,
    /// Display label (catalog label or the raw id).
    pub label: String,
    /// Provider grouping key.
    pub provider: String,
    /// Credential env-var status for the auth-hint column.
    pub auth_ready: bool,
    /// True for the synthetic "use this exact id" row.
    pub is_custom: bool,
}

/// Build the picker rows for a filter string, keeping the active model visible.
///
/// Empty filter → full bundled catalog in display order. Non-empty filter →
/// case-insensitive substring match over id/label/provider; when the filter itself is
/// not an exact known id, a trailing custom row offers it verbatim.
pub fn build_rows(filter: &str, current: &str) -> Vec<PickerRow> {
    let needle = filter.trim().to_ascii_lowercase();
    let mut rows: Vec<PickerRow> = Vec::new();
    for info in catalog::curated() {
        let hay = format!("{} {} {}", info.id, info.label, info.provider).to_ascii_lowercase();
        if !needle.is_empty() && !hay.contains(&needle) {
            continue;
        }
        rows.push(PickerRow {
            id: info.id.to_string(),
            label: info.label.to_string(),
            provider: info.provider.to_string(),
            auth_ready: info.env_key.is_empty() || std::env::var_os(info.env_key).is_some(),
            is_custom: false,
        });
    }
    let trimmed = filter.trim();
    if !trimmed.is_empty() && !rows.iter().any(|r| r.id.eq_ignore_ascii_case(trimmed)) {
        rows.push(PickerRow {
            id: trimmed.to_string(),
            label: trimmed.to_string(),
            provider: "custom".into(),
            auth_ready: catalog::credentials_ready(trimmed),
            is_custom: true,
        });
    }
    // Keep the active model reachable even under aggressive filtering.
    if let Some(pos) = rows.iter().position(|r| r.id == current)
        && pos != 0
    {
        let active = rows.remove(pos);
        rows.insert(0, active);
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
}

/// Parse submitted input that starts with `/`; `None` means ordinary prompt text.
pub fn parse_command(input: &str) -> Option<Command> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let (head, tail) = match rest.split_once(' ') {
        Some((h, t)) => (h, t.trim()),
        None => (rest, ""),
    };
    match head.to_ascii_lowercase().as_str() {
        "models" => Some(Command::Models),
        "model" if !tail.is_empty() => Some(Command::Model(tail.to_string())),
        "help" | "?" => Some(Command::Help),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_filter_lists_full_catalog_in_order() {
        let rows = build_rows("", "faux-1");
        assert!(rows.len() >= 15, "bundled catalog should be substantial");
        assert_eq!(rows[0].id, "faux-1", "current model floats to top");
        assert!(rows.iter().all(|r| !r.is_custom));
        assert!(rows.iter().any(|r| r.auth_ready && r.provider == "faux"));
    }

    #[test]
    fn filter_matches_id_label_or_provider_case_insensitively() {
        for q in ["deepseek", "DEEPSEEK", "reasoner", "Reasoner"] {
            let rows = build_rows(q, "");
            assert!(
                rows.iter().any(|r| r.provider == "deepseek"),
                "query {q} must find deepseek"
            );
        }
        assert!(
            build_rows("zzz-no-match-zzz", "")
                .iter()
                .all(|r| r.is_custom)
        );
    }

    #[test]
    fn non_exact_filter_offers_custom_row_verbatim() {
        let rows = build_rows("my-fine-tune-v3", "faux-1");
        let custom = rows.iter().find(|r| r.is_custom).expect("custom row");
        assert_eq!(custom.id, "my-fine-tune-v3");
        assert_eq!(custom.provider, "custom");

        // Exact catalog ids never duplicate into a custom row.
        assert!(build_rows("deepseek-chat", "").iter().all(|r| !r.is_custom));
    }

    #[test]
    fn commands_parse_with_optional_args_and_case() {
        assert_eq!(parse_command("/models"), Some(Command::Models));
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
        assert_eq!(
            parse_command("/bogus"),
            None,
            "unknown commands stay prompts"
        );
        assert_eq!(parse_command("/model"), None, "bare /model is not valid");
        assert_eq!(parse_command("fix the bug"), None);
    }
}
