# Writing an Epsilon extension

An extension is a **separate program**. Epsilon launches it, talks to it over its stdin and
stdout, and asks it for things. It cannot read your files, call your functions, or crash the
harness — the worst it can do is stop working, and the harness notices.

This page is the whole protocol. Everything here is implemented and tested; where something
is *not* implemented, it says so.

## The shape of it

```
Epsilon  ──spawn──▶  your program
   │                       │
   │  {"type":"interpose",   │   "here is a thing that happened,
   │   "id":"x_1",            │    what do you want to do about it?"
   │   "event":{...}}         │
   │  ◀────────────────────  │
   │  {"type":"interpose_result",  "id":"x_1","decision":{...}}
   │                       │
```

One line of JSON per message, in both directions, on stdin and stdout. That is the entire
transport. There is no socket, no port, no handshake you have to implement.

**Everything else your program writes to stdout is a bug.** A stray `println!` in Node, a
debug print in Python, a warning on stderr that gets redirected — any of those will be read
as a protocol frame and rejected. If you need to talk to a human, use **stderr**; it is
yours. The reserved frame for that is:

```json
{"type":"log","text":"anything you like"}
```

which the harness accepts and does not interpret.

## Getting started

A directory with an `extension.json` and an executable:

```json
{
  "name": "no-weekend-deploys",
  "version": "0.1.0",
  "bin": "main.js",
  "args": [],
  "events": ["before_tool_call"],
  "tools": []
}
```

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Identifies the extension in diagnostics. |
| `version` | yes | Informational. |
| `bin` | yes | Executable to run. A relative path resolves against the extension's own directory, so a self-contained folder works wherever it is installed. |
| `args` | no | Extra argv, e.g. `["run"]` for a script that needs an entry-point flag. |
| `events` | no | Which event types you want. `["*"]` means all of them. **`events` is a filter, not a subscription service** — see below. |
| `tools` | no | Tools you provide. An extension with no tools is event-only, which is the common case. |

## Blocking a tool call

Declare `"before_tool_call"` in `events`. Before every tool call, you get asked:

```json
{"type":"interpose","id":"x_1","event":{
  "type":"before_tool_call",
  "tool_call_id":"call_abc",
  "tool_name":"bash",
  "args":{"command":"git push --force"}
}}
```

To let it run, answer with an empty decision:

```json
{"type":"interpose_result","id":"x_1","decision":{}}
```

To stop it:

```json
{"type":"interpose_result","id":"x_1","decision":{
  "block": true,
  "reason": "force pushes are disabled on this repository"
}}
```

Your `reason` is shown to the user and lands in the transcript, so **write it for the person
reading it, not for your own logs.** If several extensions block the same call, every reason
is shown — so a message naming the rule and the repository beats one naming a line number.

Two fields, and only two:

| Field | Type | Effect |
|---|---|---|
| `block` | `true` | The call does not run. The model is told it was refused, with your reason. |
| `terminate` | `true` | Stop after the current batch. Honoured only when *every* blocked call asks for it, so one extension cannot halt a run alone. |

