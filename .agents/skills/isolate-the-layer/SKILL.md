---
name: isolate-the-layer
description: Use when a call that crosses a process boundary fails or returns something unusable — a provider request, an SSE stream, an epsilon-server route, an MCP or RPC payload — and the failing layer has not been identified yet; also use when a call reports success but the caller cannot use the result.
---

# Isolate the Layer

Find the layer that is actually broken before changing anything, walking the ladder in order.
The layer above a failure is usually innocent; the one below is usually fine.

## The gotcha that costs the most time

**A successful exchange can carry a failure in its payload.** A GraphQL response is `200 OK`
with an `errors` array; the status says the transport worked, and nothing else does.

This repository has the same shape, twice over:

- The provider stream seam **never fails**. An error is not thrown — it is encoded in the
  message's `StopReason::Error`, and the stream terminates normally. `is_failure()` is what
  distinguishes the two terminal shapes. Code that checks only "did the stream end" reads a
  provider failure as a finished turn.
- Retry classification is done over **message text** (`retry::classify`), and anything
  unrecognised is deliberately `Other`, which is never retried. A new error string therefore
  degrades to fail-fast rather than to a fix.

So read the payload's own error channel, not the status code and not the termination.

## Before the ladder

Reproduce offline against the faux provider where you can: a scripted failure that never
touches a network separates *our code* from *the peer's behaviour*, at the cheapest price.

## The ladder

1. **Did the call start at all?** Before anything else, confirm the code path was entered
   and the request left the process. A failure that never reached the boundary is a bug
   above this ladder entirely.
2. **Stuck, or slow?** Separate *cannot reach* from *reaches but is slow*. Time to first
   byte high with connect time low is the peer's latency; connect time high is DNS, proxy,
   firewall or VPN — which is not a defect here, and should be escalated rather than fixed
   in code.
3. **Was the exchange intact?** Transport-level integrity: TLS and certificate trust for
   HTTP, and stream framing for SSE. A server-sent-event decoder joins continuation lines
   and tolerates CRLF; a frame it mis-splits looks like a peer that stopped talking mid-answer.
4. **Were the credentials accepted?** Credential resolution and precedence, environment
   against stored secret, right scheme for the provider, right environment. This class is
   never retried and never routed to a fallback, so a bad key looks like a hard stop
   everywhere downstream — which is correct, and is why it must be cleared here.
5. **Was the request shaped as the peer expects?** Method, headers, content type, body
   shape, required fields. On our own server the equivalent is body validation: a malformed
   or absent body is refused with its own status before the route runs, so the response you
   received may not be the response your code produced.
6. **Did our side accept what came back?** Parsing, decoding, schema drift, a body that is
   prose rather than the JSON the client expects. A wire name is verified by sending a real
   frame, never by comparing serde's output against the variant list.
7. **Does the payload mean what we assume?** Parsed cleanly and still wrong: identifiers
   that do not match the request, an empty list that means "page one", a field the peer
   documents differently from how the code reads it.

## Avoid / Prefer

| Avoid | Prefer |
|---|---|
| Changing code at the layer you are reading | Probing downward until one step fails and the one below passes |
| Reading only the status or the stop reason | Reading the payload's own error channel as well |
| Asserting a wire name against serde's variant list | Sending a real frame and reading what came back |
| Blaming the provider for our parsing | Probing step 6 before step 7 blames anyone |
| Judging the peer by your own naming | Comparing against the reference behaviour |

## Test Matrix

| Observation | Layer | What distinguishes it |
|---|---|---|
| Nothing was sent | 1 | No request in the transport at all |
| Connect slow, first byte fast | 2 | Boundary reached; local network |
| Connect fast, first byte late | 2 | Peer is slow; retry or accept |
| Stream ends mid-frame | 3 | Framing or decode, not the peer |
| Hard stop, no retry, no fallback | 4 | Credentials or a configuration typo |
| Request refused before the route runs | 5 | Body validation or a trust-boundary guard |
| Peer answered, our side choked | 6 | Parsing, decoding or schema drift |
| Everything parsed, value wrong | 7 | Assumption about meaning |
| Clean end that is actually a failure | The gotcha above | `StopReason::Error` with a normal termination |

## When NOT to use it

- Failures with no boundary in them: pure local logic, a layout bug, a wrong computation.
- UI rendering problems — those are read off the frame, not off a wire.
- Performance tuning and query planning, which are not correctness failures.
- Once the layer *is* isolated. Then it is a normal fix, with a test that fails for the reason
  it names.