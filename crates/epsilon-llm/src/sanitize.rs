//! Provider-facing sanitation (port plan item 18; Hermes `agent/message_sanitization.py:195,551`).
//!
//! Two classes of malformed input that every harness with many provider wires eventually meets,
//! both of which Epsilon used to handle by *losing* the model's intent:
//!
//! 1. **Tool-call arguments that are not valid JSON.** A model emits `{"path": "a.rs",}` or a
//!    truncated object, and `providers::finish` replaced the whole payload with `{}`. The tool
//!    then ran with no arguments at all — failing validation with a message about a missing field,
//!    which says nothing about what actually happened. [`parse_tool_arguments`] recovers what can
//!    be recovered and, when it cannot, hands the raw text on so the failure can name it.
//! 2. **Duplicate tool-call ids in one message.** Providers reject them, and the result pairing
//!    becomes ambiguous (two calls, one id, two results). [`unique_call_ids`] renames later
//!    duplicates deterministically.
//!
//! The repairs are syntactic only. Nothing here invents a missing field, reorders a call, or
//! guesses at intent: an argument list that cannot be made valid is reported as such, because
//! "the model sent something I could not read" is a fact the model can act on, and a fabricated
//! `{}` is not.

use serde_json::Value;

/// What a repair pass did to one argument payload.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedArgs {
    /// Already a valid JSON object.
    Valid(Value),
    /// Recovered, with the fix recorded.
    Repaired {
        /// The recovered object.
        value: Value,
        /// Which repair produced it.
        fix: Fix,
    },
    /// Not recoverable: the raw text (bounded by the caller when it reports this).
    Unrecoverable {
        /// Exactly what the model sent.
        raw: String,
        /// Why it could not be read.
        reason: String,
    },
}

/// The repair that recovered an argument payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// The payload was a JSON string containing JSON.
    UnwrappedString,
    /// A trailing comma before a closing brace or bracket.
    TrailingComma,
    /// An unterminated string, or unclosed braces (an output-token cut).
    ClosedTruncation,
    /// Single-quoted keys/strings, converted because no double quote was present.
    SingleQuotes,
}

impl Fix {
    /// A short phrase for the result's details.
    pub fn note(&self) -> &'static str {
        match self {
            Fix::UnwrappedString => "arguments were a JSON string containing JSON",
            Fix::TrailingComma => "removed a trailing comma",
            Fix::ClosedTruncation => "closed arguments cut off by the output limit",
            Fix::SingleQuotes => "converted single-quoted JSON",
        }
    }
}

/// Read a tool call's arguments, repairing what can be repaired.
pub fn parse_tool_arguments(raw: &str) -> ParsedArgs {
    let text = raw.trim();
    if text.is_empty() {
        // An absent payload is a legitimate "no arguments" for a tool that takes none.
        return ParsedArgs::Valid(serde_json::json!({}));
    }
    match serde_json::from_str::<Value>(text) {
        // A JSON *string* is itself valid JSON, so this case is checked before the generic one:
        // some wires hand the arguments on as a string of JSON.
        Ok(Value::String(inner)) => {
            return match serde_json::from_str::<Value>(inner.trim()) {
                Ok(v) => match classify(v, text) {
                    ParsedArgs::Valid(v) => repaired(v, Fix::UnwrappedString, text),
                    other => other,
                },
                Err(_) => ParsedArgs::Unrecoverable {
                    raw: text.to_string(),
                    reason: "arguments were a string that is not JSON".to_string(),
                },
            };
        }
        Ok(v) => return classify(v, text),
        Err(_) => {}
    }

    // 2. Trailing commas before a closer.
    let no_commas = strip_trailing_commas(text);
    if no_commas != text
        && let Ok(v) = serde_json::from_str::<Value>(&no_commas)
    {
        return match classify(v, text) {
            ParsedArgs::Valid(v) => repaired(v, Fix::TrailingComma, text),
            other => other,
        };
    }

    // 3. Truncation: drop an unterminated trailing string, then close what is open.
    let closed = close_truncated(text);
    if closed != text
        && let Ok(v) = serde_json::from_str::<Value>(&closed)
    {
        return match classify(v, text) {
            ParsedArgs::Valid(v) => repaired(v, Fix::ClosedTruncation, text),
            other => other,
        };
    }

    // 4. Single quotes — only when there is no double quote to get confused about.
    if !text.contains('"')
        && let Some(converted) = convert_single_quotes(text)
        && let Ok(v) = serde_json::from_str::<Value>(&converted)
    {
        return match classify(v, text) {
            ParsedArgs::Valid(v) => repaired(v, Fix::SingleQuotes, text),
            other => other,
        };
    }

    ParsedArgs::Unrecoverable {
        raw: text.to_string(),
        reason: serde_json::from_str::<Value>(text)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "arguments are not a JSON object".to_string()),
    }
}

