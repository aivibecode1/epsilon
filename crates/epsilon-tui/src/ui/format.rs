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
        return format!("…{}", chars[chars.len().saturating_sub(keep)..].iter().collect::<String>());
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
