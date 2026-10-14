//! Importing a pi session file into an Epsilon session file — as a **copy**.
//!
//! pi ships two session envelopes. The older one (`{"type":"message","timestamp":…}` with no
//! `id`) is pi's v1, and this build already heals it — in place, behind a `.pre-epsilon` backup
//! (M114/M115). The current one is what pi's own `migrateV1ToV2` produces
//! (`references/pi-0.85.1/packages/coding-agent/src/core/session-manager.ts:231-242`): every
//! entry carries `id` and `parentId`, the kind sits at the **top level** (`type`), and the
//! entries form the branch tree pi's `/tree` navigates. That shape has no `seq`, no `kind`
//! object and an ISO `timestamp`, so [`SessionStore::open`](crate::SessionStore::open) refuses
//! it at line 2 — honestly ("a shape this build does not know"), but with nothing the user can
//! do about it.
//!
//! This module is the thing they can do. It reads the pi file, converts every entry through the
//! *same* healing the v1 path uses (`heal_legacy_kind` in [`crate::store`]) and writes a
//! brand-new Epsilon session beside it. The source is opened read-only and never written to —
//! not even to back it up, because the point of an import is that the file you imported from is
//! still the file you imported from.
//!
//! Why import-as-copy rather than heal-in-place: both formats call themselves version 3
//! (`session-manager.ts:30` and [`CURRENT_SESSION_VERSION`](epsilon_contracts::session_entry::CURRENT_SESSION_VERSION)), so the version field cannot tell
//! them apart, and pi cannot read what a conversion would leave behind. The plan that settled
//! this is `audit/evidence/session-format-gap-plan.md` §4B; the evidence for why a second
//! converter is not an option is `audit/evidence/pi-import-empties-messages-2026-09-22.md`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use epsilon_contracts::message::Message;
use epsilon_contracts::session_entry::{
    CURRENT_SESSION_VERSION, EntryKind, SessionEntry, SessionHeader, SessionHeaderKind, SessionLine,
};

use crate::store::{
    LEGACY_EXTRAS_KEY, LineDiagnosis, StoreError, heal_legacy_kind, healed_entry_timestamp,
    iso_from_ms, legacy_timestamp_ms, parse_iso8601_ms,
};

/// Suffix of the file an import writes, appended to the whole source name so the source's own
/// extension survives: `pi.jsonl` → `pi.jsonl.epsilon.jsonl`, `session` → `session.epsilon.jsonl`.
const IMPORT_SUFFIX: &str = ".epsilon.jsonl";

/// The path [`import_pi_session`] writes for a given pi session file.
///
/// Public for the same reason [`crate::original_backup_path`] is: the summary names the output,
/// and a frontend that re-spelled the suffix would eventually name a file the import did not
/// write. The name is derived from the *whole* source name, so importing files that differ only
/// by extension cannot collide.
pub fn import_output_path(source: &Path) -> PathBuf {
    let mut name = source.as_os_str().to_os_string();
    name.push(IMPORT_SUFFIX);
    PathBuf::from(name)
}

/// What an import did, and what it could not carry — the report a frontend prints.
///
/// Every field exists because the corresponding fact would otherwise be invisible: a converted
/// transcript that quietly drops an entry kind, an unmapped content part or an abandoned branch
/// looks exactly like a complete one. The same rule the store applies to a preserved unknown
/// entry (M113) and to a repair report (M75) applies here — say what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// The pi file that was read. Never written to.
    pub source: PathBuf,
    /// The new Epsilon session file that was written.
    pub output: PathBuf,
    /// Entries written to the output (the active branch, root to leaf).
    pub entries: usize,
    /// The pi `type` tags that were preserved rather than typed, in file order, deduplicated.
    /// Each is reachable inside its entry's `data`; none of them reaches a model view.
    pub preserved_kinds: Vec<String>,
    /// Content parts this build could not type, kept verbatim inside their message. Counted
    /// rather than listed: one image per turn is a number, not a list of images.
    pub unmapped_parts: usize,
    /// Entries on branches the active chain does not pass through. Reported, never imported.
    pub abandoned_entries: usize,
    /// pi's own instant for the active leaf, when the file's entries carried one — the last
    /// moment of the session, not the moment of the import.
    pub leaf_timestamp_ms: Option<u64>,
}

impl ImportReport {
    /// The one-line headline: how many entries landed in which file.
    pub fn headline(&self) -> String {
        format!(
            "imported {} entries into {} (from {})",
            self.entries,
            self.output.display(),
            self.source.display()
        )
    }

