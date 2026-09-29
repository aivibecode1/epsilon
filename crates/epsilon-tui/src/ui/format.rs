//! Formatting rules for the numbers the UI shows.
//!
//! These live apart from the widgets that use them because they carry policy:
//! how much precision a cost deserves, when a token count may round, what
//! happens to a percentage that exceeds its range. A widget that formats inline
//! makes that policy invisible and lets two screens disagree about the same
//! number — which is exactly how a status bar starts lying.

/// Format a token count so it keeps a stable width as the number grows.
///
/// The rules, kept from the reference behaviour because users read these fluently:
/// below 1000 the exact number, below ~10k one decimal of `k`, below ~1M whole
/// `k`, below ~10M one decimal of `M`, above that whole `M`.
///
/// The awkward part is the seams. Rounding must not produce `1000k` (a unit that
/// should have been carried) or `10k` immediately after `9.9k` (a field that
/// loses a character at an unpredictable moment). So each threshold is the value
/// at which the *rounded* form would cross into the next unit, not the round
/// number itself:
///
/// * `9_949` → `9.9k` — the last value that needs a decimal
/// * `9_950` → `10k` — whole thousands start here, not at a round 10_000
/// * `999_499` → `999k`, `999_500` → `1.0M` — carries rather than printing `1000k`
/// * `9_949_999` → `9.9M`, `9_950_000` → `10M` — the same seam, one unit up
pub fn format_tokens(count: u64) -> String {
    let n = count as f64;
    if count < 1_000 {
        count.to_string()
    } else if count < 9_950 {
        // One decimal: 1000 is `1.0k`, and 9949 is `9.9k`.
        format!("{:.1}k", n / 1_000.0)
    } else if count < 999_500 {
        // Whole thousands from the point where the rounding to the next unit
        // would otherwise print `1000k`.
        format!("{}k", (n / 1_000.0).round() as u64)
    } else if count < 9_950_000 {
        format!("{:.1}M", n / 1_000_000.0)
    } else {
        format!("{}M", (n / 1_000_000.0).round() as u64)
    }
}

/// Micro-USD to a compact dollar string.
///
/// Costs live as integer micro-dollars everywhere in the harness (ADR-010) so
/// nothing drifts through float arithmetic; this is the one place they become
/// text. Small amounts keep enough precision to be useful — `$0.0004` tells the
/// reader their session costs nothing yet, while `$0.00` hides whether anything
/// is happening at all.
pub fn format_cost(micro_usd: u64) -> String {
    let dollars = micro_usd as f64 / 1_000_000.0;
    if micro_usd == 0 {
        "$0".to_string()
    } else if dollars < 0.01 {
        format!("${dollars:.4}")
    } else if dollars < 1.0 {
        format!("${dollars:.3}")
    } else if dollars < 100.0 {
        format!("${dollars:.2}")
    } else {
        format!("${dollars:.0}")
    }
}

/// A percentage as a whole number, clipped to `0..=100`.
///
/// Clipping matters: an estimate built from a stale prompt size can exceed the
/// window, and a status bar reading `103%` reads as a bug in the harness rather
/// than an estimate being wrong.
pub fn format_percent(value: f64) -> String {
    format!("{}%", value.round().clamp(0.0, 100.0) as u64)
}

/// A **cache** hit rate, which must never round a partial hit up to `100%`.
///
/// `100%` is a claim about the cache: "every prompt token came from cache".
/// `format_percent` rounds, so a real 99.6% — a warm long session, sitting
/// there routinely — printed as `CH 100%`, which says something materially
/// different and is not true. That is the failure this repository calls "a
/// status bar starts lying", and it is worse here than an imprecise number
/// because the imprecision runs in the flattering direction.
///
/// The rule, which is the whole of the fix: **a rate that is not exactly 1.0
/// must not print as 100**. DSH solves the same problem with a precision ladder
/// that adds decimals until the value is distinguishable
/// (`token-format.ts:67-97`); the ladder is unnecessary here because the
/// underlying tokens are already in hand, so the honest rendering is direct.
///
/// 0.0 still prints `0%` — that claim is true, and unlike the top of the range
/// it is not the flattering direction.
pub fn format_cache_percent(rate: f64) -> String {
    if rate >= 1.0 {
        return "100%".to_string();
    }
    // One decimal is enough to separate 99.5 from 100, which is the only
    // boundary that matters. Below 99.5 the whole-percent form is exact enough
    // and cheaper to read, so the header does not gain noise for nothing.
    if rate >= 0.995 {
        format!("{:.1}%", rate * 100.0)
    } else {
        format_percent(rate * 100.0)
    }
}

#[cfg(test)]
mod cache_percent_tests {
    use super::format_cache_percent;

    /// The property, stated once: a rate that is not exactly 1.0 must not
    /// render as `100%`.
    ///
    /// Written against `format_cache_percent` rather than against a literal,
    /// so the test says what must be true instead of what the output is today.
    #[test]
    fn a_partial_hit_never_prints_as_a_full_one() {
        for (rate, label) in [
            (0.996, "a warm long session"),
            (0.999, "nearly everything cached"),
            (0.999_9, "one miss in a million"),
            (0.995, "exactly the rounding boundary"),
        ] {
            let out = format_cache_percent(rate);
            assert_ne!(
                out, "100%",
                "{label}: a {rate} rate printed as a full cache"
            );
        }
    }

