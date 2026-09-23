//! Reconciliation of a pi session file against the Epsilon file `--import` writes for it.
//!
//! M119's `Limits` section recorded one number it could not explain: counting roles *inside* the
//! imported file gave 88 user + 453 assistant + 391 tool results = 932 (which is exactly what
//! `--resume` printed), while counting the *source* gave 373 tool results, and the whole-file
//! arithmetic (1,018 imported vs 932 + 103 + 1 + 1 per-kind) did not tie out either. Two
//! explanations were guessed and neither was checked. This file checks them.
//!
//! The rule it follows is the one the M119 measurement broke: **every line of both files is
//! parsed with a real JSON parser** (`serde_json`, the same one the importer uses) and an
//! unparsable line is a hard failure, never a silent skip — a count that quietly ignores what it
//! could not read is how a real loss hides. Nothing here shells out, and nothing counts text
//! with a regex.
//!
//! Three questions, in order:
//!
//! 1. **Per type and per role, on both sides.** pi's `type` against the imported `kind.type`,
//!    and `message.role` on both sides with `toolResult` → `tool_result` normalised, because
//!    that spelling difference alone makes a naive comparison look like a loss.
//! 2. **Branch accounting.** The tree is rebuilt from `parentId`, the leaf is found by the
//!    importer's own rule (last entry no other entry names as its parent), and the chain is
//!    walked back to its root. Source entries off that chain are reported as abandoned, and the
//!    imported chain is compared against them. Whether abandoned entries *should* be imported is
//!    a design decision, not a number: `import_pi_session` writes the active chain only
//!    (`import.rs:245-251`, `ImportReport::abandoned_entries`), so this file's job is to say how
//!    many there are and to prove that the generated fixture — a linear file — has none.
//! 3. **Unmatched entries, named.** Every source entry without a counterpart in the import, and
//!    every imported entry without a source, is listed with its id, type and line so a reader
//!    can look it up instead of trusting a total.
//!
//! `references/` is not committed, so every test here skips — loudly — when pi's fixtures are
//! absent, the same rule the terminal integration test follows for `EPSILON_BIN`. Point
//! `EPSILON_RECONCILE_DIR` at a directory to keep the generated sources and their imports around
//! (the run then reconciles an import that is already there, which is how the CLI's own
//! `--import` output is checked rather than the library's).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde_json::Value;

// ---------------------------------------------------------------- plumbing

/// pi's shipped fixtures, from the reference checkout this repository reads but does not commit.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../references/pi-0.85.1/packages/coding-agent/test/fixtures")
}

/// Where the generated pi v2 sources and their imports live: `EPSILON_RECONCILE_DIR` when set,
/// else a fixed directory under the system temp dir.
fn work_dir() -> PathBuf {
    let dir = std::env::var_os("EPSILON_RECONCILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("epsilon-pi-reconciliation"));
    std::fs::create_dir_all(&dir).expect("create reconciliation directory");
    dir
}

/// Every non-blank line of a JSONL file, parsed, with its 1-based line number.
///
/// A line that is not JSON panics rather than being skipped: the M119 discrepancy was *caused*
/// by a measurement that could drop what it could not parse, and a reconciliation with the same
/// blind spot proves nothing.
fn parse_jsonl(path: &Path) -> Vec<(usize, Value)> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).unwrap_or_else(|e| {
            panic!("{}:{} is not JSON: {e}", path.display(), idx + 1);
        });
        out.push((idx + 1, value));
    }
    out
}