    /// The whole report, one line per fact, in the voice of
    /// [`crate::SessionStore::preserved_unknown_notice`] and the repair report.
    ///
    /// Silent about the things that went right: a notice that says "nothing was lost" on every
    /// successful import is noise, and noise is how the one import that *did* lose something
    /// gets skimmed past. The two lines that are always there are the two a user needs — where
    /// the session landed, and that their original pi file is still theirs to delete.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = vec![
            self.headline(),
            "the source was not modified — delete it yourself once you are happy with the copy"
                .to_string(),
        ];
        if !self.preserved_kinds.is_empty() {
            let list = self
                .preserved_kinds
                .iter()
                .map(|kind| format!("`{kind}`"))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "preserved, not applied: {list} — kept inside their entries and out of the \
                 transcript, because this build has no variant for them"
            ));
        }
        if self.unmapped_parts > 0 {
            lines.push(format!(
                "preserved, not typed: {} content part(s) this build cannot express — kept \
                 verbatim in their message",
                self.unmapped_parts
            ));
        }
        if self.abandoned_entries > 0 {
            lines.push(format!(
                "not imported: {} entry/entries on a branch the active chain does not pass \
                 through — the source still holds them, and the leaf rule (explicit marker, \
                 else the last unnamed entry) chose this chain",
                self.abandoned_entries
            ));
        }
        lines
    }
}

/// One parsed line of a pi session file, with its chain resolved inside the file's ids.
#[derive(Debug, Clone)]
struct SourceEntry {
    /// pi's entry id, as written; a pi v1 line (which has none) gets one derived from its line
    /// number, which is where that line number stays reachable.
    id: String,
    /// pi's `parentId`, as written (`None` for a root or for a v1 line's predecessor chain).
    parent: Option<String>,
    /// The epoch-millisecond instant, from the line's stamp or the message's finer one.
    timestamp_ms: u64,
    /// The whole entry object, for the shared healer (and for the preserved-entry fallback).
    raw: serde_json::Value,
}

/// Everything a pi session file holds, parsed.
#[derive(Debug, Clone)]
struct PiArchive {
    /// The header line, when the file has one (pi's v1 sessions do not).
    header: Option<SessionHeader>,
    /// The entries, in file order.
    entries: Vec<SourceEntry>,
}

/// Is this line one of pi's *tree* entries — the v2+ envelope this build refuses today?
///
/// The predicate is the shape, never the version: pi and Epsilon both call their format version
/// 3 and the two are different formats under one number (`session-manager.ts:30` vs
/// [`CURRENT_SESSION_VERSION`]), so a version field can never decide this. What can: a string
/// `type`, a string `id` and a `parentId` key (string or `null`) — pi's `SessionEntryBase`
/// (`session-manager.ts:46-51`).
///
/// `timestamp` is deliberately **not** required: a line missing it is still a pi tree entry,
/// and telling the user "I do not know this shape" about a file whose import would work is the
/// kind of half-diagnosis this predicate exists to end.
pub fn is_pi_v2_entry_shape(value: &serde_json::Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    // A header is not an entry, and `id` alone cannot tell them apart: both carry one.
    if map.get("type").and_then(|t| t.as_str()) == Some("session") {
        return false;
    }
    map.get("type").and_then(|t| t.as_str()).is_some()
        && map.get("id").and_then(|i| i.as_str()).is_some()
        && map
            .get("parentId")
            .is_some_and(|p| p.is_null() || p.is_string())
}

/// Does this file hold at least one line in pi's tree envelope?
///
/// A yes/no for callers that need to phrase their own sentence (the CLI's `--import` failure
/// path asks "was this even a pi file?"). Deliberately cheap and shallow — the first entry-shaped
/// line decides it — because its answer is only ever used to choose between two messages, and a
/// caller that needs certainty about a whole file is running [`import_pi_session`] anyway.
///
/// Not the same question as [`is_pi_v2_entry_shape`], which judges one parsed value: this one
/// reads the file, finds the first non-blank line after any header, and judges that.
pub fn looks_like_pi_session(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut seen_header = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if !seen_header && matches!(SessionLine::parse(line), Some(Ok(SessionLine::Header(_)))) {
            seen_header = true;
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim_end()) else {
            return false;
        };
        return is_pi_v2_entry_shape(&value);
    }
    false
}

