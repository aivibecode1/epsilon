# Independent review — M86/M87/M88 change set (deleg_32066967)

Read-only review, 2026-09-18, over the frozen candidate
`C:/Users/Hp/AppData/Local/Temp/epsilon-m86-green` (HEAD `d0ac16b` + the ten changed
files) and the working tree. No builds, no edits by the reviewer. Reference citations
were checked against `references/pi-0.84.2`.

**Verdict: no blockers.** 4 minor findings + 3 notes. Claim verdicts: all seven change
claims verified (claims 3 and 7 with the caveats below).

## Findings and dispositions

| # | Sev | Where | Issue | Disposition |
|---|---|---|---|---|
| 1 | minor | `epsilon-core/src/lib.rs` compact_now doc | Doc said "replace runtime history → persist entry"; the code (correctly) appends the durable entry first | **Fixed** — doc reworded to the implemented order |
| 2 | minor | `epsilon-cli/src/rpc_mode.rs` `admit_prompt` | Unbiased `select!` could let the handle arm win a tie against a queued `AgentStart`, reporting an admitted-then-failed run as *Rejected* ("no work started") and inviting a double prompt | **Fixed** — `biased;` with the event arm first; comment states why. No dedicated regression: constructing a post-admission run *failure* needs a provider whose stream ends without any frame; the faux provider cannot express it and `epsilon-cli` has no `async-trait` dev-dependency to hand-write one. The existing admission test still pins the reject path |
| 3 | minor | `epsilon-server/src/lib.rs` `prompt` | `RunSlot` was constructed after `user_message`, leaving a two-statement panic window that would leak the reservation (permanent 409) | **Fixed** — the guard is built immediately after the successful `fetch_add` |
| 4 | minor | `epsilon-core/src/lib.rs` `compact_now` | If the store append succeeded but the history mutex was poisoned, the pass returned an error with the durable entry appended and runtime history untouched — the one remaining split-view path | **Fixed** — on a poisoned history the appended entry is popped again before returning (`st.pop_last()`), mirroring `pop_last_message` |
| 5 | note | `lib.rs` event shape vs TUI | `CompactionStarted` is emitted for every attempt, `CompactionFinished` only on commit; the `Cancelled` path emits no `Notice`, so a frontend's "compacting…" status could persist until run end | **Documented** — the asymmetry is now stated on `compact_now`; `Failed` already overwrites the status through its `Notice`, and `Cancelled` ends the run immediately |
| 6 | note | coverage | No test drives the append-failure branch (the case the append-before-replace ordering exists for), nor the poisoned-history rollback | **Documented gap** — needs a store-level fault-injection seam (F07 work); named in the M86 limits and the ledger's F06/F07 notes |
| 7 | note | process | The working tree moved (fmt + review fixes) while the reviewer compared it to the frozen candidate; "byte-identical" was stale mid-review | **Process fix** — the candidate is the frozen artifact; equality of the committed files with the tested candidate is re-verified by hash at commit time (recorded in the verification log) |

## Reviewer answers worth keeping

- (a) No path commits without all three checks; there is no await between the final
  cancellation check and the runtime-history swap, so a late cancel cannot half-commit.
- (c) No legitimate refusal from the new mid-batch checkpoint: `run()` installs a fresh
  token per run and clears it before the running flag drops; children own their tokens.
- (e) The server reservation is released exactly once, by the request that took it; the
  409 path's `fetch_add`/`fetch_sub` pair is balanced. The only leak was finding 3.
- (f) The three behavioural tests cannot pass for the wrong reason; the residual risk is
  timing sensitivity (they can fail, never falsely pass).

Full reviewer report: the delegation transcript (`deleg_32066967`) and the summary in
this session; this file is the durable disposition record.
