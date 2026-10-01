# M239 — the prompt cache was marking the one place that never moves

The Anthropic wire carried exactly one `cache_control`, on the system prompt. Everything
after it — the entire transcript — was billed at full price on every call.

## Why that is the wrong single place

Anthropic caches a prefix and charges full price above the last marker. The system prompt is
the **most stable** part of a request, which is exactly why it is the **least useful** place
to mark for a coding agent: it barely changes within a session, and nothing that grows was
ever inside the cached region.

What grows is the transcript, and what repeats is the *turn*: one user request is many API
calls, because every tool result sends the whole history back. The last user message sits at
the head of that turn and does not move. Marking it means **every intra-turn call reuses the
prefix up to and including it.**

Three boundaries now, and each earns its place:

| Marker | Why |
|---|---|
| last **tool definition** | Tool schemas sit between the system prompt and the transcript; a stable marker there keeps the expensive part of the prefix reusable as the conversation extends. |
| last **system block** | The stable prefix. Kept from pi's behaviour. |
| last **user message** | **The one that pays.** Stable for the whole turn while everything after it grows. |

opencode reaches the same three (`packages/llm/src/cache-policy.ts:18-22`) for the same
reason, and says it better: *"the latest user message stays put while a single turn explodes
into many assistant/tool round-trips, so caching at that boundary lets every intra-turn API
call hit the prefix."*

## Two decisions inside the decision

**The fourth breakpoint is left free.** Anthropic accepts four. A marker on the very end of
the transcript would cache a prefix the *next* request invalidates — paid for on this call
and gone on the next. Spending three deliberately leaves room for a boundary nobody has
needed yet, rather than spending it on a marker that buys nothing.

**It is a policy, not a constant, and it lives outside the wire.** The decision is about a
conversation, not about JSON; the same policy would apply to a body assembled for a provider
with different cache semantics. Naming it separately keeps the wire file about shapes, and
makes the policy testable without building a request. OpenAI and Gemini cache implicitly and
ignore inline hints, so nothing is emitted there — **capability-gated, not provider-name-
gated**, so a provider that gains the feature gets it without a second edit.

## The test is a wire test, and it is the one that matters

`crates/epsilon-llm/tests/cache_breakpoints.rs` builds a request body through
`ProviderWire::request_body` — the call the transport makes — and reads it back. No network:
the wire is a pure function of its context.

The load-bearing assertion is **`the_tail_breakpoint_is_on_the_last_user_message_not_the_first`**.
A marker on the *first* user message is valid JSON, passes every "is there a marker" check,
and saves nothing on the calls that matter. Reading the body back is the only thing that
distinguishes the two, which is the same argument `AGENTS.md` makes about protocol tests and
the reason `cache_control` needs this level of care.

Also pinned: a turn with **no** user message assembles and gains no phantom marker; empty
system and tool sections are left out rather than gaining one.

And the unit tests on `CachePolicy` deliberately say what they are for — the arithmetic of
how many markers a policy spends. A first draft asserted `mark_last_user_message` on a
`const`, which **clippy rejected as a test with a constant value**. It was right: that
assertion could not fail, and it was standing in for a check that actually belongs at the
wire.

## Verification

**Battery `m239` — GATE PASS on attempt 1, 1,759 tests across 73 binaries, 0 failed**
(`audit/evidence/gate-m239.log`). Up from 1,752: five wire tests plus two arithmetic ones.
clippy clean across 14 crates.

## What nothing exercises

- **The saving.** Nothing here measures a dollar. The claim that this reduces cost is
  arithmetic on Anthropic's published prices, not a measurement on this machine, and the
  tests prove the markers are placed — not that a cache hit happened. A cache-hit-rate
  readout from a real session would be the evidence, and there is not one.
- **Other providers.** OpenAI and Gemini are untouched, deliberately. If a future provider
  honours inline hints, it needs `CachePolicy` wired in — the capability flag does not exist
  yet, only the reasoning for where it would go.
- **A turn whose last message is a tool result** rather than a user message. The tail marker
  is skipped entirely in that case, so a call made purely of tool results caches nothing new;
  that is correct for cost and is untested.
- **Cache TTLs.** `TextFormat` carries no TTL here; a five-minute window is the default and
  nothing asks for anything else.