/// Import a pi session file (any version) into a new Epsilon session file beside it.
///
/// Writes `<source>.epsilon.jsonl` (see [`import_output_path`]) and **never writes the
/// source**: it is opened, read, and left alone. Refuses to overwrite an existing output — the
/// same "never replace what is already there" rule the `.pre-epsilon` backup follows, for the
/// same reason: the file on disk may be the only copy of a session, and a second import is not
/// a reason to destroy the first one's result.
///
/// The entries written are the **active branch**: pi's leaf (an explicit header marker when the
/// file carries one — pi's own coding agent writes none — otherwise the entry no other entry
/// names as its parent, and among several such entries the *last* in file order rather than the
/// first) walked back to its root. Entries on other branches are counted in
/// [`ImportReport::abandoned_entries`] and left in the source, where they still are.
///
/// Errors are [`StoreError`]: an I/O failure, a line that is not whole JSON, or an existing
/// output file.
pub fn import_pi_session(source: &Path) -> Result<ImportReport, StoreError> {
    let archive = read_pi_archive(source)?;
    let output = import_output_path(source);
    if output.exists() {
        return Err(StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "{} already exists — refusing to replace it. Move it aside, or import a copy of \
                 the source under a different name",
                output.display()
            ),
        )));
    }
    let header = imported_header(&archive);
    let chain = active_chain(&archive);
    let by_id: HashMap<&str, &SourceEntry> = archive
        .entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();
    let on_chain: BTreeSet<&str> = chain.iter().map(|entry| entry.id.as_str()).collect();

    // Fresh ids, one id space. The output is a *new* session: pi's ids belong to that file, and
    // reusing them would make two files claim the same entries (and a later fork of one of them
    // claim an ancestry from the other). A parent link that names an entry outside the chain —
    // an abandoned sibling's child, a hand-edited dangling reference — keeps pointing at
    // nothing rather than being re-pointed at something that is not its ancestor: `branch_entries`
    // stops at a link it cannot resolve, which is the tolerant answer.
    let fresh: BTreeMap<&str, String> = archive
        .entries
        .iter()
        .map(|entry| (entry.id.as_str(), epsilon_contracts::new_id("ent")))
        .collect();

    let mut preserved_kinds: Vec<String> = Vec::new();
    let mut unmapped_parts = 0usize;
    let mut out = String::new();
    out.push_str(&serde_json::to_string(&header).map_err(json_err)?);
    out.push('\n');
    for (seq, entry) in chain.iter().enumerate() {
        let kind = heal_pi_entry_kind(&entry.raw, entry.timestamp_ms, &by_id);
        record_preservation(&kind, &mut preserved_kinds, &mut unmapped_parts);
        let written = SessionEntry {
            id: fresh
                .get(entry.id.as_str())
                .cloned()
                .unwrap_or_else(|| epsilon_contracts::new_id("ent")),
            seq: seq as u64,
            parent_id: entry.parent.as_deref().and_then(|p| fresh.get(p).cloned()),
            timestamp_ms: entry.timestamp_ms,
            kind,
        };
        out.push_str(&serde_json::to_string(&written).map_err(json_err)?);
        out.push('\n');
    }

    // tmp + rename, the same discipline as `persist_migration`: a failure mid-write must not
    // leave a half-session where a session is expected.
    let tmp = output.with_extension("jsonl.importing");
    if let Err(e) = std::fs::write(&tmp, out) {
        let _ = std::fs::remove_file(&tmp);
        return Err(StoreError::Io(e));
    }
    if let Err(e) = std::fs::rename(&tmp, &output) {
        let _ = std::fs::remove_file(&tmp);
        return Err(StoreError::Io(e));
    }

    Ok(ImportReport {
        source: source.to_path_buf(),
        output,
        entries: chain.len(),
        preserved_kinds,
        unmapped_parts,
        abandoned_entries: archive
            .entries
            .iter()
            .filter(|entry| !on_chain.contains(entry.id.as_str()))
            .count(),
        leaf_timestamp_ms: chain.last().map(|entry| entry.timestamp_ms),
    })
}

/// Serializing a session line cannot fail for any value this module builds; if it somehow did,
/// it is an I/O-shaped failure at the point the bytes were to be written.
fn json_err(e: serde_json::Error) -> StoreError {
    StoreError::Io(std::io::Error::other(e))
}

