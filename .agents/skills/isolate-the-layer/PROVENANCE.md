# Provenance — `isolate-the-layer`

## What this is

An **adaptation** of a skill that ships with the Hermes Agent reference harness. It is also
**renamed**: upstream calls itself `rest-graphql-debug`, which describes the peer's protocol
rather than the procedure. The procedure — walk the chain until one step fails and the step
below passes — is what is worth keeping.

- **Upstream:** `references/hermes-agent-2026.9.24/optional-skills/software-development/rest-graphql-debug/SKILL.md`
- **Author / organisation:** `eren-karakus0`, packaged for Hermes Agent
- **Licence:** **MIT**. Hermes' `LICENSE` reads "Copyright (c) 2025 Nous Research"; the
  upstream frontmatter names the same licence. The same licence is recorded for Hermes in
  this repository's `CREDITS.md`.

`references/` is read-only input. The upstream file is untouched.

## What was changed

**Kept.** The seven-step ladder in order, and the gotcha that a GraphQL response is `200 OK`
with an `errors` array — restated as a general rule (*a successful exchange can carry a
failure in its payload*) and then tied to the two places this repository has the same shape:
the provider stream seam's never-fails contract, where an error arrives as a `StopReason`
rather than a thrown failure, and `retry::classify`, where an unrecognised error string is
deliberately `Other` and therefore never retried.

**Deleted in full.**

- Every `curl` invocation and every Python block — the whole 5-minute quickstart, the
  per-step probes, the `requests` examples, the backoff helper, the contract validator, the
  redaction helper and the vendor bug-report template. A repository skill must be prose an
  agent applies; a block an agent pastes is a shell script wearing a document.
- The pytest regression-test template.
- The Hermes **Tool Patterns** section entirely (`terminal` for curl and openssl,
  `execute_code`, `web_extract`, `delegate_task`).
- The HTTP status playbook, pagination and idempotency, correlation IDs, the leak
  checklist, and the output format — all of it keyed to a REST client's surface, none of it
  a step of the ladder.

**Retargeted.** Each step now names what it looks like here rather than what it looks like
in someone else's client: the `epsilon-llm` provider adapters and their SSE decoding, auth
resolution and its never-retried class, fallback routing; and `epsilon-server`'s body
validation and trust-boundary guards, where a refused body means the response you received is
not the response your code produced. Added the offline-first step — reproduce against the
faux provider before blaming a network — which follows `AGENTS.md` §7.

**Added.** The **Test Matrix** (symptom, layer, what distinguishes it), an **Avoid /
Prefer** block, and a **When NOT to use it** section.

## Why it earns its place

`AGENTS.md` §5 records that this repository has been burned by *something reporting success
while the thing wanted did not happen*. A clean stream termination carrying a failure is that
exact shape, and it is invisible to anyone reading only the status or the stop reason. The
ladder is the procedure that keeps a diagnosis at the layer where it belongs.

## When an agent should reach for it

When a call across a boundary fails or returns something unusable and the layer is not yet
known: a provider call, a stream, a route on `epsilon-server`, an MCP or RPC payload. Also
the moment a caller reports success but cannot use the result — that is the `200 OK` with an
`errors` array, and the ladder's first question is not the right one for it.

Not for failures with no boundary in them, and not once the layer is isolated — that is a
normal fix, and it needs a test that fails for the reason it names.