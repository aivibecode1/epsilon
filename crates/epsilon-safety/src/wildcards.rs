//! Glob matching for permission patterns.
//!
//! V1 semantics (documented simplification vs opencode's full wildcard engine):
//! - `*` matches any run of characters INCLUDING path separators — so `*.env` matches any
//!   path ending in `.env`, and `*` alone matches everything;
//! - `?` matches exactly one character.
//!
//! Path-aware segment matching (`**`, boundary rules) lands with the M10 sandbox work, where
//! it can be tested against real confinement paths.

/// Match `subject` against a glob `pattern` (`*` and `?` wildcards).
pub fn glob_match(pattern: &str, subject: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = subject.chars().collect();
    glob_rec(&p, 0, &s, 0)
}

fn glob_rec(p: &[char], mut pi: usize, s: &[char], mut si: usize) -> bool {
    // Iterative star handling to avoid exponential backtracking on pathological input.
    let mut star_pi: Option<usize> = None;
    let mut star_si = 0usize;

    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_pi = Some(pi);
            star_si = si;
            pi += 1;
        } else if let Some(sp) = star_pi {
            // Backtrack into the last star, consuming one more character of subject.
            pi = sp + 1;
            star_si += 1;
            si = star_si;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
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
}