/// Read a pi session file (v1, v2 or v3) into entries with their chain resolved.
///
/// One reader for both envelopes, because the difference between them is exactly one field: v1
/// has no `id`. An entry without one gets a synthetic id derived from its line number, and its
/// `parentId` comes from file order — which is what pi's own `migrateV1ToV2` does with the same
/// input (`session-manager.ts:231-242`), and how pi chains a headerless file.
///
/// A line that is not whole JSON is an error rather than debris: an import is a deliberate act
/// on a file the user chose, so a truncated tail is something to tell them about, not to
/// silently drop. (The store's torn-tail rule answers a different question — whether a file can
/// be *opened* without inventing an ending.)
fn read_pi_archive(source: &Path) -> Result<PiArchive, StoreError> {
    let text = std::fs::read_to_string(source)?;
    let mut header: Option<SessionHeader> = None;
    let mut entries: Vec<SourceEntry> = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        if line.trim().is_empty() {
            continue;
        }
        let Some(parsed) = SessionLine::parse(line) else {
            continue; // blank, already skipped
        };
        // The header is the first line that parses as one; pi writes exactly one, so a second
        // is refused below as a second header rather than silently replacing the first.
        if header.is_none()
            && let Ok(SessionLine::Header(h)) = parsed
        {
            header = Some(h);
            continue;
        }
        // Raw JSON, not `SessionLine`: the whole point of the import is the lines that enum
        // *refuses* (a pi tree entry has no `seq` and no `kind`), so the shape is read here.
        let value: serde_json::Value = serde_json::from_str(line.trim_end())
            .map_err(|_| StoreError::corrupt(line_no, LineDiagnosis::NotJson))?;
        let Some(map) = value.as_object() else {
            return Err(StoreError::corrupt(
                line_no,
                LineDiagnosis::UnknownLineShape,
            ));
        };
        let Some(kind) = map.get("type").and_then(|t| t.as_str()) else {
            return Err(StoreError::corrupt(
                line_no,
                LineDiagnosis::UnknownLineShape,
            ));
        };
        if kind == "session" {
            return Err(StoreError::corrupt(line_no, LineDiagnosis::SecondHeader));
        }
        // An Epsilon entry is not a pi entry, and importing one would be worse than refusing:
        // its chain lives in `parent_id`, which this reader does not follow, so every entry
        // would land as an unrelated root — a session whose tree had been quietly flattened.
        // The useful answer is the command that does open it.
        if map.contains_key("parent_id") || map.contains_key("seq") || map.contains_key("kind") {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "line {line_no} is an Epsilon entry (`seq`/`parent_id`/`kind`), not a pi one \
                     — there is nothing to import. Open it with: epsilon --resume {}",
                    source.display()
                ),
            )));
        }
        let id = match map.get("id").and_then(|i| i.as_str()) {
            Some(id) => id.to_string(),
            None => format!("pi-v1-line-{line_no}"),
        };
        let parent = match map.get("parentId") {
            Some(serde_json::Value::String(p)) => Some(p.clone()),
            // An explicit `null` is pi saying "this entry starts a root". An absent `parentId`
            // means the line predates the tree, so file order is the only record of its chain.
            Some(_) => None,
            None => entries.last().map(|previous| previous.id.clone()),
        };
        entries.push(SourceEntry {
            id,
            parent,
            timestamp_ms: healed_entry_timestamp(Some(map)),
            raw: value,
        });
    }
    Ok(PiArchive { header, entries })
}

/// Heal one pi entry into an Epsilon [`EntryKind`], resolving v2's compaction target.
///
/// The conversion itself is [`heal_legacy_kind`] — the *same* function the v1 migration calls.
/// That is the point of this whole slice: the last time this repository wrote a second converter
/// for pi's messages it read an array of parts with `as_str()` and turned a 668-character prompt
/// into an empty string while reporting success (G19,
/// `audit/evidence/pi-import-empties-messages-2026-09-22.md`). One healer cannot disagree with
/// itself.
///
/// The one thing v2 adds is `firstKeptEntryId`: pi's compaction names its retained window by
/// **id**, which is a real reference inside this file, so the target is resolved here and rides
/// in the healed compaction's `details` exactly as the v1 path's positional target does. An id
/// that names no entry is treated as absent rather than invented.
fn heal_pi_entry_kind(
    raw: &serde_json::Value,
    timestamp_ms: u64,
    by_id: &HashMap<&str, &SourceEntry>,
) -> EntryKind {
    let target = raw
        .as_object()
        .and_then(|m| m.get("firstKeptEntryId"))
        .and_then(|t| t.as_str())
        .filter(|id| by_id.contains_key(id))
        .map(str::to_string);
    heal_legacy_kind(raw, timestamp_ms, target)
}