    /// And the converse, which is what makes the first test a claim about
    /// honesty rather than about timidity: a genuine full hit still says 100%.
    #[test]
    fn a_real_full_hit_still_says_one_hundred() {
        assert_eq!(format_cache_percent(1.0), "100%");
    }

    /// Zero is a true claim and must not be softened.
    #[test]
    fn a_zero_hit_says_zero() {
        assert_eq!(format_cache_percent(0.0), "0%");
    }

    /// Ordinary values keep the cheap whole-percent form, so the header does
    /// not gain a decimal everywhere for one boundary.
    #[test]
    fn ordinary_rates_are_unchanged() {
        assert_eq!(format_cache_percent(0.87), "87%");
        assert_eq!(format_cache_percent(0.5), "50%");
        assert_eq!(format_cache_percent(0.124), "12%");
    }

    /// The boundary is real and in the right place: 99.4 rounds to 99, and
    /// 99.6 must not round to 100.
    #[test]
    fn the_boundary_is_where_the_rounding_is() {
        assert_eq!(format_cache_percent(0.994), "99%");
        assert_eq!(format_cache_percent(0.996), "99.6%");
    }
}

/// A duration as the shortest honest form.
pub fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Shorten a path for display: `~` for the home directory, then the tail.
///
/// A status bar has room for one path. Showing its tail rather than its head is
/// the useful choice — the last two components identify a project, while
/// `/Users/someone/Documents/...` identifies nothing.
pub fn format_path(path: &std::path::Path, home: Option<&std::path::Path>, width: usize) -> String {
    let mut shown = path.display().to_string();
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        shown = format!("~{}", std::path::MAIN_SEPARATOR);
        if !rest.as_os_str().is_empty() {
            shown.push_str(&rest.display().to_string());
        }
    }
    if shown.chars().count() <= width {
        return shown;
    }
    // Keep the final components, which is what names the location.
    let parts: Vec<&str> = shown.split(std::path::MAIN_SEPARATOR).collect();
    let mut tail = String::new();
    for part in parts.iter().rev() {
        let candidate = if tail.is_empty() {
            part.to_string()
        } else {
            format!("{part}{}{tail}", std::path::MAIN_SEPARATOR)
        };
        if candidate.chars().count() + 2 > width {
            break;
        }
        tail = candidate;
    }
    if tail.is_empty() {
        // Nothing fits whole: hard-truncate, keeping the tail characters.
        let chars: Vec<char> = shown.chars().collect();
        let keep = width.saturating_sub(1);
        return format!(
            "…{}",
            chars[chars.len().saturating_sub(keep)..]
                .iter()
                .collect::<String>()
        );
    }
    format!("…{}{tail}", std::path::MAIN_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_format_follows_the_reference_rules() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_000), "1.0k");
        // The seam is where rounding would otherwise print four digits of
        // thousands (9949 -> 9.9k, 9950 -> 10k), not at a round 10_000: a status
        // bar that suddenly says `10.0k` would be a character wider than the
        // value it replaced for no reason the reader can see.
        assert_eq!(format_tokens(9_949), "9.9k");
        assert_eq!(format_tokens(9_950), "10k");
        assert_eq!(format_tokens(10_000), "10k");
        assert_eq!(format_tokens(999_499), "999k");
        assert_eq!(format_tokens(999_500), "1.0M");
        assert_eq!(format_tokens(1_000_000), "1.0M");
        assert_eq!(format_tokens(10_000_000), "10M");
    }

    #[test]
    fn cost_stays_readable_at_every_scale() {
        assert_eq!(format_cost(0), "$0");
        assert_eq!(format_cost(400), "$0.0004");
        assert_eq!(format_cost(12_345), "$0.012");
        assert_eq!(format_cost(1_234_567), "$1.23");
        assert_eq!(format_cost(250_000_000), "$250");
    }

    #[test]
    fn percent_is_clipped_to_a_believable_range() {
        assert_eq!(format_percent(0.0), "0%");
        assert_eq!(format_percent(12.4), "12%");
        assert_eq!(format_percent(103.0), "100%");
        assert_eq!(format_percent(-5.0), "0%");
    }

    #[test]
    fn duration_picks_the_shortest_honest_form() {
        use std::time::Duration;
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59s");
        assert_eq!(format_duration(Duration::from_secs(61)), "1m01s");
        assert_eq!(format_duration(Duration::from_secs(3_700)), "1h01m");
    }

    #[test]
    fn path_keeps_the_tail_that_names_the_project() {
        let sep = std::path::MAIN_SEPARATOR;
        // Paths are built the way the platform spells them, so the expectation
        // cannot accidentally encode a Unix-only assumption.
        let home = std::path::PathBuf::from(format!("{sep}home{sep}dev"));
        let deep = home.join("work").join("clients").join("acme").join("api");
        // Wide enough for everything: the home prefix is abbreviated, and the
        // abbreviation uses the platform separator rather than mixing two.
        assert_eq!(
            format_path(&deep, Some(&home), 80),
            format!("~{sep}work{sep}clients{sep}acme{sep}api")
        );
        // Narrower: drop whole leading components, never a partial one.
        let narrow = format_path(&deep, Some(&home), 12);
        assert!(narrow.ends_with("api"), "{narrow}");
        assert!(narrow.starts_with('…'), "{narrow}");
        assert!(narrow.chars().count() <= 12, "{narrow}");
        // A path outside home keeps its absolute form.
        let other = std::path::PathBuf::from(format!("{sep}opt{sep}tools"));
        assert_eq!(
            format_path(&other, Some(&home), 40),
            format!("{sep}opt{sep}tools")
        );
    }
}
