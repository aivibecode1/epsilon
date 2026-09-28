# M66 — a boundary the content cannot forge, and user words that survive

**Scope:** port-plan Tier 2 item 8 (untrusted-result wrapping) and the Hermes micro-compaction
invariant from item 15 — the two places where text is shaped before the model sees it.

Both are small, both are about *provenance*: one labels content that came from outside the
session, the other refuses to paraphrase the words of the person who owns it.

## What

### 1. External content arrives inside a boundary it cannot forge

A tool result is not trustworthy just because a tool produced it. `webfetch`, every MCP server and
every extension hand back text written by someone else, and it landed in the context at the same
level as the user's own instructions. The contract asked the model not to follow instructions in
content; `epsilon-core/src/untrusted.rs` now makes the boundary structural:

- Text from `webfetch`, `mcp_*` / `mcp__*` and `ext_*` / `ext__*` is enclosed in
  `<untrusted_tool_result tool="…">…</untrusted_tool_result>`.
- Every **case-variant** of the delimiter token inside the body is rewritten (`untrusted-tool-result`)
  before wrapping, so a page cannot close the boundary early, open a fake one, or slip past a
  case-sensitive check with `</Untrusted_Tool_Result>`.
- Results under 32 bytes are not wrapped: there is nothing to hide in `24°C, clear`, and wrapping
  every trivial answer costs prompt bytes to protect nothing.
- Local tools (`read`, `bash`, `grep`, …) are **not** wrapped, and their content is passed through
  byte-for-byte — including a forgery attempt. Workspace content is the user's own; burying it in
  markers would hide the context the model works in.

### 2. A truncated payload says so

Providers truncate silently and then add markers that read as ordinary prose — `"has_more": true`,
`… 13 more items`, `data_preview`, `saved to sandbox`. A truncated result looks structurally
complete, so a model summarizes the visible slice and reports it as the whole dataset. When a
marker appears in an untrusted result over 1 KB, one compact note follows the wrapper: *this data
may be INCOMPLETE — the source indicated more content than was returned*.

### 3. The user's own words are never paraphrased into a summary

`compaction` summarizes assistant work; it was also paraphrasing the user, and a paraphrased
instruction is one that gets followed approximately — "do NOT touch the parser" becomes "the
parser was discussed". `compact::user_instruction_block` now carries the user turns from the
summarized range into the new context **verbatim**, in a `<user_instructions>` block next to the
summary, and the summarization request is told to quote rather than restate.

Details that matter:

- Collected newest-first, so the 4 KB cap drops the *oldest* instructions rather than the most
  recent ones, and an omission note says how many went.
- A previous compaction's summary is not the user speaking — those messages are skipped.
- But the block a previous summary carried *is* recovered from it and merged, so instructions
  survive successive compactions instead of evaporating on the second one.
- Both blocks ride inside the same summary message as the file ledger from M64: the model's prose
  first, then the harness's facts.

## Key decisions

- **Provenance is a property of the source, not of the text.** The wrapper keys off the tool name
  (`webfetch`, `mcp_*`, `ext_*`), which the harness knows for certain, rather than guessing at
  content.
- **Neutralize rather than reject.** Forged delimiters are rewritten, not dropped: the model still
  sees what the page said (that is the evidence), it just cannot be confused about where the page
  ends.
- **Both directions of the boundary are tested.** One test proves an MCP result *is* wrapped and
  neutralized; another proves a local result is *not* — and that its byte-for-byte content,
  forgery attempt included, survives. A single test could not tell the difference between the two
  rules working and the wrapper being applied to everything.
- **Never summarize the user.** The summary is the model's own paraphrase of its own work; the
  instructions are the user's and belong in the context as typed. This is the one invariant where
  being lossy is not a quality trade-off but a correctness bug.
- **Shaping happens once, at message construction.** Nothing re-writes a result after the fact, so
  prompt caching sees stable bytes and the session file holds what the model actually read.

## Test-caught bugs

1. **Instructions evaporated on the second compaction.** The block was built from the summarized
   range, and after the first compaction that range contains only the *previous summary* — which is
   skipped as harness prose. So the user's words survived exactly one compression and were gone
   after two. Caught by the loop-level test (which compacts twice); fixed by recovering the block
   from the prior summary, deduping, and merging — the same pattern `FileLedger::from_summary`
   already used.
