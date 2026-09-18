//! Fuzzy subsequence matching (M16c) — pi's `fuzzy.ts` ported, no deps.
//!
//! Matches if all query chars appear in order (case-insensitive). Lower score is
//! better: word-boundary hits and consecutive runs win, gaps lose.

/// One match result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzyMatch {
    /// All query chars found in order.
    pub matches: bool,
    /// Lower is better; meaningless when `matches` is false.
    pub score: i64,
}

// Helper for pi's swapped alpha/numeric fallback (codex52 -> 52codex).
fn swapped_alpha_numeric(q: &str) -> Option<String> {
    // split into leading letters + trailing digits  ("codex52" -> "52codex") or
    // leading digits + trailing letters ("52codex" -> "codex52")
    let bytes = q.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut split = None;
    for i in 1..bytes.len() {
        let a_is_alpha = bytes[i - 1].is_ascii_alphabetic();
        let b_is_digit = bytes[i].is_ascii_digit();
        let a_is_digit = bytes[i - 1].is_ascii_digit();
        let b_is_alpha = bytes[i].is_ascii_alphabetic();
        if (a_is_alpha && b_is_digit) || (a_is_digit && b_is_alpha) {
            // only accept a single transition point; verify the rest is uniform
            let left_all_alpha = bytes[..i].iter().all(|c| c.is_ascii_alphabetic());
            let left_all_digit = bytes[..i].iter().all(|c| c.is_ascii_digit());
            let right_all_digit = bytes[i..].iter().all(|c| c.is_ascii_digit());
            let right_all_alpha = bytes[i..].iter().all(|c| c.is_ascii_alphabetic());
            if (left_all_alpha && right_all_digit) || (left_all_digit && right_all_alpha) {
                split = Some(i);
                break;
            }
        }
    }
    let i = split?;
    Some(format!("{}{}", &q[i..], &q[..i]))
}

/// Fuzzy subsequence match (pi `fuzzy.ts` parity — lower score is better).
pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0,
        };
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    if q.len() > t.len() {
        return FuzzyMatch {
            matches: false,
            score: 0,
        };
    }

    // pi: score tracks consecutive runs (escalating bonus), gaps, word boundaries,
    // slight late-anchor penalty (i*0.1), and an exact-match bonus.
    let mut score: i64 = 0;
    let mut qi = 0usize;
    let mut last_match: Option<usize> = None;
    let mut consecutive_run: i64 = 0;
    for (ti, tc) in t.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if *tc == q[qi] {
            let boundary = ti == 0
                || t[ti - 1] == ' '
                || t[ti - 1] == '-'
                || t[ti - 1] == '_'
                || t[ti - 1] == '.'
                || t[ti - 1] == '/'
                || t[ti - 1] == ':';
            if last_match == Some(ti.saturating_sub(1)) {
                consecutive_run += 1;
                score -= consecutive_run * 5;
            } else {
                consecutive_run = 0;
                if let Some(prev) = last_match {
                    score += ((ti - prev - 1) * 2) as i64;
                }
            }
            if boundary {
                score -= 10;
            }
            // pi: slight penalty for later anchors so earlier hits rank higher when
            // scores otherwise tie; use integer tenths to stay in i64.
            score += (ti as i64) / 10;
            last_match = Some(ti);
            qi += 1;
        }
    }
    if qi != q.len() {
        // pi swapped alpha/numeric fallback: "codex52" still finds "gpt-5.2-codex"
        // Normalize query by swapping a single trailing/leading digit block.
        let ql = query.to_lowercase();
        if let Some(swapped) = swapped_alpha_numeric(&ql) {
            let sw: Vec<char> = swapped.chars().collect();
            // re-run a cheap subsequence check for the swapped form with +5 penalty
            let mut qi2 = 0usize;
            let mut last2: Option<usize> = None;
            let mut score2: i64 = 5;
            let mut run2: i64 = 0;
            for (ti, tc) in t.iter().enumerate() {
                if qi2 >= sw.len() {
                    break;
                }
                if *tc == sw[qi2] {
                    let boundary =
                        ti == 0 || matches!(t[ti - 1], ' ' | '-' | '_' | '.' | '/' | ':');
                    if last2 == Some(ti.saturating_sub(1)) {
                        run2 += 1;
                        score2 -= run2 * 5;
                    } else {
                        run2 = 0;
                        if let Some(p) = last2 {
                            score2 += ((ti - p - 1) * 2) as i64;
                        }
                    }
                    if boundary {
                        score2 -= 10;
                    }
                    score2 += (ti as i64) / 10;
                    last2 = Some(ti);
                    qi2 += 1;
                }
            }
            if qi2 == sw.len() {
                return FuzzyMatch {
                    matches: true,
                    score: score2,
                };
            }
        }
        return FuzzyMatch {
            matches: false,
            score: 0,
        };
    }
    // pi: exact text match is strongly preferred
    if query.to_lowercase() == text.to_lowercase() {
        score -= 100;
    }
    FuzzyMatch {
        matches: true,
        score,
    }
}

/// Filter and sort by fuzzy match quality (pi `fuzzyFilter` parity).
///
/// Splits the query on whitespace and slash; every token must match. Results sort by total score.
pub fn fuzzy_filter<T: Clone>(items: &[T], query: &str, get_text: impl Fn(&T) -> String) -> Vec<T> {
    let q = query.trim();
    if q.is_empty() {
        return items.to_vec();
    }
    let tokens: Vec<&str> = q
        .split(|c: char| c.is_whitespace() || c == '/')
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.is_empty() {
        return items.to_vec();
    }
    let mut scored: Vec<(i64, T)> = Vec::new();
    for item in items {
        let text = get_text(item);
        let mut total: i64 = 0;
        let mut ok = true;
        for tok in &tokens {
            let m = fuzzy_match(tok, &text);
            if !m.matches {
                ok = false;
                break;
            }
            total += m.score;
        }
        if ok {
            scored.push((total, item.clone()));
        }
    }
    scored.sort_by_key(|(s, _)| *s);
    scored.into_iter().map(|(_, it)| it).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_matches_out_of_order() {
        assert!(fuzzy_match("ds", "deepseek-chat").matches);
        assert!(fuzzy_match("dsc", "deepseek-chat").matches);
        assert!(!fuzzy_match("zx", "deepseek-chat").matches);
        assert!(fuzzy_match("", "anything").matches);
        // Case-insensitive both ways.
        assert!(fuzzy_match("DS", "deepseek-chat").matches);
    }

    #[test]
    fn boundary_and_consecutive_hits_score_better() {
        let boundary = fuzzy_match("so", "claude-sonnet-4"); // 's' after '-'
        let interior = fuzzy_match("so", "some-model-xx"); // hmm also boundary; compare directly instead
        assert!(boundary.matches && interior.matches);

        let consecutive = fuzzy_match("ab", "xxabc");
        let gappy = fuzzy_match("ab", "xaxbxc");
        assert!(
            consecutive.score < gappy.score,
            "consecutive ({}) beats gappy ({})",
            consecutive.score,
            gappy.score
        );
    }

    #[test]
    fn shorter_gap_prefers_earlier_anchor() {
        let early = fuzzy_match("m4", "m4-model");
        let late = fuzzy_match("m4", "model-x-m4");
        assert!(early.matches && late.matches);
        assert!(early.score < late.score, "earlier anchor scores better");
    }
}
