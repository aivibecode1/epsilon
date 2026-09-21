# Session-format gap: pi's `label` and `session_info` entries, and what it takes to read them

Read-only investigation of HEAD `d8c3e51` (M112, clean tree); nothing was modified and no cargo command
was run. "I did not verify" marks what reading cannot settle (§5).

## 1. What pi's format has that ours does not

pi ships **two** session layers that disagree about unknown entries. The coding-agent vocabulary: nine entry
kinds (`session-manager.ts:144-153`, definitions `:53-141`) — what is on a user's disk under
`~/.pi/agent/sessions/`:

| pi kind | fields beyond `id`/`parentId`/`timestamp` | Epsilon |
|---|---|---|
| `message` | `message: AgentMessage` | `EntryKind::Message` (`session_entry.rs:95-102`) |
| `model_change` / `thinking_level_change` | `provider`+`modelId` / `thinkingLevel` | `ModelChange` (`:104-109`), `ThinkingLevelChange` (`:110-114`) |
| `compaction` | `summary`, `firstKeptEntryId`, `tokensBefore`, `details?`, `usage?` | `Compaction` (`:122-133`) |
| `branch_summary` | `fromId`, `summary`, `details?`, `usage?` | `BranchSummary` (`:135-142`) |
| `custom` | `customType`, `data?` | `Custom` (`:144-149`) |
| `custom_message` | `customType`, `content`, `display`, `details?` | **absent** (`Message::Custom` is closest, `message.rs:235-259`) |
| `label` | `targetId`, `label?` | **absent** |
| `session_info` | `name?` | **absent** |

**The two missing kinds a user can see.** `label` is a bookmark on an entry (`session-manager.ts:110-115`,
cleared by an absent or empty label `:982-988`): pi's `/tree` filters to `labeled-only` and prints
`[label: …]` (`tree-selector.ts:374-377`, `:839-841`), and a label is itself a tree entry that later
entries parent onto. `session_info` carries the session's display name (`session-manager.ts:117-121`:
latest wins, empty clears `:1163-1175`), shown in `/resume` instead of the first message
(`session-format.md:304`; read at `:713-716`). The model/thinking/tools trio is bookkeeping — pi's own
harness importer drops it from the tree into lane configuration (`legacy-v3.ts:100-112`, `:204-212`,
`:381-423`; `harness.md:1455`).