2. **A test that could not fail, and a test that lied.** The first local-tool test asserted the
   result contained no delimiter token — but the fixture *is* a forgery attempt, so it always
   contained one, and the assertion was wrong rather than failing. Rewritten to assert what
   actually matters: the result does not *start* with the wrapper and the payload arrives
   byte-for-byte.
3. **A compaction test that proved nothing three times over.** It first compacted an empty middle
   (threshold too low, so the block was empty), then never compacted at all (M64's usage anchor
   makes the faux provider's fixed `input: 100` the estimate, so a 4 KB result stayed under the
   threshold). Fixed by sizing the fixture past the anchor and pinning the threshold above the
   first turn — and the failure mode of each attempt was visible only because the assertion quoted
   the whole summary text.

## Verification

```
$ bash scripts/gate.sh m66
===== fmt --check =====      clean
===== clippy =====           0 warnings
===== test =====             574 tests, 0 failed, 1 ignored (the live smoke)
===== demo =====             completes
===== real-check =====       compiles
```

Fourteen new tests:

| Test | What it pins |
|---|---|
| `the_named_sources_are_untrusted_and_local_tools_are_not` | the classification itself, in both directions |
| `external_text_arrives_inside_a_boundary_that_names_its_source` | the wrapper, and that the content inside it is shown, not censored |
| `content_cannot_close_or_forge_the_boundary_in_any_case` | four forgery shapes, including mixed case: exactly one closing delimiter survives, and it is the harness's |
| `a_short_result_is_left_exactly_as_it_was` | the 32-byte floor |
| `a_long_result_that_reports_more_upstream_says_so` | the incompleteness notice, and no false alarm without a marker |
| `the_notice_needs_both_a_marker_and_length` | both conditions of the notice |
| `the_users_own_words_survive_compaction_verbatim` | exact words, in order |
| `a_previous_summary_is_not_mistaken_for_something_the_user_said` | harness prose is not attributed to the user |
| `the_cap_drops_the_oldest_turns_and_says_how_many` | the cap's direction, and that the block stays bounded |
| `nothing_to_keep_means_nothing_is_added` | no empty blocks |
| `an_mcp_result_reaches_the_model_inside_a_boundary_the_content_cannot_forge` (loop) | the wrapper through the real loop, with a hostile payload |
| `a_local_tool_result_is_handed_over_untouched` (loop) | the other direction: local content is not wrapped and not edited |
| `a_truncated_fetch_says_it_is_incomplete` (loop) | the notice through the real loop |
| `a_compaction_keeps_the_users_own_words` (loop) | two compactions, with the instruction still verbatim in the context |

Checked by mutation — the shapes disabled one at a time:

```
# is_untrusted_source() always false
$ cargo test -p epsilon-core
untrusted::tests::the_named_sources_are_untrusted_and_local_tools_are_not ... FAILED
$ cargo test -p epsilon-core --test context_shaping
an_mcp_result_reaches_the_model_inside_a_boundary_the_content_cannot_forge ... FAILED
a_truncated_fetch_says_it_is_incomplete ... FAILED
a_local_tool_result_is_handed_over_untouched ... ok        # correctly unaffected

# user_instruction_block() always empty
$ cargo test -p epsilon-core
compact::instruction_block_tests::the_users_own_words_survive_compaction_verbatim ... FAILED
compact::instruction_block_tests::a_previous_summary_is_not_mistaken_for_something_the_user_said ... FAILED
compact::instruction_block_tests::the_cap_drops_the_oldest_turns_and_says_how_many ... FAILED
compact::instruction_block_tests::nothing_to_keep_means_nothing_is_added ... ok   # correctly unaffected
$ cargo test -p epsilon-core --test context_shaping
a_compaction_keeps_the_users_own_words ... FAILED
```

Both mutations were reverted before the gate run quoted above.

## Follow-ups

Provider-side sanitization (Hermes' argument repair and interrupted-sequence closing, item 18),
the remaining Phase 3 guardrails (verification enforcement, no-progress and empty-response guards),
and the standing items: committing M33–M66 (D-12), live-provider evidence for every wire (D-13),
release artifacts (D-21). See [`audit/07-plan.md`](../../audit/07-plan.md).