/// Rebuild pi's v2 envelope over a v1 fixture: every non-header line gets an `id` and a
/// `parentId` chained to the previous entry — pi's own `migrateV1ToV2`
/// (`references/pi-0.85.1/packages/coding-agent/src/core/session-manager.ts:230-242`), which is
/// what makes the file a shape `--resume` refuses and `--import` exists for.
///
/// Returns the number of entries written. Ids are derived from the line number so a reader can
/// map an entry back to the source line by eye.
///
/// Written to a temporary name and renamed into place: two tests in this binary generate the same
/// fixture concurrently, and a reader must never catch a half-written file (which would look like
/// a line this harness cannot parse — the very thing it refuses to tolerate).
fn write_pi_v2(v1: &Path, out: &Path) -> usize {
    let text = std::fs::read_to_string(v1).unwrap_or_else(|e| panic!("read {}: {e}", v1.display()));
    let mut lines = String::new();
    let mut previous: Option<String> = None;
    let mut entries = 0usize;
    let tmp_path = out.with_extension("jsonl.writing");
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("{}:{} is not JSON: {e}", v1.display(), idx + 1));
        if value.get("type").and_then(Value::as_str) == Some("session") {
            lines.push_str(line);
            lines.push('\n');
            continue;
        }
        let id = format!("e{:04}", idx + 1);
        let map = value.as_object_mut().expect("a pi entry is an object");
        map.insert("id".into(), Value::String(id.clone()));
        map.insert(
            "parentId".into(),
            previous.clone().map_or(Value::Null, Value::String),
        );
        previous = Some(id);
        entries += 1;
        lines.push_str(&serde_json::to_string(&value).expect("serialize a pi entry"));
        lines.push('\n');
    }
    std::fs::write(&tmp_path, lines)
        .unwrap_or_else(|e| panic!("write {}: {e}", tmp_path.display()));
    std::fs::rename(&tmp_path, out)
        .unwrap_or_else(|e| panic!("rename into {}: {e}", out.display()));
    entries
}

// ---------------------------------------------------------------- the comparison

/// One source entry, with the two chain fields the importer reads.
struct SourceEntry {
    line: usize,
    id: String,
    parent: Option<String>,
    value: Value,
}

/// One entry with no counterpart on the other side, named so a reader can look it up.
struct Unmatched {
    line: usize,
    id: String,
    label: String,
    /// True when the entry is off the active chain, which is *why* it has no counterpart: the
    /// importer writes the chain only. Marked rather than merged into the silent loss column.
    abandoned: bool,
}

impl std::fmt::Display for Unmatched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {} id {} {}", self.line, self.id, self.label)?;
        if self.abandoned {
            write!(f, " [off the active chain — not imported by design]")?;
        }
        Ok(())
    }
}

/// What the two files say, side by side.
struct Recon {
    source_lines: usize,
    source_entries: usize,
    source_types: BTreeMap<String, usize>,
    source_roles: BTreeMap<String, usize>,
    imported_lines: usize,
    imported_entries: usize,
    imported_kinds: BTreeMap<String, usize>,
    imported_roles: BTreeMap<String, usize>,
    on_chain: usize,
    abandoned: usize,
    unmatched_source: Vec<Unmatched>,
    unmatched_imported: Vec<Unmatched>,
    /// Imported `parent_id` links that do not point at the previous imported entry — zero for a
    /// linear source, and the cheapest proof that the import preserved the chain rather than
    /// flattening it.
    link_breaks: usize,
    /// Matched pairs whose text content differs (every string under a `text` key, sorted).
    text_mismatches: Vec<Unmatched>,
    /// Entries carrying a `thinkingSignature` this build has no field for — the one *field*-level
    /// loss the fixtures show. Named so it cannot be read as an entry-level one.
    lost_signatures: Vec<Unmatched>,
}

impl Recon {
    fn render(&self) -> String {
        let mut out = String::new();
        let rows = |map: &BTreeMap<String, usize>| -> String {
            if map.is_empty() {
                return "    (none)".to_string();
            }
            map.iter()
                .map(|(k, v)| format!("    {k:<22} {v}\n"))
                .collect()
        };
        out.push_str(&format!(
            "  lines            source {}   imported {} (incl. header line)\n",
            self.source_lines, self.imported_lines
        ));
        out.push_str(&format!(
            "  entries          source {}   imported {}\n",
            self.source_entries, self.imported_entries
        ));
        out.push_str("  source    by type:\n");
        out.push_str(&rows(&self.source_types));
        out.push_str("  source    by message.role:\n");
        out.push_str(&rows(&self.source_roles));
        out.push_str("  imported  by kind.type:\n");
        out.push_str(&rows(&self.imported_kinds));
        out.push_str("  imported  by message.role:\n");
        out.push_str(&rows(&self.imported_roles));
        out.push_str(&format!(
            "  branch           on the active chain {}   abandoned {}\n",
            self.on_chain, self.abandoned
        ));
        out.push_str(&format!("  chain links      broken {}\n", self.link_breaks));
        out.push_str(&format!(
            "  unmatched        source {}   imported {}\n",
            self.unmatched_source.len(),
            self.unmatched_imported.len()
        ));
        for u in &self.unmatched_source {
            out.push_str(&format!("    source-only:   {u}\n"));
        }
        for u in &self.unmatched_imported {
            out.push_str(&format!("    imported-only: {u}\n"));
        }
        for u in &self.text_mismatches {
            out.push_str(&format!("    text differs:  {u}\n"));
        }
        out.push_str(&format!(
            "  field loss       thinkingSignature this build has no slot for: {} entry/entries\n",
            self.lost_signatures.len()
        ));
        for u in &self.lost_signatures {
            out.push_str(&format!("    field dropped: {u}\n"));
        }
        out
    }
}

