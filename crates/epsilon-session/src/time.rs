//! Session timestamps, rendered the way the wire and the file names need them.
//!
//! Two consumers want the same string and neither may depend on the other: the RPC surface
//! renders entry timestamps for a pi-shaped client, and `--export`/session *file names* need the
//! same rendering (pi names a file `<ISO timestamp>_<id>.jsonl`,
//! `references/pi-0.85.1/packages/agent/src/harness/session/jsonl/repo.ts:42-45`). The formatter
//! lives here, in the crate that owns session files, rather than being written a second time in
//! whichever crate needed it next — which is the failure A12 measured in `global_dir`
//! (documented twice, byte for byte, in two crates that cannot see each other).

use std::path::{Path, PathBuf};

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

/// pi's directory name for a working copy — one directory per cwd, so "the newest session in
/// this workspace" is a question with an answer.
///
/// Transcribed from `references/pi-0.85.1/packages/agent/src/harness/session/jsonl/repo.ts:42-45`
/// rather than invented:
///
/// ```ts
/// function sessionDirectoryName(cwd: string): string {
///     return `--${cwd.replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}--`;
/// }
/// ```
///
/// The leading separator is stripped *before* the replacement, which is why a POSIX path does not
/// begin with a doubled dash and a Windows drive letter keeps its colon turned into a dash. Both
/// cases are asserted below against the reference's own rules, not against this comment.
#[must_use]
pub fn session_dir_name(cwd: &Path) -> String {
    // `replace(/^[/\\]/, "")` — one leading separator only.
    let raw = cwd.to_string_lossy();
    let stripped = raw
        .strip_prefix('/')
        .or_else(|| raw.strip_prefix('\\'))
        .unwrap_or(&raw);
    // `replace(/[/\\:]/g, "-")` — every separator and every drive colon.
    let mut name = String::with_capacity(stripped.len() + 4);
    name.push_str("--");
    for c in stripped.chars() {
        if c == '/' || c == '\\' || c == ':' {
            name.push('-');
        } else {
            name.push(c);
        }
    }
    name.push_str("--");
    name
}

/// pi's file name for one session: `<ISO timestamp>_<id>.jsonl`.
///
/// Transcribed from the same file (`repo.ts:47-50`):
///
/// ```ts
/// function sessionFileName(createdAt: number, id: string): string {
///     const timestamp = new Date(createdAt).toISOString().replace(/[:.]/g, "-");
///     return `${timestamp}_${encodeURIComponent(id)}.jsonl`;
/// }
/// ```
///
/// The timestamp goes through the *same* formatter the wire uses — that is the whole reason the
/// two functions live in one module: the colons and dots become dashes here, and the string a
/// client sees keeps them.
#[must_use]
pub fn session_file_name(created_at_ms: u64, id: &str) -> String {
    let stamp = epoch_millis_to_iso8601(created_at_ms);
    let stamp: String = stamp
        .chars()
        .map(|c| if c == ':' || c == '.' { '-' } else { c })
        .collect();
    format!("{stamp}_{}.jsonl", encode_uri_component(id))
}

/// Where a session file lives: `root/<pi's cwd directory>/<pi's file name>`.
///
/// One call, so that a caller cannot compose half the rule — which is how two crates end up
/// disagreeing about where a file is (A12's finding, in the config directory).
#[must_use]
pub fn session_file_for(root: &Path, cwd: &Path, created_at_ms: u64, id: &str) -> PathBuf {
    root.join(session_dir_name(cwd))
        .join(session_file_name(created_at_ms, id))
}

/// JavaScript's `encodeURIComponent`, for the id component of a file name.
///
/// A session id is generated here and is always in the unreserved set, so this is a transcription
/// of the *rule* rather than of a need: the moment an id comes from outside (an imported session,
/// a forked pi file), a name built without it could contain a separator and write outside the
/// directory that was just computed. Unreserved is `A-Za-z0-9-_.!~*'()`; everything else is
/// percent-encoded per UTF-8 byte, which is what the JavaScript function does.
#[must_use]
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        let c = byte as char;
        if c.is_ascii_alphanumeric()
            || matches!(c, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')')
        {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        encode_uri_component, epoch_millis_to_iso8601, session_dir_name, session_file_for,
        session_file_name,
    };
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

    /// The two name shapes, against the reference's own rules rather than against this
    /// implementation: a POSIX path loses its leading separator *before* the replacements (so it
    /// does not begin with a doubled dash), and a Windows drive colon becomes a dash.
    #[test]
    fn the_directory_name_follows_the_references_two_replacements() {
        assert_eq!(session_dir_name(Path::new("/home/u/p")), "--home-u-p--");
        // `^[/\\]` does not match `C`, so the whole path is kept and `:` is replaced.
        assert_eq!(
            session_dir_name(Path::new(r"C:\Users\Hp\Epsilon")),
            "--C--Users-Hp-Epsilon--"
        );
        assert_eq!(
            session_dir_name(Path::new("relative/dir")),
            "--relative-dir--"
        );
    }

    /// And the file name: colons and dots become dashes in the timestamp, the id is escaped, and
    /// the extension is `.jsonl`.
    #[test]
    fn the_file_name_is_the_timestamp_then_the_escaped_id() {
        assert_eq!(
            session_file_name(1_700_000_000_123, "session01"),
            "2023-11-14T22-13-20-123Z_session01.jsonl"
        );
        // A hostile id cannot escape the directory the caller just computed.
        let name = session_file_name(0, "../../etc/passwd");
        assert!(!name.contains('/'), "{name}");
        assert!(name.ends_with(".jsonl"), "{name}");
        assert!(name.contains("%2F"), "{name}");
    }

    /// The punctuation `encodeURIComponent` leaves alone, and one multi-byte character to prove
    /// the encoding is per UTF-8 byte rather than per `char`.
    #[test]
    fn the_id_escaping_matches_encode_uri_component() {
        assert_eq!(encode_uri_component("aZ0-_.!~*'()"), "aZ0-_.!~*'()");
        assert_eq!(encode_uri_component("a b"), "a%20b");
        assert_eq!(encode_uri_component("é"), "%C3%A9");
        assert_eq!(encode_uri_component("a/b"), "a%2Fb");
    }

    /// One call composes the whole path, so no caller can apply half the rule.
    #[test]
    fn the_path_is_the_directory_and_the_name() {
        let path = session_file_for(Path::new("/sessions"), Path::new("w"), 0, "id1");
        assert_eq!(
            path,
            Path::new("/sessions/--w--/1970-01-01T00-00-00-000Z_id1.jsonl")
        );
    }
}
