//! Session timestamps, rendered the way the wire and the file names need them.
//!
//! Two consumers want the same string and neither may depend on the other: the RPC surface
//! renders entry timestamps for a pi-shaped client, and `--export`/session *file names* need the
//! same rendering (pi names a file `<ISO timestamp>_<id>.jsonl`,
//! `references/pi-0.85.1/packages/agent/src/harness/session/jsonl/repo.ts:42-45`). The formatter
//! lives here, in the crate that owns session files, rather than being written a second time in
//! whichever crate needed it next — which is the failure A12 measured in `global_dir`
//! (documented twice, byte for byte, in two crates that cannot see each other).

/// Epoch milliseconds to an ISO-8601 string, the way JavaScript's `new Date(n).toISOString()`
/// renders it.
///
/// A civil-date conversion by hand, on purpose: a date library is a new dependency for one
/// formatting call, and this is the standard days-from-civil algorithm (Hinnant's, inverted so the
/// era shift keeps every term positive). UTC only, with a `Z` suffix, because a timestamp with no
/// offset is a bug waiting for a client in another timezone — and pi's `toISOString()` is always
/// UTC with a `Z`.
///
/// The boundaries are pinned against the real calendar rather than against this arithmetic: the
/// leap day of 2000, the end of 9999, the epoch itself, and a negative-free millisecond remainder.
#[must_use]
pub fn epoch_millis_to_iso8601(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::epoch_millis_to_iso8601;

    /// Against the real calendar, not against the arithmetic above — a self-consistent
    /// conversion is exactly what a wrong one looks like.
    #[test]
    fn the_conversion_matches_the_real_calendar_at_its_boundaries() {
        assert_eq!(epoch_millis_to_iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(epoch_millis_to_iso8601(1_000), "1970-01-01T00:00:01.000Z");
        // 2000-02-29 — the leap day a naive four-year rule gets right and a century rule gets
        // wrong, which is why this one is asserted rather than assumed.
        assert_eq!(
            epoch_millis_to_iso8601(951_782_400_000),
            "2000-02-29T00:00:00.000Z"
        );
        assert_eq!(
            epoch_millis_to_iso8601(4_102_444_800_000),
            "2100-01-01T00:00:00.000Z"
        );
    }

    /// The millisecond remainder is the part a hand-rolled formatter drops, and a client sorting
    /// on it would then see ties that are not ties.
    #[test]
    fn the_millisecond_remainder_survives() {
        assert_eq!(
            epoch_millis_to_iso8601(1_700_000_000_123),
            "2023-11-14T22:13:20.123Z"
        );
    }
}