/// The label the *import* is expected to carry for one source entry: `type` or `type/role`,
/// with pi's `toolResult` normalised to Epsilon's `tool_result`.
///
/// This restates the healer's mapping (`store.rs`'s `heal_legacy_kind` / `heal_legacy_message`)
/// so a mismatch is a real disagreement rather than a spelling difference: `message` chooses its
/// variant by role, a role with no variant (pi's `bashExecution`) and a `user` message carrying a
/// part this build cannot type both heal to a `custom` message, and the two kinds whose pi shape
/// lines up field for field stay typed only when their fields are there.
fn source_label(entry: &Value) -> String {
    match entry.get("type").and_then(Value::as_str).unwrap_or("") {
        "message" => {
            let message = entry.get("message");
            match message
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                .unwrap_or("user")
            {
                "assistant" => "message/assistant".into(),
                "toolResult" => "message/tool_result".into(),
                "user" if !has_unmapped_parts(message) => "message/user".into(),
                _ => "message/custom".into(),
            }
        }
        "model_change"
            if entry.get("provider").and_then(Value::as_str).is_some()
                && entry.get("modelId").and_then(Value::as_str).is_some() =>
        {
            "model_change".into()
        }
        "thinking_level_change" if entry.get("thinkingLevel").and_then(Value::as_str).is_some() => {
            "thinking_level_change".into()
        }
        "compaction" => "compaction".into(),
        other => format!("custom/{other}"),
    }
}

/// Does this message carry a content part this build cannot type — which turns a `user` message
/// into a preserved `custom` one? (`store.rs`'s `legacy_block`, restated.)
fn has_unmapped_parts(message: Option<&Value>) -> bool {
    match message.and_then(|m| m.get("content")) {
        Some(Value::Array(parts)) => parts.iter().any(unmapped_part),
        Some(Value::String(_)) | Some(Value::Null) | None => false,
        // A shape that is neither an array nor a string is preserved whole.
        Some(_) => true,
    }
}

fn unmapped_part(part: &Value) -> bool {
    match part.get("type").and_then(Value::as_str) {
        Some("text") => part.get("text").and_then(Value::as_str).is_none(),
        Some("thinking") => part.get("thinking").and_then(Value::as_str).is_none(),
        Some("toolCall" | "tool_call") => part.get("id").and_then(Value::as_str).is_none(),
        _ => true,
    }
}

/// The label an imported entry actually carries.
fn imported_label(entry: &Value) -> String {
    let kind = entry.get("kind").unwrap_or(&Value::Null);
    match kind.get("type").and_then(Value::as_str).unwrap_or("") {
        "message" => format!(
            "message/{}",
            kind.get("message")
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                .unwrap_or("?")
        ),
        "custom" => format!(
            "custom/{}",
            kind.get("data")
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ),
        other => other.to_string(),
    }
}