/// The header this build writes for an imported session.
///
/// A new session: fresh id (an import is not the pi session, and two files claiming one id is
/// how a resume or a fork ends up in the wrong transcript), the source's `cwd` so a resumed
/// session still knows where it ran, and the source's creation instant where it wrote one —
/// because the alternative, `now`, re-dates a session from last year to the moment of the
/// import, which is the defect M114 fixed for entries arriving one layer up. Unknown header
/// fields ride along in `preserved` (M115), so a pi header keeps what this build does not
/// model.
///
/// A headerless source (pi v1) gets `now`, because a file that never recorded its creation
/// instant has none to carry.
fn imported_header(archive: &PiArchive) -> SessionHeader {
    let source = archive.header.as_ref();
    SessionHeader {
        kind: SessionHeaderKind::Session,
        version: Some(CURRENT_SESSION_VERSION),
        id: epsilon_contracts::new_id("ses"),
        timestamp: source.map_or_else(
            || iso_from_ms(crate::store::now_ms()),
            // pi writes an ISO string here, and it is kept **verbatim**: the source's own
            // creation instant, spelled the way the source spelled it. A header that wrote
            // something else (this build's own older files wrote bare epoch milliseconds) has
            // no spelling worth keeping, so it is converted — the instant survives, and the
            // output header is one format either way.
            |h| match parse_iso8601_ms(&h.timestamp) {
                Some(_) => h.timestamp.clone(),
                None => iso_from_ms(legacy_timestamp_ms(&serde_json::Value::String(
                    h.timestamp.clone(),
                ))),
            },
        ),
        cwd: source.map(|h| h.cwd.clone()).unwrap_or_default(),
        // A pi file's `parentSession` names a *pi* path or id. The copy descends from the pi
        // session, not from an Epsilon session that may not exist, so it claims no parent.
        parent_session: None,
        preserved: source.map(|h| h.preserved.clone()).unwrap_or_default(),
    }
}

/// The active branch of a pi archive: the leaf walked back to its root, oldest first.
///
/// pi's leaf is runtime state, not a field: `_buildIndex` takes the **last entry in file order**
/// as the leaf (`session-manager.ts:972-991`), and every append chains onto it. This function
/// applies that rule in the order a reader can defend:
///
/// 1. an explicit leaf marker on the header (`leafId` / `leaf_id` / `leaf`) when it names an
///    entry — pi's coding agent writes none, so a real pi file never takes this path, but a file
///    that does state where it ended beats a guess made from order;
/// 2. otherwise the entries no other entry names as its parent — the tips of the tree;
/// 3. among several tips, the **last in file order**, which is both what pi itself would have
///    chosen and what "the session went this way" means in an append-only log. Choosing the
///    *first* would silently import an abandoned branch and present it as the session, which is
///    the failure this rule exists to prevent rather than a detail of it.
///
/// A chain that re-enters an entry (a cycle in a hand-edited file) stops instead of looping —
/// the same tolerant contract [`crate::SessionStore::branch_entries`] keeps.
fn active_chain(archive: &PiArchive) -> Vec<&SourceEntry> {
    let Some(leaf) = explicit_leaf(archive).or_else(|| unnamed_leaf(archive)) else {
        return Vec::new();
    };
    let by_id: HashMap<&str, &SourceEntry> = archive
        .entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();
    let mut chain: Vec<&SourceEntry> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut cursor = Some(leaf);
    while let Some(id) = cursor {
        let Some(entry) = by_id.get(id).copied() else {
            break;
        };
        if !seen.insert(entry.id.as_str()) {
            break;
        }
        cursor = entry.parent.as_deref();
        chain.push(entry);
    }
    chain.reverse();
    chain
}

/// The header's explicit leaf marker, when it wrote one that names an entry.
///
/// Three spellings, because the marker is not part of pi's documented header at all
/// (`SessionHeader` is `session-manager.ts:32-39`; `leafId` is runtime state there): a file that
/// carries one is a different tool sharing the envelope or a future pi, and either way the
/// file's own statement about where the session ended beats an inference from order.
fn explicit_leaf(archive: &PiArchive) -> Option<&str> {
    let source = archive.header.as_ref()?;
    ["leafId", "leaf_id", "leaf"]
        .iter()
        .find_map(|key| source.preserved.get(*key).and_then(|v| v.as_str()))
        .filter(|id| archive.entries.iter().any(|entry| entry.id == *id))
}

