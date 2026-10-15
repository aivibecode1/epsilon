# 18 — Reference-capability re-verification: changes, question tool, plan mode (2026-10-15)

**Provenance.** This is the third attempt at the criterion-3 capability audit. The first
dispatch died on an HTTP 504 after 96 minutes; the second, deliberately narrowed to three
candidates, died on an HTTP 429 during its first broad grep. Rather than spend a fourth
dispatch on a flaky provider, the integrator verified all three candidates directly against
the tree — every line cited below was read. That asymmetry is stated, per the convention of
this folder: the *reference* readings here are lighter than a dedicated study's.

---

## 1. `changes` — a session's file diffs — **implement now, ranked first**

**What exists, all verified:** the checkpoint store records a pre-image before every
mutation (`crates/epsilon-tools/src/files.rs:520` write, `:718`/`:793` edit →
`CheckpointStore::record`) and a post-image after (`amend_with_post_image`, same three
sites), content-deduplicated into blobs, with `Recorded::{Stored, Created, Skipped,
Amended}` naming every outcome (`crates/epsilon-tools/src/checkpoint.rs:114-131`) and
`latest_for`/`paths`/`restore`/`entries` as the access surface. `/restore <path>` is a real
command with a preview-then-confirm argument shape (`crates/epsilon-contracts/src/command.rs:184-189):
its own history (`:376-385`) records that the *write* half reached this command only after
an audit found the revert reachable from tests alone. The permission dialog now draws a real
diff on the ask path (`322f7b0`, the defaulted `Tool::preview` seam), and the diff renderer
it uses is the card's.

**What genuinely lacks:** any aggregation. There is no `/changes` command, no panel, and
nothing groups the store's entries by session — `Entry` carries `seq`, `at`, `path`, `tool`,
pre- and post-image hashes, but no session id (`checkpoint.rs:52-113`). So "what did this
run change, and what did each file look like before it" has no surface even though both
halves of the answer are on disk.

**Smallest correct shape.** A `/changes` command that lists this session's mutated paths,
then a panel that reuses: the diff renderer from the permission preview, the picker's
overlay pattern, and `/restore <path>` (already implemented) as the revert action.
Session scoping is the only new read: filter entries by the session's start time — the
store's `seq` is monotonic per workspace and `at` is unix seconds — with the filter pinned
by a test, not by a convention in a comment.

**Verdict: implement now, first of the three.** It is the only one whose value is daily
("what did the agent just do to my tree"), whose data already exists, and whose failure
modes are already understood (the pre-image hash guard at `:89-101` exists precisely because
restoring over a user's newer edit would destroy their work).

## 2. A question tool — **defer, with its trigger named**

pi ships a typed `tool-ask-user` (`packages/interaction/tool-ask-user/src/index.ts`: injects
`['tools', 'userQuestions']`, a schema with a `timeout` default 120, and a description that
scopes it to "a confirmation, a choice, or missing information before proceeding"). Epsilon
has no equivalent: the only structured interactive surface is the approval channel
(`crates/epsilon-safety/src/channel_ui.rs`), which fails closed and belongs to permission
decisions, not questions; the grep for `ask_user`/`user_questions` across `crates/` is empty.

The smallest correct shape is not small: a new `Tool` in the contract, a question channel
beside the approval one with the same fail-closed discipline, an overlay in every mode that
renders a blocking question (the terminal's overlay, HTTP/SSE's event stream, RPC's
request/response), and a timeout story that cannot lie about an answer nobody gave. That is
a protocol change across three frontends, and today the model can already ask in text and
the user can answer in text — the missing thing is structure, not capability.

**Verdict: defer.** Trigger: a real workflow where a free-text answer is materially worse
than a structured one (a multi-choice decision the model must not paraphrase). Recorded so
the next campaign does not re-derive the cost.

## 3. Plan mode — **not a capability gap; a discoverability gap**

**What exists:** `--plan` prepends `epsilon_safety::plan_ruleset()` to the layered engine so
mutating capabilities are denied outright and monotonic-deny keeps that boundary against
project rules and approvals (`crates/epsilon-runtime/src/lib.rs:4929-4943`, with the
ADR-006 amendment at M31/F10 naming the layering and the shell refusal). It is wired through
`RunParams`, the CLI (`crates/epsilon-cli/src/main.rs:482`), and the RPC/HTTP paths. It is
NOT a no-op — invariant 7 is satisfied — and it is not in `docs/user/configuration.md`.

**What genuinely lacks:** pi's *review* half. pi's `exit_plan_mode` presents a plan and asks
before edits are allowed; ours denies mutations outright, which is the same guarantee with a
different interaction: a user in plan mode who wants to proceed must restart without the
flag. A TUI toggle (a `/plan` arm flipping the same layer, ADR-worthy because it is a
runtime mode change) is the small correct upgrade; a pi-shaped plan-review flow is the
large one.

**Verdict: the discoverability half is a docs change (done in the same wave as `changes`);
the toggle half is deferred to after `changes`, on the same reuse — the gate-layer switch,
not a new mode system.**

---

## Re-ranking, by what a user notices missing first

1. **`changes` panel** — implement now. Data exists; the daily question; the reuse list is
   concrete.
2. **Plan mode discoverability** — a one-command doc fix plus, later, a toggle.
3. **Question tool** — defer with the trigger above.
