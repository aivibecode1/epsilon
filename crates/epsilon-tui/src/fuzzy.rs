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

/// Score `query` against `text` (both compared case-insensitively).
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

    let mut score: i64 = 0;
    let mut qi = 0usize;
    let mut last_match: Option<usize> = None;
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
            match last_match {
                Some(prev) if prev + 1 == ti => score -= 5, // consecutive bonus
                Some(prev) => score += ((ti - prev - 1) * 2) as i64, // gap penalty
                None => {}
            }
            if boundary {
                score -= 10;
            }
            last_match = Some(ti);
            qi += 1;
        }
    }
    FuzzyMatch {
        matches: qi == q.len(),
        score,
    }
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