/// Every string under a `text` key — and under `thinking`, which is the same content spelled the
/// other way (pi's `{"type":"thinking","thinking":…}` against Epsilon's `{"type":"thinking",
/// "text":…}`) — sorted. The content check that catches a conversion which keeps the entry count
/// and empties the messages (the G19 defect); without the second key it reports every reasoning
/// block as a difference.
fn text_of(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if (key == "text" || key == "thinking")
                        && let Value::String(s) = child
                    {
                        out.push(s.clone());
                        continue;
                    }
                    walk(child, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    walk(value, &mut out);
    out.sort();
    out
}

/// How many thinking parts of this message carry a non-empty `thinkingSignature`.
///
/// pi writes one per reasoning block — an opaque provider signature Anthropic needs to replay the
/// block. [`epsilon_contracts::ContentBlock::Thinking`] has no field for it, so it is the one
/// thing this importer does not carry: a *field*, not an entry, which is why it is counted and
/// named separately instead of being folded into the entry totals.
fn signatures_in(entry: &Value) -> usize {
    entry
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|part| {
                    part.get("type").and_then(Value::as_str) == Some("thinking")
                        && part
                            .get("thinkingSignature")
                            .and_then(Value::as_str)
                            .is_some_and(|signature| !signature.is_empty())
                })
                .count()
        })
        .unwrap_or(0)
}

/// The active chain of a pi archive: the leaf walked back to its root, as *file indices*, oldest
/// first — the importer's own rule (`import.rs`'s `active_chain`), restated here so the
/// comparison does not have to trust the thing it is checking.
fn chain_indices(entries: &[SourceEntry], header: Option<&Value>) -> Vec<usize> {
    let leaf = header
        .and_then(|h| {
            ["leafId", "leaf_id", "leaf"]
                .iter()
                .find_map(|key| h.get(*key).and_then(Value::as_str))
        })
        .filter(|id| entries.iter().any(|e| e.id == *id))
        .map(str::to_string)
        .or_else(|| {
            let named: BTreeSet<&str> =
                entries.iter().filter_map(|e| e.parent.as_deref()).collect();
            entries
                .iter()
                .rfind(|e| !named.contains(e.id.as_str()))
                .or(entries.last())
                .map(|e| e.id.clone())
        });
    let by_id: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();
    let mut chain: Vec<usize> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut cursor = leaf;
    while let Some(id) = cursor {
        let Some(&idx) = by_id.get(id.as_str()) else {
            break;
        };
        if !seen.insert(entries[idx].id.as_str()) {
            break;
        }
        cursor = entries[idx].parent.clone();
        chain.push(idx);
    }
    chain.reverse();
    chain
}