| layer | what pi does with an entry kind it does not recognize |
|---|---|
| coding-agent `SessionManager` (the files users have) | **keeps it.** `parseSessionEntryLine` is `JSON.parse` plus a cast, with no type validation (`session-manager.ts:503-511`); the object stays in `fileEntries`, is indexed (`:972-991`), returned by `getEntries`/`getTree` (`:1315-1317`, `:1324-1334`) and rewritten verbatim (`:993-1003`, `:1043-1055`) |
| harness JSONL v4 storage (pi's newer format) | **refuses:** `Invalid JSONL write kind: …` (`storage.ts:47-66`), wrapped as `Invalid JSONL storage <path>: line N` (`:201-208`) |
| harness legacy-v3 importer | **refuses:** `Unsupported legacy v3 record type at line N` (`legacy-v3.ts:173-187`) |
| harness, kinds it knows but does not keep | **hoisted out of the tree**: `label`/`session_info` become a session-name value and per-entry label values (`legacy-v3.ts:204-212`, `:432-467`; `harness.md:1454`) |

## 2. What happens today, step by step

Trace of a pi file with a `label` entry, opened by `--resume` (`main.rs:1683-1684`):

1. `SessionStore::open` → `open_with_parent` → `reindex` (`store.rs:287-294`, `:354`, `:494`), which
   collects non-blank lines with offsets (`:510-520`) and consumes the first as a header (`:526-532`).
   pi's header parses — Epsilon's `SessionHeader` is field-compatible (`session_entry.rs:41-62`) — so it
   is **not** indexed.
2. The first *entry* line — pi's first `message`, e.g. `{"type":"message","id":"a1b2c3d4","parentId":"…",
   "timestamp":"2024-…","message":{…}}` (`session-format.md:208`) — goes to `SessionLine::parse`
   (`store.rs:549`): the enum tries `SessionHeader` (`type` must be `"session"`), then `SessionEntry`,
   which *requires* `seq`, `timestamp_ms` and `kind` (`session_entry.rs:76-88`) — none present →
   `Some(Err(…))`.
3. `parse_entry_lenient` (`:573`, `:1322-1336`) retries the typed parse; `is_legacy_entry_shape` wants
   `type`+`timestamp` and the **absence** of `id`/`seq` (`:1340-1350`), and pi's line carries `id` → `None`.
4. The line is whole JSON, so not a torn tail (`:586-595`); `diagnose_line` (`:594` → `:1280-1304`) finds
   no `kind.type`, no `version`/`session_id`, no legacy shape → `UnknownLineShape` (`:1303`), and `reindex`
   returns `Err(StoreError::corrupt(2, …))` before indexing anything (`:610-612`), writing nothing: `open`
   writes only for a brand-new file (`:357-362`) or a torn tail (`:354-356`).
5. `--resume`'s `?` (`main.rs:1683-1684`) surfaces `open <path>: session line #2: valid JSON that is not an
   entry, a header or a legacy entry — … (file left intact)` (`store.rs:128-138`, `:85-90`) and exits.
6. **The `label` line is never reached.** The same kinds *inside our envelope*
   (`{"id":"ent_…","seq":N,"parent_id":…,"timestamp_ms":…,"kind":{"type":"label",…}}`) fail one step
   earlier in diagnosis and in the same place: the unknown tag fails `SessionLine::parse`, `diagnose_line`
   sees `kind.type` and returns `UnknownEntryType { entry_type: "label" }` (`:1285-1293`), and `reindex`
   refuses (`:610-612`; see `:477-483`, `:121-126`).

Pinned today by `session_store.rs:663-691` (`UnknownEntryType`, file byte-identical) and `:770-815`
(`UnknownLineShape`/`SecondHeader` for whole-JSON final lines, both newline spellings); the browser still
*lists* the file (pi's header parses, `discover.rs:81-102`), and it fails only when opened.

**User-visible outcome, one sentence:** a pi session file is listed by the browser and then refused at its
first entry line, so the whole conversation is unavailable and the message blames a shape this build does
not know — while the file itself is left byte-identical.

## 3. The design decision

**(a) Add the missing variants to `EntryKind`.** Two kinds get names, so a title and a bookmark can be
*applied* rather than merely retained, and it is the landing place for any importer. But the next new kind
is still refused, and per §2 no real pi file becomes readable — the envelope fails before the kind does.
It also obliges fork to re-chain copied paths (`session-manager.ts:1434-1460`), which this tree recorded
in advance (`M90-session-tree.md:115-118`).

**(b) Keep unknown entries as opaque, preserved lines.** Raw-line preservation — bytes kept beside the
index, never parsed — is not viable here, because the store's invariants are positional and typed: `seq`
*is* the file index (`store.rs:691-710`), `cached_entries[i]` must be the entry at `index[i]`
(`:245-261`, `:650`), `read_all` asserts the two agree (`:862-864`), every rewrite re-serializes *typed*
values (`persist_migration` `:387-419`, `fork_branch_at` `:1095-1099`), and repair cuts the file at the
last **indexed** line (`keep_end` `:665-671`, `truncate_to_index` `:677-687`, `pop_last` `:821-858`). An
unindexed foreign line would be truncated by the next repair or dropped by the next rewrite — a worse
version of the failure M108 fixed. Payload preservation inside a typed entry *is* viable, and this tree
already does it: `heal_legacy_line`'s fallback keeps a v1 line it cannot interpret as a `Custom` entry
holding the raw JSON — "nothing silently dropped … the model view skips `Custom` by construction"
(`store.rs:1405-1412`, `:1461-1464`). The cheap shortcut is a trap: `#[serde(other)]` must be on a *unit*
variant (`serde_derive-1.0.228/src/internals/check.rs:187`), so tag and body are discarded at parse time.

**(c) Both — my choice**, in the payload-preserving form: typed variants for kinds whose meaning we can
apply, one opaque variant for kinds we cannot, carrying the original `kind` object verbatim so a rewrite
reproduces it. The rule that keeps M108's promise: **an unknown tag is preserved, a known tag with an
unreadable body is still refused.** The first is "a newer build wrote something I do not know", which
preservation answers exactly. The second is "an entry exists that this build cannot show the model":
preserving *that* leaves a line contributing nothing to the model view — the silent transcript change
M108 was written to prevent (`store.rs:54-60`, `:477-483`).

