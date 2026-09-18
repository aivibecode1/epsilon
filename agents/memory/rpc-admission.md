# RPC prompt admission (RPC-01)

## Verified state
- Source: `audit/evidence/revalidation-core-review.md` [RPC-01]; plan row 4 half.
- HEAD d9c1810. `crates/epsilon-cli/src/rpc_mode.rs` was already dirty (inherited
  `@`-reference change: `user_message` delegates to
  `epsilon_core::references::user_message`, which main.rs at HEAD already requires).
  Review called that hunk benign/protocol-neutral; both hunks ship in the same commit.

## Change
- New `admit_prompt(agent, prompt) -> Admission{Started, Rejected(String)}`: spawns
  `Agent::run`, then races the JoinHandle against the agent's event stream. `Agent::run`
  settles admission before its first await (AlreadyRunning returns immediately;
  AgentStart is emitted right after), so the reply states what happened.
- `Op::Prompt` replies `started` only for `Admission::Started`; otherwise an error with
  the real reason. Post-admission failure is logged to stderr (was silently discarded).
- Deleted `agent_is_busy` (always-false advisory helper).
- Test: `a_second_prompt_is_rejected_while_a_run_is_admitted` (in-file unit test;
  FauxProvider behind SlowProvider; aborts after assertion; bounded re-admission wait).

## Status
- Uncommitted. Ships in the round-B candidate with the M86 compaction slice, so one
  battery covers both. Milestone doc: `docs/milestones/M87-rpc-admission.md` (PENDING
  evidence). Ledger note added to F13.
- Open after this: RPC-02 correlation IDs/abort/state ops; serve-mode admission; F18
  task-state reset on resume.