/// Parse both files and compare them, entry by entry, along the chain.
fn reconcile(source: &Path, imported: &Path) -> Recon {
    let source_lines = parse_jsonl(source);
    let imported_lines = parse_jsonl(imported);

    // --- source side: the header, then the entries with their chain resolved.
    let header = source_lines
        .iter()
        .find(|(_, v)| v.get("type").and_then(Value::as_str) == Some("session"))
        .map(|(_, v)| v.clone());
    let mut entries: Vec<SourceEntry> = Vec::new();
    let mut source_types: BTreeMap<String, usize> = BTreeMap::new();
    let mut source_roles: BTreeMap<String, usize> = BTreeMap::new();
    for (line, value) in &source_lines {
        if value.get("type").and_then(Value::as_str) == Some("session") {
            continue;
        }
        *source_types
            .entry(
                value
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
            )
            .or_default() += 1;
        if value.get("type").and_then(Value::as_str) == Some("message") {
            *source_roles
                .entry(
                    value
                        .get("message")
                        .and_then(|m| m.get("role"))
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string(),
                )
                .or_default() += 1;
        }
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("pi-v1-line-{line}"));
        let parent = match value.get("parentId") {
            Some(Value::String(p)) => Some(p.clone()),
            Some(_) => None,
            None => entries.last().map(|e: &SourceEntry| e.id.clone()),
        };
        entries.push(SourceEntry {
            line: *line,
            id,
            parent,
            value: value.clone(),
        });
    }
    let chain = chain_indices(&entries, header.as_ref());
    let on_chain: BTreeSet<&str> = chain.iter().map(|&i| entries[i].id.as_str()).collect();

    // --- imported side: entries in `seq` order, kinds and roles as written.
    let mut imported: Vec<&Value> = Vec::new();
    let mut imported_kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut imported_roles: BTreeMap<String, usize> = BTreeMap::new();
    for (_, value) in &imported_lines {
        // The import's own header line: `{"type":"session",…}` with no `kind` object.
        if value.get("kind").is_none() {
            continue;
        }
        *imported_kinds
            .entry(
                value
                    .get("kind")
                    .and_then(|k| k.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
            )
            .or_default() += 1;
        if value
            .get("kind")
            .and_then(|k| k.get("type"))
            .and_then(Value::as_str)
            == Some("message")
        {
            *imported_roles
                .entry(
                    value
                        .get("kind")
                        .and_then(|k| k.get("message"))
                        .and_then(|m| m.get("role"))
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string(),
                )
                .or_default() += 1;
        }
        imported.push(value);
    }
    imported.sort_by_key(|v| v.get("seq").and_then(Value::as_u64).unwrap_or(u64::MAX));

    // --- the comparison, position by position along the chain.
    let mut unmatched_source = Vec::new();
    let mut unmatched_imported = Vec::new();
    let mut text_mismatches = Vec::new();
    let mut lost_signatures = Vec::new();
    for (position, &idx) in chain.iter().enumerate() {
        let entry = &entries[idx];
        let label = source_label(&entry.value);
        match imported.get(position) {
            Some(other) => {
                let other_label = imported_label(other);
                if other_label != label {
                    unmatched_source.push(Unmatched {
                        line: entry.line,
                        id: entry.id.clone(),
                        label: label.clone(),
                        abandoned: false,
                    });
                    unmatched_imported.push(Unmatched {
                        line: 0,
                        id: other
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("?")
                            .to_string(),
                        label: other_label,
                        abandoned: false,
                    });
                } else {
                    if label != "message/custom"
                        && text_of(&entry.value)
                            != text_of(other.get("kind").unwrap_or(&Value::Null))
                    {
                        text_mismatches.push(Unmatched {
                            line: entry.line,
                            id: entry.id.clone(),
                            label: label.clone(),
                            abandoned: false,
                        });
                    }
                    let signatures = signatures_in(&entry.value);
                    if signatures > 0 {
                        lost_signatures.push(Unmatched {
                            line: entry.line,
                            id: entry.id.clone(),
                            label: format!(
                                "{label} — {signatures} thinking part(s) with a provider signature \
                                 this build has no field for"
                            ),
                            abandoned: false,
                        });
                    }
                }
            }
            None => unmatched_source.push(Unmatched {
                line: entry.line,
                id: entry.id.clone(),
                label,
                abandoned: false,
            }),
        }
    }
    // Entries off the active chain have no counterpart either — named here, marked as the design
    // decision they are, so "abandoned" never hides inside a count that looks like loss.
    for entry in entries
        .iter()
        .filter(|entry| !on_chain.contains(entry.id.as_str()))
    {
        unmatched_source.push(Unmatched {
            line: entry.line,
            id: entry.id.clone(),
            label: source_label(&entry.value),
            abandoned: true,
        });
    }
    for (position, other) in imported.iter().enumerate().skip(chain.len()) {
        unmatched_imported.push(Unmatched {
            line: position + 1,
            id: other
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            label: imported_label(other),
            abandoned: false,
        });
    }
    let link_breaks = imported
        .windows(2)
        .filter(|pair| {
            pair[1].get("parent_id").and_then(Value::as_str)
                != pair[0].get("id").and_then(Value::as_str)
        })
        .count();

    Recon {
        source_lines: source_lines.len(),
        source_entries: entries.len(),
        source_types,
        source_roles,
        imported_lines: imported_lines.len(),
        imported_entries: imported.len(),
        imported_kinds,
        imported_roles,
        on_chain: chain.len(),
        abandoned: entries.len() - chain.len(),
        unmatched_source,
        unmatched_imported,
        link_breaks,
        text_mismatches,
        lost_signatures,
    }
}

// ---------------------------------------------------------------- the tests

