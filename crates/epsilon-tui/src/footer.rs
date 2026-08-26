//! Footer/status computations (M16a) — pi's `footer.ts` conventions, pure + tested.
//!
//! Answers, on one line: where am I (cwd ~-shortened + git branch), what model,
//! token totals (compact k/M), cache-hit rate (pi's headline metric), context
//! window usage (`?` right after compaction), session cost. No deps beyond std:
//! the branch comes from reading `.git/HEAD` directly (never gitoxide).

use std::path::Path;

/// Compact token format (pi `formatTokens` verbatim rules):
/// `<1000 → n`, `<10k → x.yk`, `<1M → nk`, `<10M → x.yM`, else `nM`.
/// The k/M integer forms ROUND (JS Math.round semantics), not truncate.
pub fn format_tokens(count: u64) -> String {
    if count < 1_000 {
        return count.to_string();
    }
    if count < 10_000 {
        return format!("{:.1}k", count as f64 / 1_000.0);
    }
    if count < 1_000_000 {
        return format!("{}k", (count as f64 / 1_000.0).round() as u64);
    }
    if count < 10_000_000 {
        return format!("{:.1}M", count as f64 / 1_000_000.0);
    }
    format!("{}M", (count as f64 / 1_000_000.0).round() as u64)
}

/// `~`-shortened cwd (pi `formatCwdForFooter`). Outside home → path unchanged.
/// Separators come from `PathBuf` joining, so they are consistent per OS.
pub fn format_cwd(cwd: &Path, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return cwd.to_string_lossy().into_owned();
    };
    match cwd.strip_prefix(home) {
        Ok(rel) => {
            if rel.as_os_str().is_empty() {
                "~".to_string()
            } else {
                std::path::PathBuf::from("~")
                    .join(rel)
                    .to_string_lossy()
                    .into_owned()
            }
        }
        Err(_) => cwd.to_string_lossy().into_owned(),
    }
}

/// Current git branch read straight from `<dir>/.git/HEAD`
/// (`ref: refs/heads/<name>`); detached HEADs yield the short sha; anything else
/// → None. Cheap enough to call per draw; callers may add a TTL.
pub fn git_branch(workspace: &Path) -> Option<String> {
    let head = std::fs::read_to_string(workspace.join(".git").join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(rest) = head.strip_prefix("ref: refs/heads/") {
        return Some(rest.to_string());
    }
    // Detached: raw sha (print first 7 like git status does).
    if !head.is_empty() {
        let mut short = head.to_string();
        short.truncate(7);
        return Some(short);
    }
    None
}

/// Cache-hit rate in percent from accumulated usage (pi CH% convention):
/// cache_read ÷ latest prompt size (input + cache_read + cache_write).
/// `None` when there was no prompt yet or no cache traffic to speak of.
pub fn ch_rate(latest_prompt_tokens: u64, cache_read: u64) -> Option<f64> {
    if latest_prompt_tokens == 0 || cache_read == 0 {
        return None;
    }
    Some(cache_read as f64 / latest_prompt_tokens as f64 * 100.0)
}

/// One component of the assembled footer: text plus a semantic color tag.
#[derive(Debug, Clone, PartialEq)]
pub struct FooterPart {
    /// Rendered text (already compact).
    pub text: String,
    /// Semantic color: neutral dims, warn/error thresholds (context pressure).
    pub tone: Tone,
}

/// Severity driving footer coloring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    /// Ordinary dim stat.
    Normal,
    /// Context >70% (pi warning threshold).
    Warn,
    /// Context >90% (pi error threshold).
    Error,
}

/// Context-window component: `12%/200k` or `?/200k` right after compaction
/// (auto-compaction is always enabled in Epsilon today, so no `(auto)` suffix).
/// Returns parts with tones set by the >70%/>90% thresholds.
pub fn context_parts(
    latest_prompt_tokens: u64,
    compacted: bool,
    context_window: Option<u64>,
) -> Vec<FooterPart> {
    let Some(window) = context_window.filter(|w| *w > 0) else {
        return vec![];
    };
    let win = format_tokens(window);
    let (label, tone) = if compacted && latest_prompt_tokens == 0 {
        (format!("?/{win}"), Tone::Normal)
    } else if latest_prompt_tokens == 0 {
        return vec![];
    } else {
        let p = latest_prompt_tokens as f64 / window as f64 * 100.0;
        let tone = if p > 90.0 {
            Tone::Error
        } else if p > 70.0 {
            Tone::Warn
        } else {
            Tone::Normal
        };
        (format!("{p:.1}%/{win}"), tone)
    };
    vec![FooterPart { text: label, tone }]
}