**Against M108, preservation answers the goal M108 states better** — a complete line written on purpose
must not be deleted (`M108-unknown-final-line.md:15-19`). Refusal honours that by denying the whole session;
preservation honours it *and* leaves the session usable, without writing a byte at open (`reindex` only
reads, `store.rs:494-652`). The cost is the other half of that principle — "skipping it would change the
transcript silently" (`store.rs:80-83`): a preserved entry is not *applied*, so a meaning-bearing kind
leaves the model view older than the file. It is paid down by naming the kinds we can apply and by making
preservation **loud** (the store reports what it kept, the CLI prints it), and M108's `Limits` already flags
the rest: a known `kind.type` with a broken body is reported as a newer build's entry, and the known-name
list "needs to live somewhere the diagnoser can reach" (`M108:117-127`) — the list this rule needs anyway.

**The case in the gap report is not closed by (a), (b) or (c):** a genuine pi file still refuses at line 2
(§2). Kinds are one gap; the *envelope* is a second, larger one (no `seq`, ISO `timestamp`, `parentId`, kind
at the top level rather than under `kind`), so plan it as an importer, not enum variants.

## 4. The smallest faithful slice

**4A — entry kinds (small, independently landable, and the landing place for 4B).**

1. `crates/epsilon-contracts/src/session_entry.rs`: add `Label { target_id, label: Option<String> }` and
   `SessionInfo { name: Option<String> }` (pi's fields and optionality, `session-manager.ts:110-121`), plus
   `Unknown { type_name: String, body: serde_json::Value }` documented as *replay this object unchanged*;
   add `pub const KNOWN_ENTRY_TYPES: &[&str]` beside the enum as the single source of truth (the diagnoser
   needs it, `M108:117-127`). Put `deserialize_with`/`serialize_with` on `SessionEntry.kind`: the
   deserializer tries `EntryKind` on the parsed value and falls back to `Unknown` **only when the tag is a
   string not in `KNOWN_ENTRY_TYPES`**; the serializer writes `body` verbatim for `Unknown` and otherwise
   defers to the derived impl (the attribute is on the field, so that call is not recursive). Tests:
   `an_unknown_kind_round_trips_verbatim` (parse → serialize → the `kind` object is byte-identical);
   `a_known_kind_with_a_broken_body_is_not_swallowed` (envelope + `kind.type` `message`, bad body → `Err`);
   `label_and_session_info_keep_their_optional_fields`.
2. `crates/epsilon-session/src/store.rs`: add `LineDiagnosis::MalformedEntry { entry_type }`, whose
   `explain()` says the *type is known* and the *body is not*, and have `diagnose_line` (`:1280-1304`)
   consult `KNOWN_ENTRY_TYPES` — `UnknownEntryType` only for a genuinely unknown tag. Record what `reindex`
   preserved (`preserved: Vec<(u64, String)>`, refilled beside `cached_entries`, `:650`) and expose
   `pub fn preserved_unknown(&self)`; `model_view` needs no change (its `_ => {}` arm, `:1208`, skips
   non-message kinds, and `is_llm_visible` stays false, `session_entry.rs:177-179`).
   Tests in `crates/epsilon-session/tests/session_store.rs`:
   - `an_unknown_entry_kind_is_kept_and_the_session_opens` — header + two known entries +
     `{"id":"ent_x","seq":2,"parent_id":null,"timestamp_ms":0,"kind":{"type":"future_thing","payload":1}}`;
     asserts `open` is `Ok`, `len()==3`, `read_entry(2)` is `Unknown{type_name:"future_thing"}`, `model_view()`
     holds only the two known messages, `preserved_unknown()==[(2,"future_thing")]`, and the file is
     byte-identical. It rewrites (not deletes) the first case of `a_complete_but_unknown_final_line_is_refused_not_truncated`
     (`:770-815`), the way M108 rewrote its own predecessor.
   - `a_preserved_entry_survives_the_migration_rewrite` (v1 file: open → migrate → `persist_migration` →
     reopen → kind and payload intact) and `a_preserved_entry_survives_a_fork` (the fork holds the entry and
     payload; a `Label` on the path keeps its `target_id`, since fork copies the path verbatim
     `store.rs:1095-1099` — M90's re-chaining obligation `M90:115-118` bites only if labels are later stripped).
   - Negative cases, kept: `SecondHeader` and `UnknownLineShape` for every whole-JSON final line
     (`:770-815`); `NotJson` (`:710`); `a_torn_tail_is_still_tolerated` (`:739-760`). A line with no entry
     envelope is still refused: the store cannot index it without inventing an id and a seq (`:245-261`).
     And `a_known_kind_with_a_broken_body_is_refused_by_its_real_cause`:
     `{"id":"ent_x","seq":1,"parent_id":null,"timestamp_ms":0,"kind":{"type":"message"}}` →
     `Corrupt { line: 2, diagnosis: MalformedEntry { entry_type: "message" } }`, file byte-identical.
3. `crates/epsilon-cli/src/main.rs:1683-1712` (resume path): print how many entries were kept and of which
   kinds after a successful open — a store report nobody prints is the silent skip again. Test: a
   resume-shaped fixture in `crates/epsilon-cli/tests/doctor.rs` asserting the line on stderr.
4. **Not** in 4A: the label resolver and `/tree` labeled-only filter (pi's rule: walk in file order, set on
   a non-empty label, clear otherwise — `session-manager.ts:972-991`), and `/name` writing `SessionInfo`
   rather than `Custom{"session_name"}` (`app.rs:2096-2108`).

**4B — the pi-file importer (its own milestone; this is what §2 needs), in order:** recognize pi's v3 line
shape (`type`+`id`+`parentId`+ISO `timestamp`, no `seq`/`kind`) as a second legacy predicate beside
`is_legacy_entry_shape` (`store.rs:1340-1350`); parse ISO timestamps by hand, since this tree has no date
crate and `iso_now` only emits (`:1250-1260`); convert payloads (pi's content is a bare string or a block
array and its messages carry no `id`, `session-format.md:75-102`); map kinds onto 4A's variants,
materializing a compaction's `retainedTail` from `firstKeptEntryId` on its branch as pi's importer does
(`legacy-v3.ts:301-323`), and decide explicitly about `custom_message`, which pi sends to the model
(`session-format.md:275`, `:339`) and Epsilon's `Message::Custom` never does (`message.rs:246-247`); add
`#[serde(alias = "parentSession")]` to `SessionHeader.parent_session` (`session_entry.rs:60-61`). Land it
as **import-as-copy** (`/import <pi-file>` writes a new Epsilon session), never as an in-place migration
inside `open`: both formats call themselves version 3 (`session-manager.ts:30`, `session_entry.rs:34`), and
this store already refuses to heal a legacy shape found in a current-version file for exactly that reason
(`LegacyInCurrentFile`, `store.rs:625-632`, `:68-69`, `:91-95`). pi's harness agrees — a read-only open
leaves a v3 file unchanged and normalization persists on the first *write* (`harness.md:1463`) — and the
roadmap says so in advance (`audit/04-roadmap.md:107`).

## 5. What could go wrong

- **The version numbers collide** (`session-manager.ts:30`, `session_entry.rs:34`): pi's v3 and Epsilon's v3
  are different formats under one number, so 4B must trigger on the line shape, never the version field.
- **4A needs no migration of existing files.** This build's write path cannot produce `Unknown`, so the
  variant appears only in files from a newer build; files refused today become readable on disk,
  unmodified, and the refusal of envelope-less lines does not change (§4A.2).
- **The rewrite paths are where preservation can still be lost** (§3b): `persist_migration` (`:387-419`),
  `fork_branch_at` (`:1095-1099`) and `pop_last`/`truncate_to_index` (`:821-858`, `:677-687`) all
  re-serialize or cut; the 4A.2 tests pin the first two, and `keep_end` (`:665-671`) is safe only because
  a preserved entry *is* indexed.
- **Envelope fidelity is not total, and `Unknown` is a one-way door for the tag.** `SessionEntry` has no
  `deny_unknown_fields` (`session_entry.rs:76-88`), so an unknown *top-level* key is dropped on any rewrite
  today, for known kinds too; and a rewrite reproduces the original `kind` only while `body` is replayed
  verbatim, so the doc comment must forbid "tidying" it into structured fields.
- **Adjacent gap found, not fixed:** `discover::display_name` scans only the last 8 KiB (`discover.rs:26`,
  `:134-162`), so a name written early in a long session is invisible to the browser where pi streams the
  whole file (`session-manager.ts:703-737`); `search.rs` and the TUI parse files directly (`M108:114-116`)
  and were not checked, and a `SessionInfo` name inherits the same window.
- **I did not verify:** nothing was executed, so (i) that serde falls back exactly as 4A.1 assumes — the
  claim rests on the field list at `session_entry.rs:76-88` plus serde's documented behaviour, and that
  step's tests are the check; (ii) the exact string a user sees for a pi file (composed from `Display` at
  `store.rs:128-138`); (iii) whether pi still writes v3 by default (`session-format.md:19-27`).