/// The import for `source`: written now, or reused when it is already on disk.
///
/// Cargo runs every test in this binary concurrently and two of them reconcile these same
/// fixtures; a run with `EPSILON_RECONCILE_DIR` set reconciles bytes the CLI wrote earlier still.
/// The importer refuses to replace an existing output — by design, and that refusal is the point
/// — so "it is already there" is a normal case here, not a failure. Returns the headline to print.
fn ensure_import(source: &Path, expected_entries: usize) -> String {
    let imported = epsilon_session::import_output_path(source);
    let reuse = |why: &str| format!("reconciling {why} {}", imported.display());
    if imported.exists() {
        return reuse("the import already at");
    }
    match epsilon_session::import_pi_session(source) {
        Ok(report) => {
            assert_eq!(
                report.entries, expected_entries,
                "the import's own count disagrees with the source's entry count"
            );
            assert_eq!(
                report.abandoned_entries, 0,
                "the generated fixture chains every entry, so nothing is off the chain"
            );
            report.headline()
        }
        // Another test in this binary won the race for the same output file; it wrote the same
        // import, so reconciling what is there is the honest thing to do.
        Err(_) if imported.exists() => reuse("the import another test wrote to"),
        Err(e) => panic!("import {}: {e}", source.display()),
    }
}

/// The importer carries every entry of a linear pi file, once, with its role intact.
///
/// Runs against both of pi's fixtures: `large-session.jsonl` (1,018 entries) is the file M119
/// measured, and `before-compaction.jsonl` (1,002 entries, with compaction, `bashExecution` and
/// unknown-to-this-build shapes) is the second file that tells a systematic rule from a one-off.
#[test]
fn pi_import_carries_every_source_entry() {
    let fixtures = fixtures_dir();
    if !fixtures.is_dir() {
        eprintln!(
            "SKIP: {} is absent — references/ is not committed",
            fixtures.display()
        );
        return;
    }
    for name in ["large-session", "before-compaction"] {
        let v1 = fixtures.join(format!("{name}.jsonl"));
        if !v1.is_file() {
            eprintln!("SKIP: {} is absent", v1.display());
            continue;
        }
        let source = work_dir().join(format!("{name}.v2.jsonl"));
        let entries = write_pi_v2(&v1, &source);
        let imported = epsilon_session::import_output_path(&source);
        let headline = ensure_import(&source, entries);
        let recon = reconcile(&source, &imported);
        println!("=== {name}: {headline} ===\n{}", recon.render());
        assert!(
            recon.unmatched_source.is_empty(),
            "entries in the source have no counterpart in the import:\n{}",
            recon.render()
        );
        assert!(
            recon.unmatched_imported.is_empty(),
            "the import carries entries the source does not:\n{}",
            recon.render()
        );
        assert!(
            recon.text_mismatches.is_empty(),
            "matched entries whose content differs:\n{}",
            recon.render()
        );
        assert_eq!(
            recon.source_entries,
            recon.imported_entries,
            "entry counts differ:\n{}",
            recon.render()
        );
        assert_eq!(recon.abandoned, 0, "the generated fixture is linear");
        assert_eq!(recon.link_breaks, 0, "the imported chain is not linked");
    }
}

