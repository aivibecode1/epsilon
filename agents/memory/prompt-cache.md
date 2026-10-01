# Working note — the prompt cache, and why the marker is where it is

**Status: landed at M239. The rule below is what a future edit can easily break.**

## The one sentence

**The tail `cache_control` goes on the LAST user message.** Not the first, not the end of
the transcript.

Anthropic caches a prefix and charges full price above the last marker. A turn is many API
calls — every tool result re-sends the whole history — and the user message at the head of
that turn does not move. So every intra-turn call reuses the prefix above it. A marker on the
*first* user message is valid JSON, passes every "is there a marker" check, and saves nothing
on the calls that matter.

`crates/epsilon-llm/tests/cache_breakpoints.rs` asserts this by building a real body through
`ProviderWire::request_body`. **That test is the guard.** If you touch the wire, run it.

## Three markers, and the fourth deliberately free

system block, last tool definition, last user message. Anthropic accepts four; the fourth is
left free because a marker on the very end caches a prefix the *next* request invalidates —
paid for this call, gone on the next.

## Where the decision lives

`crates/epsilon-llm/src/cache_policy.rs`, as a value — not a constant inside the wire. The
decision is about a conversation, not about JSON, and the same policy would apply to a body
assembled for another provider.

## Not yet true

- **Nothing here measures a dollar.** The saving is arithmetic on published prices; the tests
  prove markers are *placed*, not that a hit happened. A cache-hit readout from a real
  session is the missing evidence.
- **OpenAI and Gemini emit nothing**, because they cache implicitly. That is
  capability-gated *in intent* but there is no capability flag yet — a provider that starts
  honouring inline hints needs `CachePolicy` wired in.
- **A call made only of tool results** skips the tail marker entirely. Correct for cost,
  untested.

## Gotcha that cost a round

`clippy` rejects `assert!(SOME_CONST.field)` as **"this assertion has a constant value"** —
correctly. A test on a `const` boolean cannot fail, and it is worse than no test because it
looks like coverage. Anything about *where* a value lands has to be tested where it lands.