/// The full right-hand footer segment: ↑in ↓out · R/W · CH% · $cost · model,
/// then the context part last (it carries the threshold tones).
pub fn stats_parts(stats: &FooterStats) -> Vec<FooterPart> {
    let mut parts: Vec<FooterPart> = Vec::new();

    if stats.tokens_in > 0 {
        parts.push(FooterPart {
            text: format!("↑{}", format_tokens(stats.tokens_in)),
            tone: Tone::Normal,
        });
    }
    if stats.tokens_out > 0 {
        parts.push(FooterPart {
            text: format!("↓{}", format_tokens(stats.tokens_out)),
            tone: Tone::Normal,
        });
    }
    if stats.cache_read > 0 {
        parts.push(FooterPart {
            text: format!("R{}", format_tokens(stats.cache_read)),
            tone: Tone::Normal,
        });
    }
    if stats.cache_write > 0 {
        parts.push(FooterPart {
            text: format!("W{}", format_tokens(stats.cache_write)),
            tone: Tone::Normal,
        });
    }
    if let Some(ch) = ch_rate(stats.latest_prompt_tokens, stats.cache_read) {
        parts.push(FooterPart {
            text: format!("CH{ch:.1}%"),
            tone: Tone::Normal,
        });
    }
    if stats.cost_micro_usd > 0 {
        parts.push(FooterPart {
            text: format!("${:.3}", stats.cost_micro_usd as f64 / 1_000_000.0),
            tone: Tone::Normal,
        });
    }
    // M25: thinking-level indicator (pi shows the level in its status line; the
    // README's future row promised this). Off stays hidden — no noise when
    // extended thinking is not requested.
    if let Some(level) = stats.thinking_level.filter(|l| l.enabled()) {
        parts.push(FooterPart {
            text: format!("think:{}", level.as_str()),
            tone: Tone::Normal,
        });
    }
    parts.extend(context_parts(
        stats.latest_prompt_tokens,
        stats.compacted,
        stats.context_window,
    ));
    parts
}