/// The M119 gap was a *measurement artefact*, not a loss: the file whose roles were counted had
/// already been through `--resume`, whose repair pass appends one synthetic tool result per
/// dangling tool call plus a `TurnEnd{Aborted}` marker (`repair.rs:88-167`, called from
/// `main.rs:1741` before the "resumed N entries" line). Those appended entries are then compared
/// against the import's own report — taken *before* the repair — and the difference looks like
/// the importer dropping 18 entries.
///
/// This test reproduces the whole sequence on one file and names the arithmetic: import (exact),
/// reconcile (nothing unmatched), then the resume pass, then reconcile again — where the only
/// unmatched imported entries are exactly the ones repair appended.
#[test]
fn the_m119_gap_is_resume_repair_not_import_loss() {
    let fixtures = fixtures_dir();
    let v1 = fixtures.join("large-session.jsonl");
    if !v1.is_file() {
        eprintln!(
            "SKIP: {} is absent — references/ is not committed",
            v1.display()
        );
        return;
    }
    let source = work_dir().join("large-session.repair.v2.jsonl");
    write_pi_v2(&v1, &source);
    let imported = epsilon_session::import_output_path(&source);
    if imported.exists() {
        std::fs::remove_file(&imported).expect("clear the previous import");
    }
    let report = epsilon_session::import_pi_session(&source).expect("import a pi file");

    // Before resume: the import against its source, entry for entry.
    let fresh = reconcile(&source, &imported);
    assert!(
        fresh.unmatched_source.is_empty() && fresh.unmatched_imported.is_empty(),
        "the import itself lost or invented entries:\n{}",
        fresh.render()
    );
    assert_eq!(fresh.source_roles.get("toolResult"), Some(&373));
    assert_eq!(fresh.imported_roles.get("tool_result"), Some(&373));

    // What `--resume` does before it prints "resumed N entries from …".
    let mut store = epsilon_session::SessionStore::open(&imported).expect("open the import");
    let repair = epsilon_session::repair::repair_on_open(&mut store).expect("repair pass");
    let resumed = store.model_view().expect("model view").len();

    // After resume: the same reconciliation, on the file the resume path left behind.
    let after = reconcile(&source, &imported);
    println!(
        "=== large-session, after the `--resume` repair pass ===\n\
         import reported {} entries; repair added {} tool result(s){}; model view {}\n{}",
        report.entries,
        repair.repaired_tool_calls,
        if repair.appended_turn_end {
            " and a turn-end marker"
        } else {
            ""
        },
        resumed,
        after.render()
    );

    // The source still has no unmatched entries — nothing the repair appended came from the
    // source, and nothing the source held was dropped.
    assert!(
        after.unmatched_source.is_empty(),
        "the source lost entries across the resume pass:\n{}",
        after.render()
    );
    // The imported-only entries are exactly repair's own appends.
    assert_eq!(
        after.unmatched_imported.len(),
        repair.repaired_tool_calls + usize::from(repair.appended_turn_end),
        "the entries the import has and the source does not are not repair's:\n{}",
        after.render()
    );
    let appended_tool_results = after
        .unmatched_imported
        .iter()
        .filter(|u| u.label == "message/tool_result")
        .count();
    assert_eq!(appended_tool_results, repair.repaired_tool_calls);
    assert!(
        after
            .unmatched_imported
            .iter()
            .all(|u| u.label == "message/tool_result" || u.label == "turn_end"),
        "repair appended something other than tool results and a turn end:\n{}",
        after.render()
    );
    // And the M119 numbers, now accounted for: 914 source messages + 18 synthetic tool results
    // = 932, which is what `--resume` printed.
    assert_eq!(
        repair.repaired_tool_calls, 18,
        "the fixture's 18 dangling tool calls are what M119 counted as 391 tool results"
    );
    assert!(repair.appended_turn_end);
    assert_eq!(after.imported_roles.get("tool_result"), Some(&391));
    assert_eq!(resumed, 932, "the number `--resume` printed");
    assert_eq!(
        after.imported_entries,
        report.entries + repair.repaired_tool_calls + 1
    );
    assert_eq!(after.imported_kinds.get("turn_end"), Some(&1));
    assert_eq!(
        after.imported_entries,
        after.imported_kinds.values().sum::<usize>(),
        "the post-resume per-kind counts sum to the post-resume entry count"
    );
}