Anything else in your decision is ignored. In particular, **you cannot change the arguments**
— see [What is not supported](#what-is-not-supported).

## Rewriting a tool result

Declare `"tool_result"`. After a tool succeeds, you get asked what the model should see:

```json
{"type":"interpose","id":"x_2","event":{
  "type":"tool_result",
  "tool_call_id":"call_abc",
  "tool_name":"read",
  "input":{"path":".env"},
  "content":[{"type":"text","text":"API_KEY=hunter2"}],
  "is_error":false,
  "details":{"path":".env"},
  "usage":null
}}
```

You see the real result — including the `details` a renderer uses — because an extension that
wants to redact a secret has to be able to see it. Answer with any subset of:

```json
{"type":"interpose_result","id":"x_2","decision":{
  "content":[{"type":"text","text":"[redacted by no-secrets]"}],
  "reason": "redacted a credential"
}}
```

| Field | Type | Effect |
|---|---|---|
| `content` | array | Replaces what the model sees. `{"type":"text","text":"..."}` is the only block understood; others are dropped. |
| `details` | any | Replaces the details payload. |
| `is_error` | bool | Flips the error flag. `false` clears an error the tool reported. |
| `usage` | object | Replaces the usage that feeds the cost readout. |
| `terminate` | bool | Same hint as above. |

**Only the fields you include change.** Omit `details` and the original is kept. Answer with
an empty decision and nothing changes at all.

Give a `reason` if you rewrite. The harness records it in the result's details so a silent
change to what the model sees is traceable — and it will not overwrite a `details` you
supplied yourself, because a renderer is reading those.

## Ending a run

Declare `"turn_end"`. After each turn you are asked whether the run should continue:

```json
{"type":"interpose_result","id":"x_3","decision":{"stop":true}}
```

| Field | Type | Effect |
|---|---|---|
| `stop` | `true` | The run ends cleanly after this turn: no further model request, queued follow-ups are not drained. |
| `model` | string | Switch model for subsequent requests. |

This is the seam to use for stopping before the context fills up.

## Rules you can rely on

These are guarantees, each one tested against a real child process.

**Your `id` is how your answer is matched.** Echo it back exactly. An answer with an `id`
you were not given is discarded.

**Silence is never a veto and never consent.** If you do not answer, the tool call proceeds.
The harness waits about **one second** for a decision — a long pause would be paid on *every*
tool call in the run — and treats no answer as "no opinion".

**You cannot end a run by accident.** A `stop` is honoured only when the value is exactly
`true`. A string, a number, or a decision that is not an object at all is ignored, because a
broken extension that halts every session is worse than one that does nothing.

**You cannot corrupt the conversation.** The two hooks that shape what the model sees
(`transform_context`, `convert_to_llm`) are implemented as pass-through, so a child process
never rewrites the prompt. This is a deliberate divergence from pi — see
[What is not supported](#what-is-not-supported).

**Every extension is asked, and any one of them wins.** A block is a block. There is no
majority and no first-one-wins, so *adding* an extension can only add a restriction. That is
also why you will see the reason from every extension that objected rather than only the
first.

**You are only asked about events you declared.** The `events` list is a filter applied
before your process is involved at all, so declaring fewer events costs you nothing.

## Limits you can hit

These are hard caps, and hitting one ends the conversation rather than degrading it.

| Limit | Value | What happens |
|---|---|---|
| Time to answer a decision | **1 second** | Treated as no opinion. The call proceeds. |
| Time for an `invoke` | **10 seconds** | The call fails. |
| Bytes in one frame | **2 MiB** | The stream is stopped — see below. |
| Bytes in one `invoke` | **16 MiB** | Same. |
| Crashes tolerated per run | **2** | The third disables the extension for the rest of the run. |

The two byte caps **stop the stream rather than skipping a frame**, because a line too long
to parse has left the reader's position untrustworthy: continuing would splice the remainder
of that line into the next frame and produce plausible-looking nonsense. If you are sending
large `details` or `content`, that is the failure you will hit.

Keep frames well under 2 MiB. In practice, tool results are the only thing large enough to
approach it, and a `content` rewrite that returns a whole file listing to save a few bytes is
a bad trade anyway.

## What is not supported

Known gaps, so you do not design against something that does not work.

**You cannot change tool arguments.** pi lets an extension mutate the arguments of a call
before it runs. Epsilon does not, and the reason is in the reference itself: pi performs
**no re-validation after the mutation**. Here the arguments cross a process boundary, so
honouring the change would mean re-validating them against the tool's schema — something pi
does not do. Porting the unsafe half would be worse than the gap.

**You cannot reshape the context.** You can end a run and swap the model between turns, but
you cannot rewrite the messages the provider receives.

**Image content blocks are dropped.** `content` accepts `{"type":"text"}`; other block types
are ignored rather than approximated. A result containing an image is preserved as-is unless
you replace it with text.

**`before_tool_call` and `tool_result` share one channel.** Answer the event you were asked
about. The `type` on the event is how you tell them apart.

## Testing against the harness

`ext-fixture` in this repository answers protocol frames and is what the test suite drives,
so you can model your client on it. It is a test binary, not an example to copy wholesale —
but its shape (read a line, match on `type`, write one line) is the whole client.

Two things are worth checking as you go:

- **Write only protocol frames to stdout.** A stray print is the single most common way to
  break an extension, and it fails confusingly.
- **Answer with the `id` you were given.** A plausible-looking answer with the wrong `id` is
  discarded silently, which looks identical to the harness ignoring you.

## Connecting an MCP server over HTTP

Extensions run as child processes. An MCP server can instead live at a URL and
speak StreamableHTTP. Each `[[mcp_servers]]` entry is one or the other:
`command` spawns stdio, `url` dials HTTP. Both together — or neither — is
rejected with the entry named.

```toml
[[mcp_servers]]
name = "docs"
url = "https://mcp.example.com/mcp"

[mcp_servers.headers]
"x-api-key" = "only-in-this-file"
```

`epsilon --mcp-http docs=https://mcp.example.com/mcp` attaches the same
server for one run. The flag carries only the endpoint: headers and bearer
tokens live in the config file, never on a command line, because command
lines leak into shell history and process lists.

Rules, all enforced when the entry is read, not when the first call runs:

- `Authorization` headers and bearer tokens travel only over `https://`,
  unless the URL is loopback (`localhost`, `127.*`, `::1`). Anything else is
  refused outright.
- `accept`, `mcp-session-id` and `last-event-id` cannot be set. They belong
  to the transport.
- There is no OAuth. A bearer token is static configuration.
- The legacy SSE transport is not accepted. An entry shaped for it is
  rejected with its name.

A project-declared HTTP server starts only in a trusted workspace, exactly
like a project-declared stdio server. A dead server fails the launch with
the server named rather than running without its tools.