/// The entry no other entry names as its parent; the last such entry in file order.
///
/// "Last" rather than "first" is a decision, not a detail. Several unnamed entries means either
/// that the session was navigated — pi moves the leaf back to an earlier entry and appends a new
/// child there, leaving the old child a tip — or that the file holds two roots. File order is the
/// only record of which came later, and the later one is what the session was doing when it
/// stopped.
///
/// A file where *every* entry is named as some entry's parent has a cycle among its tips; the
/// last entry in file order is then the least-wrong answer, and the walk will not loop on it.
fn unnamed_leaf(archive: &PiArchive) -> Option<&str> {
    if archive.entries.is_empty() {
        return None;
    }
    let named: BTreeSet<&str> = archive
        .entries
        .iter()
        .filter_map(|entry| entry.parent.as_deref())
        .collect();
    archive
        .entries
        .iter()
        .rfind(|entry| !named.contains(entry.id.as_str()))
        .or_else(|| archive.entries.last())
        .map(|entry| entry.id.as_str())
}

/// Record what one written entry preserved rather than typed.
///
/// Two different facts, both invisible from the transcript: an entry kind that healed to
/// `Custom` (a kind this build has no variant for, or one whose required fields did not line
/// up), and a content part that could not be typed and was kept inside its message.
fn record_preservation(kind: &EntryKind, kinds: &mut Vec<String>, unmapped: &mut usize) {
    if let EntryKind::Custom { data, .. } = kind {
        let tag = data
            .as_ref()
            .and_then(|d| d.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("unknown");
        if !kinds.iter().any(|seen| seen == tag) {
            kinds.push(tag.to_string());
        }
    }
    *unmapped += unmapped_parts_in(kind);
}

/// How many preserved content parts one written entry carries.
///
/// Counted from what the healer *wrote*, not from the source: the question a reader of the
/// report has is what the imported file holds.
fn unmapped_parts_in(kind: &EntryKind) -> usize {
    let EntryKind::Message { message, .. } = kind else {
        return 0;
    };
    match message {
        // A part this build cannot type rides in the tool result's `details` slot.
        Message::ToolResult(result) => result
            .details
            .get(LEGACY_EXTRAS_KEY)
            .and_then(|p| p.as_array())
            .map_or(0, Vec::len),
        // An unmappable *user* part (or an unknown role) heals to a `Custom` message holding
        // the whole object — one preserved thing, whatever it held.
        Message::Custom(custom) => usize::from(custom.custom_type == "legacy"),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The suffix rule is stated once, so this is what pins it: the source's own name stays
    /// intact in front of it, extension and all.
    #[test]
    fn the_output_name_keeps_the_source_name_whole() {
        assert_eq!(
            import_output_path(Path::new("sessions/pi-v2.jsonl")),
            PathBuf::from("sessions/pi-v2.jsonl.epsilon.jsonl")
        );
        assert_eq!(
            import_output_path(Path::new("no-extension")),
            PathBuf::from("no-extension.epsilon.jsonl")
        );
    }

    /// The v2 predicate accepts pi's tree entries and nothing else — in particular not a
    /// header, which also carries `type` and `id`.
    #[test]
    fn the_tree_predicate_accepts_pi_entries_only() {
        assert!(is_pi_v2_entry_shape(&serde_json::json!({
            "type": "message",
            "id": "a1b2c3d4",
            "parentId": null,
            "timestamp": "2025-12-08T22:41:05.306Z",
            "message": {"role": "user", "content": []}
        })));
        assert!(is_pi_v2_entry_shape(&serde_json::json!({
            "type": "model_change",
            "id": "e2",
            "parentId": "a1b2c3d4",
            "provider": "anthropic",
            "modelId": "claude-opus-4-5"
        })));
        assert!(
            !is_pi_v2_entry_shape(&serde_json::json!({
                "type": "session",
                "id": "ses_x",
                "timestamp": "2025-12-08T22:41:05.306Z",
                "cwd": "/tmp"
            })),
            "a header carries `type` and `id` too, and must not be read as an entry"
        );
        assert!(
            !is_pi_v2_entry_shape(&serde_json::json!({
                "type": "message",
                "seq": 1,
                "parent_id": null,
                "timestamp_ms": 7,
                "kind": {"type": "message"}
            })),
            "an Epsilon entry has no `id` at the top level and no `parentId`"
        );
        assert!(
            !is_pi_v2_entry_shape(&serde_json::json!({
                "type": "message",
                "timestamp": "2025-12-08T22:41:05.306Z",
                "message": {"role": "user"}
            })),
            "pi's own v1 line is healed in place, not imported"
        );
    }
}