/// Abandoned branches are not imported — and that is a *decision the report states*, not a count
/// that quietly shrinks. `import_pi_session` writes the active chain only (`import.rs:245-251`)
/// and counts everything else in `ImportReport::abandoned_entries`, whose summary line tells the
/// user the source still holds them.
///
/// pi's own fixtures are linear, so this builds the branched shape by hand: `a → b → {d, c}`,
/// where pi navigated back to `b` and continued at `c`. The leaf rule (the *last* entry no other
/// entry names as its parent) picks `c`, which makes `d` the abandoned entry.
#[test]
fn abandoned_branches_are_reported_not_imported() {
    let source = work_dir().join("branched.v2.jsonl");
    let imported = epsilon_session::import_output_path(&source);
    let _ = std::fs::remove_file(&imported);
    let message = |id: &str, parent: &str, role: &str, text: &str| {
        format!(
            "{{\"type\":\"message\",\"id\":\"{id}\",\"parentId\":{parent},\
             \"timestamp\":\"2025-12-08T22:41:05.306Z\",\
             \"message\":{{\"role\":\"{role}\",\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}],\
             \"timestamp\":1765230065306}}}}"
        )
    };
    let file = [
        "{\"type\":\"session\",\"id\":\"ses-branch\",\"timestamp\":\"2025-12-08T22:41:05.306Z\",\"cwd\":\"/tmp\"}".to_string(),
        message("a", "null", "user", "one"),
        message("b", "\"a\"", "assistant", "two"),
        message("d", "\"b\"", "assistant", "abandoned"),
        message("c", "\"b\"", "assistant", "three"),
    ]
    .join("\n")
        + "\n";
    std::fs::write(&source, file).expect("write the branched fixture");

    let report = epsilon_session::import_pi_session(&source).expect("import a pi file");
    let recon = reconcile(&source, &imported);
    println!(
        "=== branched ===\n{}\n  {}",
        recon.render(),
        report.summary().join("\n  ")
    );

    assert_eq!(recon.on_chain, 3, "a, b and c are the active chain");
    assert_eq!(recon.abandoned, 1, "d hangs off b and is not on the chain");
    assert_eq!(report.entries, 3);
    assert_eq!(report.abandoned_entries, 1);
    let unmatched: Vec<&str> = recon
        .unmatched_source
        .iter()
        .map(|u| u.id.as_str())
        .collect();
    assert_eq!(
        unmatched,
        vec!["d"],
        "the abandoned entry is the one named as missing:\n{}",
        recon.render()
    );
    assert!(
        recon.unmatched_source[0].abandoned,
        "the missing entry is off the chain, which is why it is missing — not an unexplained drop"
    );
    assert!(
        !std::fs::read_to_string(&imported)
            .expect("read the import")
            .contains("abandoned"),
        "an abandoned branch entry reached the import"
    );
    assert!(
        recon.unmatched_imported.is_empty(),
        "the import invented entries:\n{}",
        recon.render()
    );
}

/// The harness, pointed at a pair of files it did not make.
///
/// This is the entry point for artefacts produced by the real binary — `epsilon --import` and the
/// `--resume` that follows it — which is the only way to check the bytes a user would actually
/// have. Set `EPSILON_RECONCILE_PAIR` to `source.jsonl;imported.jsonl` (semicolon-separated) and
/// the run prints the same table the fixture tests print. It asserts nothing: the caller chose the
/// pair, and the numbers are the evidence.
#[test]
fn reconcile_an_external_pair() {
    let Some(pair) = std::env::var_os("EPSILON_RECONCILE_PAIR") else {
        eprintln!("SKIP: EPSILON_RECONCILE_PAIR is not set (`source.jsonl;imported.jsonl`)");
        return;
    };
    let pair = pair.to_string_lossy().to_string();
    let (source, imported) = pair
        .split_once(';')
        .expect("EPSILON_RECONCILE_PAIR is `source.jsonl;imported.jsonl`");
    let recon = reconcile(Path::new(source), Path::new(imported));
    println!("=== {source} vs {imported} ===\n{}", recon.render());
}

/// The source-side detail the M119 arithmetic could not tie out: per kind, the import equals the
/// source, and the only additions are repair's. Kept separate from the two tests above because
/// it is an assertion about *both* fixtures at once — a rule, not an anecdote.
#[test]
fn per_kind_counts_tie_out_on_both_sides() {
    let fixtures = fixtures_dir();
    if !fixtures.is_dir() {
        eprintln!(
            "SKIP: {} is absent — references/ is not committed",
            fixtures.display()
        );
        return;
    }
    for name in ["large-session", "before-compaction"] {
        let v1 = fixtures.join(format!("{name}.jsonl"));
        if !v1.is_file() {
            continue;
        }
        let source = work_dir().join(format!("{name}.v2.jsonl"));
        let entries = write_pi_v2(&v1, &source);
        ensure_import(&source, entries);
        let imported = epsilon_session::import_output_path(&source);
        let recon = reconcile(&source, &imported);
        assert_eq!(
            recon.source_entries,
            recon.source_types.values().sum::<usize>(),
            "the source's own per-type counts do not sum to its entry count"
        );
        assert_eq!(
            recon.imported_entries,
            recon.imported_kinds.values().sum::<usize>(),
            "the import's per-kind counts do not sum to its entry count:\n{}",
            recon.render()
        );
        assert_eq!(
            recon.source_entries,
            recon.imported_entries,
            "import is not entry-for-entry:\n{}",
            recon.render()
        );
    }
}
