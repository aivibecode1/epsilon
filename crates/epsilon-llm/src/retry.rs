//! Provider retry policy: classification + decorrelated-jitter backoff honoring Retry-After.
//!
//! Pure functions with no clock/network dependencies, so scheduling logic is unit-testable
//! (the caller supplies attempt counts and the observed Retry-After). Mirrors opencode's
//! exponential-with-jitter shape and pi's provider-retry-after cap.

use std::time::Duration;

/// Classification of a provider failure, driving the retry decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Rate limiting (429, quota) — retry with backoff; honor Retry-After.
    RateLimit,
    /// Transient server error (5xx) — retry.
    Server,
    /// Connection/socket/stream failure — retry.
    Network,
    /// Authentication/authorization — never retried (retrying leaks attempts and can lock
    /// accounts).
    Auth,
    /// Anything else — not retried by default.
    Other,
}

/// Spellings that mean **the account cannot pay**, not **the account is being throttled**.
///
/// These are checked *before* the rate-limit branch, because they frequently arrive wearing
/// a 429 and the word "quota" — and without this ordering the rate-limit branch wins and
/// retries something backing off cannot fix. `insufficient_quota` is OpenAI's billing code;
/// the rest is common gateway wording. pi resolves the same collision by testing its
/// non-retryable pattern first (`references/pi-1.0.0/packages/ai/src/utils/retry.ts:246-251`)
/// and pins `"429 quota exceeded"` as non-retryable (`test/retry.test.ts:80-86`).
const SPENDING_LIMIT_NEEDLES: [&str; 8] = [
    "insufficient_quota",
    "insufficient balance",
    "out of budget",
    "quota exceeded",
    "monthly usage limit",
    "usage limit reached",
    "available balance",
    "billing",
];

/// Is this a spending limit rather than a throttle?
///
/// **A spending limit is not a throttle.** Backing off does not put money in an account, so
/// retrying one costs the user five rounds of wall-clock before the same failure returns —
/// and the tell is invisible: nothing on screen says "this will not fix itself". The word
/// `quota` alone is not the signal, because it is genuinely ambiguous; the phrases above are
/// the ones a provider uses when it means the account rather than the request rate.
fn is_spending_limit(m: &str) -> bool {
    SPENDING_LIMIT_NEEDLES
        .iter()
        .any(|needle| m.contains(needle))
}

/// Classify a failure by its message text. Deliberately conservative: unknown strings are
/// `Other` (no retry), so classification bugs degrade to fail-fast rather than hammering.
pub fn classify(detail: &str) -> FailureKind {
    let m = detail.to_ascii_lowercase();
    // A spending limit first, and its classification is `Other` rather than `Auth` on
    // purpose: `Auth` is a key problem, and the front ends render that as "the model was
    // never reached". A billed-out account reached the model fine.
    if is_spending_limit(&m) {
        return FailureKind::Other;
    }
    if m.contains("rate limit")
        || m.contains("rate_limit")
        || m.contains("quota")
        || m.contains("too many requests")
        || m.contains("429")
    {
        FailureKind::RateLimit
    } else if m.contains("unauthorized")
        || m.contains("forbidden")
        || m.contains("invalid api key")
        || m.contains("401")
        || m.contains("403")
    {
        FailureKind::Auth
    } else if m.contains("timeout")
        // M60: reqwest says "operation timed out" and the stream watchdog says
        // "timed out" — both are network failures, and a classifier that only
        // knows the noun would route the most common stall to `Other` (no retry).
        || m.contains("timed out")
        || m.contains("connection")
        || m.contains("socket")
        || m.contains("reset")
        || m.contains("eof")
        || m.contains("dns")
    {
        FailureKind::Network
    } else if m.contains(" 500")
        || m.contains(" 502")
        || m.contains(" 503")
        || m.contains("internal server")
        || m.contains("bad gateway")
        || m.contains("service unavailable")
        || m.contains("overloaded")
    {
        FailureKind::Server
    } else {
        FailureKind::Other
    }
}

/// Whether a classified failure should be retried after `attempt` failures already seen.
///
/// `attempt` is zero-based: `0` means the first try just failed.
pub fn should_retry(kind: FailureKind, attempt: u32, max_attempts: u32) -> bool {
    if attempt + 1 >= max_attempts {
        return false;
    }
    matches!(
        kind,
        FailureKind::RateLimit | FailureKind::Server | FailureKind::Network
    )
}

