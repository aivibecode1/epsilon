//! What a tool card *says*, per tool.
//!
//! Before this module every card spoke the same dialect. A result was rendered as
//! `format!("{name} → {}", truncate(body, 400))` and that whole string was the card's
//! **header line**. So a `read` card's headline was 400 characters of the file the user
//! was already looking at, and the one thing they actually wanted — which file, which
//! lines — was nowhere. The data was there: `read` reports `first_line`, `last_line` and
//! `total_lines` in `details`, and the path is in the arguments. It was simply being
//! thrown away in favour of a format string that did not know what tool it was describing.
//!
//! pi solves this with one file per tool (`core/tools/renderers/*.ts`). This is the same
//! idea in one place, and it is deliberately narrower:
//!
//! - pi hides a collapsed `read` result entirely (`read.ts:120-122` returns `""`). We do
//!   **not**, because pi pairs that with a click-to-expand affordance we do not have
//!   (`analysis #12`). Hiding the content with no way to bring it back would trade
//!   information for tidiness, which is the wrong trade. That divergence is a decision,
//!   not an oversight, and it is revisited if/when the card gains an expand binding.
//! - pi renders a `1 Hz Elapsed` row for `bash`. Left to M152's successor: the card's
//!   duration has the render-cache problem documented at `Card::elapsed_ms`, and a
//!   renderer cannot honestly fix a number that is computed elsewhere.
//!
//! Everything here is *presentation*. No renderer invents a fact: a line range comes from
//! `details.read`, a truncation flag from `details.truncated`, a path from the arguments
//! the model itself sent. If a field is missing, the head degrades to something vaguer
//! rather than to a guess.

use serde_json::Value;

/// The one-line summary of a tool call, shown while it runs and kept as the card's
/// headline afterwards.
///
/// This replaces a flat "find the first of eight known keys" heuristic. The heuristic was
/// wrong in a way that mattered: for `grep` it returned the *pattern* with no indication of
/// where it matched, and for any tool whose arguments did not use one of those eight spellings
/// it fell through to a raw JSON dump. A per-tool match says what the call actually was.
pub fn call_head(name: &str, args: &Value, ellipsis: &str) -> String {
    match name {
        "shell" | "bash" | "run" => with_command(args, ellipsis),
        "grep" => match (str_arg(args, "pattern"), str_arg(args, "path")) {
            (Some(p), Some(path)) => format!("/{p}/ in {path}"),
            (Some(p), None) => format!("/{p}/"),
            (None, Some(path)) => path,
            (None, None) => String::new(),
        },
        "read" | "ls" | "write" | "edit" | "glob" | "find" | "webfetch" | "web_fetch" => {
            str_arg(args, "path")
                .or_else(|| str_arg(args, "file_path"))
                .or_else(|| str_arg(args, "filePath"))
                .or_else(|| str_arg(args, "url"))
                .unwrap_or_default()
        }
        _ => str_arg(args, "command")
            .or_else(|| str_arg(args, "path"))
            .or_else(|| str_arg(args, "pattern"))
            .or_else(|| str_arg(args, "query"))
            .or_else(|| str_arg(args, "url"))
            .or_else(|| str_arg(args, "prompt"))
            .unwrap_or_default(),
    }
}

/// The headline a settled card shows, which may know more than the call did.
///
/// A `read` that returned lines 10–50 of a 300-line file should say so, because "read" alone
/// does not distinguish a two-line config from a page of source.
pub fn result_head(
    name: &str,
    args: &Value,
    details: &Value,
    body: &str,
    ellipsis: &str,
) -> String {
    let call = call_head(name, args, ellipsis);
    let extra = match name {
        "read" => details
            .get("read")
            .and_then(|r| {
                let first = r.get("first_line").and_then(Value::as_i64)?;
                let last = r.get("last_line").and_then(Value::as_i64)?;
                let total = r.get("total_lines").and_then(Value::as_i64);
                Some(match total {
                    // A whole-file read of the entire file: repeating the range would be noise.
                    Some(t) if first <= 1 && last >= t => String::new(),
                    Some(t) => format!(":{}–{last} of {t}", first),
                    None => format!(":{first}–{last}"),
                })
            })
            .unwrap_or_default(),
        "grep" => match (
            details.get("truncated").and_then(Value::as_bool),
            details.get("match_limit_reached").and_then(Value::as_u64),
        ) {
            (Some(true), Some(limit)) => format!(" · truncated at {limit} matches"),
            (Some(true), None) => " · truncated".into(),
            _ => String::new(),
        },
        "shell" | "bash" | "run" => {
            let mut extra = match details.get("output_complete").and_then(Value::as_bool) {
                // M148: the command exited successfully but a descendant held the pipe, so what
                // is on the card is not everything the command produced. The card must say so.
                Some(false) => " · output incomplete".to_string(),
                _ => String::new(),
            };
            // P-159: truncated output was spilled whole; the card names the
            // file so the rest is retrievable. Only present when truncation
            // actually spilled (small outputs carry no key).
            if let Some(path) = details.get("full_output_path").and_then(Value::as_str) {
                extra.push_str(&format!(" · full output: {path}"));
            }
            extra
        }
        _ => String::new(),
    };

    let head = format!("{call}{extra}");
    if head.is_empty() {
        // No path, no pattern, no command — a tool this renderer has never heard of. Keep
        // the tool's name, because this string is also the *collapsed transcript line*, where
        // dropping the name would leave a row of output with nothing saying which tool
        // produced it. Never a fourth of a kilobyte of it, which is what this replaced.
        let first = first_line(body, ellipsis);
        if first.is_empty() {
            name.to_string()
        } else {
            format!("{name} → {first}")
        }
    } else {
        head
    }
}

/// First non-blank line of a body, trimmed to something a header can hold.
///
/// The truncation mark is passed in rather than written here, for the reason every other glyph is a
/// field: `…` is **not in CP437**, so a legacy console draws it as a box (M219/A22).
fn first_line(body: &str, ellipsis: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| crate::state::truncate_public(l, 80, ellipsis))
        .unwrap_or_default()
}

/// `shell`'s headline is a command line, and reads as one: `$ cargo test`.
fn with_command(args: &Value, ellipsis: &str) -> String {
    match str_arg(args, "command").or_else(|| str_arg(args, "cmd")) {
        Some(cmd) if !cmd.trim().is_empty() => {
            format!("$ {}", collapse(&cmd, 100, ellipsis))
        }
        _ => String::new(),
    }
}

/// A non-empty string argument. Blank values are treated as absent, because a renderer that
/// renders `""` has lost information relative to one that renders nothing.
fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Collapse a multi-line command to its first line: a header is one line, and a
/// `bash -c` script's second line is usually an unrelated part of the same idea.
fn collapse(s: &str, max: usize, ellipsis: &str) -> String {
    let first = s.lines().next().unwrap_or(s).trim();
    crate::state::truncate_public(first, max, ellipsis)
}

#[cfg(test)]
mod tests;