/// Accept a repair — unless it emptied the payload.
///
/// Truncation recovery can legitimately reach `{}` when the cut landed before anything readable.
/// Reporting that as a success would be the old silent-`{}` behaviour wearing a repair's clothes:
/// the tool would run with no arguments and the model would be told it forgot a field. It is
/// reported as unreadable instead, with the raw text, so the model can see where it was cut.
fn repaired(value: Value, fix: Fix, raw: &str) -> ParsedArgs {
    let emptied =
        value.as_object().is_some_and(|o| o.is_empty()) && !matches!(raw.trim(), "{}" | "{ }");
    if emptied {
        return ParsedArgs::Unrecoverable {
            raw: raw.to_string(),
            reason: "the arguments were cut before any complete member; nothing could be recovered"
                .to_string(),
        };
    }
    ParsedArgs::Repaired { value, fix }
}

/// A repaired payload still has to be an object: tools take objects, and a bare value would fail
/// every typed deserialization with a type error that hides the real story.
fn classify(v: Value, raw: &str) -> ParsedArgs {
    if v.is_object() {
        ParsedArgs::Valid(v)
    } else {
        ParsedArgs::Unrecoverable {
            raw: raw.to_string(),
            reason: format!("arguments must be a JSON object, got {}", kind_of(&v)),
        }
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Remove commas that sit directly before a closing brace or bracket, ignoring string contents.
fn strip_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut pending_comma: Option<usize> = None;
    for c in text.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                pending_comma = None;
                in_string = true;
                out.push(c);
            }
            ',' => {
                pending_comma = Some(out.len());
                out.push(c);
            }
            '}' | ']' => {
                if let Some(pos) = pending_comma.take() {
                    out.remove(pos);
                }
                out.push(c);
            }
            c if c.is_whitespace() => {
                // Whitespace between a comma and a closer still counts as a trailing comma.
                out.push(c);
            }
            _ => {
                pending_comma = None;
                out.push(c);
            }
        }
    }
    out
}

/// Close a truncated payload: drop an unterminated trailing string, then balance the brackets.
fn close_truncated(text: &str) -> String {
    let mut body = text.to_string();
    // An unterminated string swallows the rest of the payload; cut it back to its opening quote
    // only if it is the *last* thing open (otherwise the payload is broken earlier, not cut).
    let mut in_string = false;
    let mut escaped = false;
    let mut open_quote: Option<usize> = None;
    for (i, c) in body.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
                open_quote = None;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            open_quote = Some(i);
        }
    }
    if in_string && let Some(q) = open_quote {
        body.truncate(q);
        body = drop_incomplete_member(&body);
    }

    // Balance braces/brackets outside strings.
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for c in body.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.last() == Some(&c) {
                    stack.pop();
                } else {
                    // A closer that matches nothing: not a truncation, a malformed payload.
                    return text.to_string();
                }
            }
            _ => {}
        }
    }
    if stack.is_empty() {
        return text.to_string();
    }
    let mut out = body;
    while let Some(closer) = stack.pop() {
        out.push(closer);
    }
    out
}

/// Remove a member the cut left half-written: after the unterminated string is dropped, the
/// payload may end in `"key":` or `,"key":` (the cut landed before the value) or in a comma with
/// nothing after it. Those are not closable — `{"a": 1, "b":}` is not JSON — so the incomplete
/// member goes, rather than being closed into something the model never wrote.
fn drop_incomplete_member(body: &str) -> String {
    let mut s = body.trim_end().to_string();
    loop {
        if let Some(stripped) = s.strip_suffix(',') {
            s = stripped.trim_end().to_string();
            continue;
        }
        if let Some(stripped) = s.strip_suffix(':') {
            // Drop the colon and the key it belonged to.
            let head = stripped.trim_end();
            let Some(open) = head.rfind('"') else {
                return s;
            };
            let key_region = &head[..open];
            let Some(key_open) = key_region.rfind('"') else {
                return s;
            };
            s = head[..key_open].trim_end().to_string();
            continue;
        }
        return s;
    }
}

