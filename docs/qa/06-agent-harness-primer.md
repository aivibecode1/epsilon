# Design Q&A — What Is an Agent Harness? (A Beginner's Primer)

> Written 2026-08-24 for students and newcomers. Every other document in this repo
> assumes you already know what a "coding agent" is. This one does not. Read it first
> if the rest of the documentation feels like jargon.

## The question

What is an AI agent harness, what does Epsilon actually do, and why did we build one
from scratch when pi, opencode, DeepSeek Harness, and Hermes already exist?

## Part 1 — What happens when you type a prompt

An LLM by itself can only do one thing: read text in, write text out. It cannot see
your files, run commands, or remember yesterday. An **agent** is an LLM placed inside a
program that gives it hands and memory. That program is called the **harness**.

Here is one full turn of Epsilon, step by step:

```
you type:  "why do the tests fail?"
   │
   ▼
1. BUILD THE PROMPT     Epsilon assembles everything the model needs:
                        instructions + your message + recent conversation.
   │
   ▼
2. CALL THE MODEL       Your API key is used to stream tokens from
                        Claude/GPT/Gemini/... over HTTPS.
   │
   ▼
3. READ THE REPLY       Usually the model answers with a TOOL CALL,
                        for example:  bash("cargo test 2>&1 | tail -20")
   │
   ▼
4. CHECK PERMISSIONS    Is this command allowed here? Safe reads run;
                        risky things pause and ask YOU: allow / deny.
   │
   ▼
5. RUN THE TOOL         The command really executes on your machine
                        (inside resource limits), output is captured.
   │
   ▼
6. FEED IT BACK         The tool result is appended to the conversation
                        and sent to the model again (back to step 2).
   │
   ▼
7. FINISH               When the model replies with plain text and no more
                        tool calls, the answer is shown to you and saved.
```

The loop from step 2 to 6 is called **the agent loop**. Everything else in a harness
exists to make that loop safe, cheap, fast, and recoverable.

## Part 2 — The five jobs every harness must do

| Job | What it means | Where Epsilon does it |
|---|---|---|
| Talk to models | Send prompts to Claude/OpenAI/Gemini APIs, handle streaming and errors | `epsilon-llm` |
| Give the agent tools | read/write/edit files, grep, run shell commands | `epsilon-tools` |
| Keep it safe | Permission rules + OS-level limits so the agent cannot wreck your machine | `epsilon-safety` |
| Remember | Save every conversation to disk so sessions survive crashes and restarts | `epsilon-session` |
| Show it to humans | The terminal interface you type into | `epsilon-tui` |

`epsilon-core` holds the loop itself and connects all of the above. `epsilon-cli` is
the single binary you run. Two more crates (`epsilon-ext`, `epsilon-mcp`) let outside
tools plug in without changing core code.

## Part 3 — Words used everywhere else in these docs

- **Steering**: typing a new instruction while the agent is still working. It gets
  added between tool calls instead of being ignored until the run ends.
- **Compaction**: long conversations would eventually overflow the model's context
  window. Compaction summarizes old parts while keeping the raw log on disk untouched.
- **Prompt cache**: providers charge much less (and answer faster) when the start of
  your prompt is byte-for-byte identical to last time. Epsilon builds prompts in tiers
  that keep that prefix stable. This is pi's biggest real-world advantage, so we copied it.
- **JSONL**: one JSON object per line. Epsilon's session files are this format, so
  you can open them in any text editor, diff them in git, or replay them.
- **Permission gate**: a checkpoint before each tool runs. Rules decide allow/ask/deny.
- **Sandbox**: OS-level limits (memory caps, process limits) around shell commands,
  so even an approved command cannot eat all your RAM or spawn runaway processes.
- **StreamFn**: Epsilon's name for "the thing that talks to a model". Its contract:
  it never throws errors; failures arrive as normal messages the loop understands.
- **Faux provider**: a fake scripted model used in tests. All tests therefore run
  offline, cost nothing, and never randomly fail because a real API had a bad day.

## Part 4 — Why base Epsilon on pi?

We audited four production harnesses at source level (`reports/harness-analysis.md`)
and found each has one great strength:

- **pi** has the cleanest agent loop and the cheapest operation (small stable prompts
  = high cache hits = lower bills and faster replies).
- **DeepSeek Harness** has the strongest security (real OS sandboxing).
- **Hermes** has the best prompt-cache discipline and operational reliability habits.
- **opencode** has the friendliest permission system and product polish.

The plan: take pi's skeleton exactly, then bolt on the best part of each other system
at clearly named places. The full reasoning lives in `reports/comparative-analysis.md`
and the ADRs (`design/epsilon-decisions.md`). Nothing was copied blindly; every ported
idea cites its origin in comments and dev logs.

## Part 5 — How to explore this repository without drowning

1. Run it: `cargo run -p epsilon-cli -- --demo` (offline scripted demo, no keys needed).
2. Skim `README.md`, then this primer, then `docs/milestones/` in order. Each
   milestone log tells what was built, what went wrong, and how it was fixed.
3. Code reading order matches the build order: `epsilon-contracts` (just types),
   `epsilon-session` (just storage), `epsilon-llm` (just model calls),
   `epsilon-core` (the loop). By then the rest reads itself.

## Related

- `docs/qa/01-why-rust.md` — why the implementation language is Rust
- `docs/qa/02-lego-brick-methodology.md` — why the codebase is split into small crates
- `docs/qa/03-partial-vs-full-port.md` — what existed after the early milestones