/// Delay before the next attempt (None = do not retry).
///
/// - Honors provider `Retry-After` when present (capped at [`MAX_DELAY`]).
/// - Otherwise exponential base-500ms growth with decorrelated jitter in [0.5x, 1.5x],
///   seeded from wall-clock nanos (bounds-tested, not exactness-tested — exactness would
///   require injecting a clock, deferred until flakiness is ever observed).
pub fn backoff_delay(attempt: u32, retry_after: Option<Duration>) -> Duration {
    if let Some(d) = retry_after {
        // Provider-provided waits win, but never exceed our hard cap.
        return d.min(MAX_DELAY);
    }
    let exp = BASE_MS.saturating_mul(1u64 << attempt.min(6));
    let ceiling = exp.min(MAX_DELAY.as_millis() as u64);
    let jitter = pseudo_jitter(attempt);
    let delay = ceiling / 2 + jitter % (ceiling / 2 + 1);
    Duration::from_millis(delay)
}

const BASE_MS: u64 = 500;
/// Hard cap on any single wait (opencode uses similar caps via provider headers).
pub const MAX_DELAY: Duration = Duration::from_secs(30);

fn pseudo_jitter(seed: u32) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let mut x = seed ^ nanos;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    u64::from(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_billing_limit_is_final_even_when_it_wears_a_429() {
        // **The same condition, two spellings, two behaviours.**
        //
        // `quota` sat in the rate-limit branch, which is checked FIRST, so
        // `"429 quota exceeded"` classified as `RateLimit` and was retried five times with
        // backoff — while `"HTTP 403: you do not have access to this model"` classified as
        // `Auth` and was final. They are the same underlying condition (the account cannot
        // pay for this), and only one of them cost the user five rounds of wall-clock before
        // the honest failure surfaced.
        //
        // pi resolves it by checking the non-retryable pattern *before* the retryable one
        // (`references/pi-1.0.0/packages/ai/src/utils/retry.ts:246-251`), and pins exactly
        // this string (`test/retry.test.ts:80-86`). The rule is that a *spending* limit is
        // not a *throttle*: backing off does not put money in an account.
        for m in [
            "429 quota exceeded",
            "insufficient_quota",
            "You exceeded your current quota, please check your plan and billing details.",
            "HTTP 429: out of budget",
            "HTTP 429: monthly usage limit reached",
        ] {
            let kind = classify(m);
            assert_ne!(
                kind,
                FailureKind::RateLimit,
                "a spending limit must not be retried as a throttle: {m} classified {kind:?}"
            );
            assert!(
                !should_retry(kind, 0, 5),
                "and it must not be retried at all: {m} classified {kind:?}"
            );
        }

        // **The other half, so this is not "quota is now never retryable".** A genuine
        // throttle — the thing backoff actually fixes — keeps retrying.
        for m in [
            "HTTP 429 Too Many Requests",
            "rate limited, retry after 20s",
            "too many requests",
        ] {
            assert_eq!(
                classify(m),
                FailureKind::RateLimit,
                "a real throttle must stay retryable: {m}"
            );
        }
    }

    #[test]
    fn classifies_common_failures() {
        assert_eq!(
            classify("HTTP 429 Too Many Requests"),
            FailureKind::RateLimit
        );
        assert_eq!(classify("Invalid API key provided"), FailureKind::Auth);
        assert_eq!(classify("connection reset by peer"), FailureKind::Network);
        assert_eq!(
            classify("provider stream timed out: no data for 120s"),
            FailureKind::Network
        );
        assert_eq!(classify("operation timed out"), FailureKind::Network);
        assert_eq!(classify("503 Service Unavailable"), FailureKind::Server);
        assert_eq!(classify("something novel"), FailureKind::Other);
    }

    #[test]
    fn auth_and_never_retry_kinds_are_final() {
        assert!(!should_retry(FailureKind::Auth, 0, 5));
        assert!(!should_retry(FailureKind::Other, 0, 5));
        assert!(should_retry(FailureKind::RateLimit, 0, 5));
        assert!(!should_retry(FailureKind::Server, 4, 5));
    }

    #[test]
    fn backoff_respects_caps_and_honors_retry_after() {
        // saturated exponent stays within [half-ceiling, cap] thanks to decorrelated jitter
        let d = backoff_delay(9, None);
        assert!(d >= Duration::from_secs(15) && d <= MAX_DELAY, "got {d:?}");
        assert_eq!(
            backoff_delay(0, Some(Duration::from_millis(1200))),
            Duration::from_millis(1200)
        );
        // oversized Retry-After gets clamped
        assert_eq!(backoff_delay(0, Some(Duration::from_secs(999))), MAX_DELAY);
    }

    #[test]
    fn backoff_grows_but_stays_bounded() {
        let d0 = backoff_delay(0, None);
        let d3 = backoff_delay(3, None);
        assert!(d0 >= Duration::from_millis(250) && d0 <= MAX_DELAY);
        assert!(d3 >= d0.saturating_sub(Duration::from_millis(250)));
    }
}