/// Convert `{'a': 'b'}` into `{"a": "b"}`. Only called when the payload holds no double quote.
fn convert_single_quotes(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 2);
    let mut in_string = false;
    for c in text.chars() {
        match c {
            '\'' => {
                in_string = !in_string;
                out.push('"');
            }
            '"' => return None, // ambiguous; refuse rather than guess
            c => out.push(c),
        }
    }
    if in_string {
        return None; // unterminated: that is truncation, handled elsewhere
    }
    Some(out)
}

/// Make tool-call ids unique within one message, preserving order and first occurrence.
///
/// Providers reject duplicate ids, and a duplicate makes the result pairing ambiguous: two calls
/// with one id, two results, and no way for a reader (or a wire) to say which answered which.
/// Later duplicates are renamed deterministically (`<id>#2`), so replaying the same message
/// produces the same ids.
///
/// Returns how many ids were renamed.
pub fn unique_call_ids(ids: &mut [String]) -> usize {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut renamed = 0usize;
    let mut taken: std::collections::HashSet<String> = ids.iter().cloned().collect();
    for id in ids.iter_mut() {
        if seen.insert(id.clone()) {
            continue;
        }
        let mut n = 2usize;
        let candidate = loop {
            let candidate = format!("{id}#{n}");
            if !taken.contains(&candidate) {
                break candidate;
            }
            n += 1;
        };
        taken.insert(candidate.clone());
        seen.insert(candidate.clone());
        *id = candidate;
        renamed += 1;
    }
    renamed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_arguments_pass_through_untouched() {
        let raw = r#"{"path": "src/lib.rs", "offset": 10}"#;
        match parse_tool_arguments(raw) {
            ParsedArgs::Valid(v) => assert_eq!(v["path"], "src/lib.rs"),
            other => panic!("expected valid, got {other:?}"),
        }
        // An empty payload is a legitimate no-argument call, not a failure.
        assert_eq!(
            parse_tool_arguments(""),
            ParsedArgs::Valid(serde_json::json!({}))
        );
        assert_eq!(
            parse_tool_arguments("   "),
            ParsedArgs::Valid(serde_json::json!({}))
        );
    }

    #[test]
    fn a_trailing_comma_is_recovered_and_named() {
        // The shape a model actually emits when it "tidies" its own JSON.
        let raw = r#"{"path": "a.rs", "limit": 5,}"#;
        match parse_tool_arguments(raw) {
            ParsedArgs::Repaired { value, fix } => {
                assert_eq!(fix, Fix::TrailingComma);
                assert_eq!(value["limit"], 5);
            }
            other => panic!("expected a repair, got {other:?}"),
        }
        // Inside a string, a comma before a brace is content, not syntax.
        let raw = r#"{"text": "a,}", "n": 1}"#;
        match parse_tool_arguments(raw) {
            ParsedArgs::Valid(v) => assert_eq!(v["text"], "a,}"),
            other => panic!("string contents must survive: {other:?}"),
        }
        // Arrays count too.
        let raw = r#"{"edits": [{"a": 1},]}"#;
        assert!(matches!(
            parse_tool_arguments(raw),
            ParsedArgs::Repaired {
                fix: Fix::TrailingComma,
                ..
            }
        ));
    }

    #[test]
    fn an_arguments_payload_cut_off_by_the_output_limit_is_closed() {
        // The model ran out of tokens mid-object.
        let raw = r#"{"path": "src/lib.rs", "old_string": "fn main() {"#;
        match parse_tool_arguments(raw) {
            ParsedArgs::Repaired { value, fix } => {
                assert_eq!(fix, Fix::ClosedTruncation);
                assert_eq!(value["path"], "src/lib.rs");
                assert!(
                    value.get("old_string").is_none() || value["old_string"].is_string(),
                    "the half-written value is dropped, not invented: {value}"
                );
            }
            other => panic!("expected a truncation repair, got {other:?}"),
        }
        // Unclosed braces after the string was cut.
        let raw = r#"{"edits": [{"old_string": "a", "new_string": "b""#;
        assert!(matches!(
            parse_tool_arguments(raw),
            ParsedArgs::Repaired {
                fix: Fix::ClosedTruncation,
                ..
            }
        ));
    }

    #[test]
    fn doubled_encoding_is_unwrapped_once() {
        let inner = r#"{"path": "a.rs"}"#;
        let raw = serde_json::to_string(inner).unwrap();
        match parse_tool_arguments(&raw) {
            ParsedArgs::Repaired { value, fix } => {
                assert_eq!(fix, Fix::UnwrappedString);
                assert_eq!(value["path"], "a.rs");
            }
            other => panic!("expected an unwrap, got {other:?}"),
        }
    }

    #[test]
    fn single_quotes_are_converted_only_when_that_is_unambiguous() {
        match parse_tool_arguments("{'path': 'a.rs'}") {
            ParsedArgs::Repaired { value, fix } => {
                assert_eq!(fix, Fix::SingleQuotes);
                assert_eq!(value["path"], "a.rs");
            }
            other => panic!("expected a quote conversion, got {other:?}"),
        }
        // With double quotes present, a single quote may be an apostrophe inside a value:
        // guessing would corrupt the payload, so the payload is reported instead.
        let mixed = r#"{"text": "it's fine'}}"#;
        assert!(matches!(
            parse_tool_arguments(mixed),
            ParsedArgs::Unrecoverable { .. }
        ));
    }

    #[test]
    fn an_unreadable_payload_is_handed_on_rather_than_replaced_with_nothing() {
        // The old behaviour: `{}` — the tool ran with no arguments and complained about a missing
        // field. The new behaviour: the raw text is preserved so the failure can quote it.
        for raw in [
            "not json at all",
            "{unquoted: keys}",
            "{\"a\": }",
            "[1, 2, 3]",         // valid JSON, wrong shape for a tool
            "42",                // ditto
            "\"just a string\"", // ditto (and not itself JSON)
        ] {
            match parse_tool_arguments(raw) {
                ParsedArgs::Unrecoverable { raw: kept, reason } => {
                    assert!(!reason.is_empty(), "the reason must say something: {raw}");
                    assert!(!kept.is_empty(), "the raw text is kept: {raw}");
                }
                other => panic!("{raw} must not be silently accepted: {other:?}"),
            }
        }
    }

    #[test]
    fn a_repair_that_recovers_nothing_is_refused() {
        // Every member incomplete: closing it would "succeed" with `{}` — the silent
        // empty-arguments behaviour this module exists to remove.
        let raw = r#"{"text": "it's fine'}"#;
        match parse_tool_arguments(raw) {
            ParsedArgs::Unrecoverable { raw: kept, reason } => {
                assert_eq!(kept, raw, "the model's text comes back verbatim");
                assert!(
                    reason.contains("cut before"),
                    "and the reason says where it stopped: {reason}"
                );
            }
            other => panic!("an empty recovery must be refused, got {other:?}"),
        }
        // A genuinely empty payload is not a repair: it is the call as written.
        assert_eq!(
            parse_tool_arguments("{}"),
            ParsedArgs::Valid(serde_json::json!({}))
        );
    }

    #[test]
    fn a_repair_never_invents_a_missing_field() {
        // `{"path":` has no value at all. Closing it would produce `{"path":}` — invalid, so the
        // repair must fail rather than manufacture a null the model never sent.
        assert!(matches!(
            parse_tool_arguments(r#"{"path":"#),
            ParsedArgs::Unrecoverable { .. }
        ));
    }

    #[test]
    fn duplicate_call_ids_are_renamed_deterministically() {
        let mut ids = vec![
            "call_1".to_string(),
            "call_2".to_string(),
            "call_1".to_string(),
        ];
        assert_eq!(unique_call_ids(&mut ids), 1);
        assert_eq!(ids, vec!["call_1", "call_2", "call_1#2"]);
        // The first occurrence keeps its id (results already reference it), and replaying the
        // same input gives the same names.
        let mut again = vec![
            "call_1".to_string(),
            "call_2".to_string(),
            "call_1".to_string(),
        ];
        unique_call_ids(&mut again);
        assert_eq!(again, ids);

        // Three collisions, and a name that is already taken.
        let mut many = vec![
            "c".to_string(),
            "c".to_string(),
            "c".to_string(),
            "c#2".to_string(),
        ];
        assert_eq!(unique_call_ids(&mut many), 2);
        assert_eq!(
            many[1], "c#3",
            "an id that already exists is not stolen: {many:?}"
        );
        assert_eq!(many[2], "c#4");
        assert_eq!(many[3], "c#2");

        // No duplicates, nothing renamed.
        let mut clean = vec!["a".to_string(), "b".to_string()];
        assert_eq!(unique_call_ids(&mut clean), 0);
    }

    #[test]
    fn each_fix_describes_itself() {
        for fix in [
            Fix::UnwrappedString,
            Fix::TrailingComma,
            Fix::ClosedTruncation,
            Fix::SingleQuotes,
        ] {
            assert!(!fix.note().is_empty());
        }
    }
}
