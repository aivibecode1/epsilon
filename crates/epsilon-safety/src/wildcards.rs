//! Glob matching for permission patterns.
//!
//! Opencode-parity semantics (`packages/core/src/util/wildcard.ts:match`):
//! - Backslashes are normalized to `/` in BOTH inputs before matching.
//! - Regex metas `.+^${}()|[]\` are escaped; `*` → `.*`, `?` → `.`.
//! - A trailing ` .*` becomes `( .*)?` (so `foo .*` matches `foo` as well).
//! - The resulting regex is anchored (`^…$`) and matched case-insensitively
//!   on Windows, case-sensitively elsewhere — matching opencode's `si` vs `s`
//!   flag split.

/// Match `subject` against a glob `pattern` (`*` and `?` wildcards).
pub fn glob_match(pattern: &str, subject: &str) -> bool {
    // Cheap literal: no wildcards at all. Must still normalize for the trailing-.* rule.
    let p_norm = pattern.replace('\\', "/");
    let s_norm = subject.replace('\\', "/");
    if !p_norm.contains(['*', '?']) {
        // Keep the regex path for correctness around the ` .*` suffix + metas.
        return match_regex(&p_norm, &s_norm);
    }
    match_regex(&p_norm, &s_norm)
}

fn match_regex(pattern_norm: &str, subject_norm: &str) -> bool {
    let mut escaped = String::with_capacity(pattern_norm.len() * 2);
    for c in pattern_norm.chars() {
        match c {
            '.' | '+' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\' => {
                escaped.push('\\');
                escaped.push(c);
            }
            '*' => escaped.push_str(".*"),
            '?' => escaped.push('.'),
            other => escaped.push(other),
        }
    }
    if escaped.ends_with(" .*") {
        escaped.truncate(escaped.len() - 3);
        escaped.push_str("( .*)?");
    }
    let full = format!("^{escaped}$");
    regex_match(&full, subject_norm)
}

fn regex_match(pattern: &str, subject: &str) -> bool {
    regex_impl(pattern, subject)
}

fn regex_impl(pattern: &str, subject: &str) -> bool {
    let re = regex::RegexBuilder::new(pattern)
        .case_insensitive(cfg!(windows))
        .dot_matches_new_line(true)
        .build()
        .unwrap_or_else(|_| regex::Regex::new("$^").unwrap());
    re.is_match(subject)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_match_within_and_across_segments() {
        assert!(glob_match("*", "anything/at/all"));
        assert!(glob_match("*.env", "/repo/.env"));
        assert!(glob_match("*.env", "deep/nested/x.env"));
        assert!(!glob_match("*.env", "x.environment"));
        assert!(glob_match("*.env.example", "cfg/app.env.example"));
    }

    #[test]
    fn question_mark_and_exact() {
        assert!(glob_match("cargo", "cargo"));
        assert!(!glob_match("cargo", "cargo-audit"));
        assert!(glob_match("c?rgo", "cargo"));
        assert!(!glob_match("c?rgo", "carggo"));
    }

    #[test]
    fn empty_patterns() {
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
        assert!(glob_match("*", ""), "a star matches zero characters too");
        assert!(glob_match("**", ""));
    }

    #[test]
    fn backslash_normalization_and_regex_meta_escape() {
        // Backslashes become forward slashes before matching.
        assert!(glob_match(r"src\*.rs", "src/foo.rs"));
        assert!(glob_match(r"src\*.rs", r"src\foo.rs"));
        // Dots are literals now (regex-escaped), not \"any char\".
        assert!(!glob_match("a.b", "acb"));
        assert!(glob_match("a.b", "a.b"));
        // Trailing ` *` becomes optional group — \"foo *\" matches \"foo\" too
        // (opencode's ` .*` -> `( .*)?` rule after escaping).
        assert!(glob_match("foo *", "foo"));
        assert!(glob_match("foo *", "foo bar baz"));
        assert!(!glob_match("foo *", "foobar"));
    }

    #[test]
    fn opencode_wildcard_parity_spot_checks() {
        // Spot checks derived from opencode's wildcard.ts cases.
        assert!(glob_match("*.ts", "src/foo.ts"));
        assert!(!glob_match("*.ts", "src/foo.tsx"));
        assert!(glob_match("src/*", "src/a/b/c"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
    }
}