/// Everything the stats half of the footer needs (caller gathers from UiState/catalog).
#[derive(Debug, Clone, Copy, Default)]
pub struct FooterStats {
    /// Session input tokens.
    pub tokens_in: u64,
    /// Session output tokens.
    pub tokens_out: u64,
    /// Session cache reads.
    pub cache_read: u64,
    /// Session cache writes.
    pub cache_write: u64,
    /// Latest single-call prompt size.
    pub latest_prompt_tokens: u64,
    /// Compacted with no fresh counts yet.
    pub compacted: bool,
    /// Active model's context window (None/0 hides the context component honestly).
    pub context_window: Option<u64>,
    /// Active thinking level (M25 footer indicator). `None`/Off hides it.
    pub thinking_level: Option<epsilon_contracts::ThinkingLevel>,
    /// Session cost so far in integer micro-USD (ADR-010; never floats internally).
    pub cost_micro_usd: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_format_matches_pi_rules() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_000), "1.0k");
        assert_eq!(format_tokens(9_999), "10.0k");
        assert_eq!(format_tokens(12_345), "12k");
        assert_eq!(format_tokens(999_999), "1000k");
        assert_eq!(format_tokens(4_500_000), "4.5M");
        assert_eq!(format_tokens(48_000_000), "48M");
        assert_eq!(format_tokens(0), "0");
    }

    #[test]
    fn cwd_shortens_inside_home_only() {
        let home = Path::new("/home/mohamed");
        // PathBuf::join normalizes separators per-OS, so build the expectation the
        // same way the implementation does (no hardcoded separator literals).
        // Build BOTH sides through PathBuf so the comparison is separator-style
        // agnostic (a "/" literal path joins differently on Windows than "\" does).
        let cwd = std::path::Path::new("/home/mohamed")
            .join("projects")
            .join("app");
        let expected = std::path::PathBuf::from("~")
            .join(
                std::path::Path::new("/home/mohamed")
                    .join("projects")
                    .join("app")
                    .strip_prefix(home)
                    .unwrap(),
            )
            .to_string_lossy()
            .into_owned();
        assert_eq!(format_cwd(&cwd, Some(home)), expected);
        assert_eq!(format_cwd(Path::new("/home/mohamed"), Some(home)), "~");
        assert_eq!(
            format_cwd(Path::new("/elsewhere/x"), Some(home)),
            "/elsewhere/x"
        );
        assert_eq!(format_cwd(Path::new("/x"), None), "/x");
    }

    #[test]
    fn git_branch_parses_head_and_detached() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::create_dir_all(ws.join(".git")).unwrap();
        std::fs::write(ws.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(git_branch(ws).as_deref(), Some("main"));

        std::fs::write(
            ws.join(".git").join("HEAD"),
            "3f9c2ab77e5c4411aa88bb9f00d1e2c33a445566",
        )
        .unwrap();
        assert_eq!(git_branch(ws).as_deref(), Some("3f9c2ab"));

        std::fs::remove_dir_all(ws.join(".git")).unwrap();
        assert_eq!(git_branch(ws), None);
    }

    #[test]
    fn ch_rate_is_share_of_latest_prompt() {
        assert_eq!(ch_rate(0, 100), None);
        assert_eq!(ch_rate(100, 0), None);
        let r = ch_rate(10_600, 9_000).unwrap();
        assert!((r - 84.905_660_377_358_49).abs() < 1e-9);
    }

    #[test]
    fn context_shows_question_mark_after_compaction() {
        let parts = context_parts(0, true, Some(200_000));
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].text, "?/200k");

        // Before any response at all: nothing to show (honest empty).
        assert!(context_parts(0, false, Some(200_000)).is_empty());
        // Unknown window hides entirely.
        assert!(context_parts(50, false, None).is_empty());

        let warm = context_parts(150_000, false, Some(200_000)).pop().unwrap();
        assert_eq!(warm.text, "75.0%/200k");
        assert_eq!(warm.tone, Tone::Warn);

        let hot = context_parts(190_000, false, Some(200_000)).pop().unwrap();
        assert_eq!(hot.text, "95.0%/200k");
        assert_eq!(hot.tone, Tone::Error);

        let cool = context_parts(20_000, false, Some(200_000)).pop().unwrap();
        assert_eq!(cool.tone, Tone::Normal);
    }

    #[test]
    fn stats_parts_follow_pi_shape() {
        let s = FooterStats {
            tokens_in: 120_000,
            tokens_out: 3_400,
            cache_read: 110_000,
            cache_write: 6_000,
            latest_prompt_tokens: 116_000,
            compacted: false,
            context_window: Some(200_000),
            cost_micro_usd: 42_500,
            thinking_level: None,
        };
        let texts: Vec<String> = stats_parts(&s).into_iter().map(|p| p.text).collect();
        assert_eq!(
            texts,
            vec![
                "↑120k".to_string(),
                "↓3.4k".to_string(),
                "R110k".to_string(),
                "W6.0k".to_string(),
                "CH94.8%".to_string(),
                "$0.043".to_string(),
                "58.0%/200k".to_string(),
            ]
        );
    }

    #[test]
    fn fresh_session_renders_minimal_footer() {
        let texts: Vec<String> = stats_parts(&FooterStats::default())
            .into_iter()
            .map(|p| p.text)
            .collect();
        assert!(texts.is_empty(), "no usage yet → no fake numbers");
    }

    #[test]
    fn thinking_indicator_hides_when_off_and_shows_the_level_otherwise() {
        // M25: Off (and None) render nothing — no noise when thinking is off.
        let off = FooterStats {
            thinking_level: Some(epsilon_contracts::ThinkingLevel::Off),
            ..FooterStats::default()
        };
        assert!(
            stats_parts(&off)
                .iter()
                .all(|p| !p.text.starts_with("think:"))
        );

        let high = FooterStats {
            tokens_in: 1_000,
            latest_prompt_tokens: 1_000,
            context_window: Some(200_000),
            thinking_level: Some(epsilon_contracts::ThinkingLevel::High),
            ..FooterStats::default()
        };
        let texts: Vec<String> = stats_parts(&high).into_iter().map(|p| p.text).collect();
        assert!(texts.contains(&"think:high".to_string()));
    }
}
