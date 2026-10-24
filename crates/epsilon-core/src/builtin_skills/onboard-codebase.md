# Onboard a codebase before changing it

Use this skill at the start of work in an unfamiliar repository, or after a long
gap. The goal is a map you can act on, not a summary that sounds plausible.

## Start with the repository's own rules

1. Read `AGENTS.md`, `CONTRIBUTING.md`, and any `README` in the root.
2. Find the build manifest: `Cargo.toml`, `package.json`, `pyproject.toml`,
   `go.mod`, `Makefile`. Read the workspace members and the dependency policy.
3. Find the test command and the release/verification command. Do not guess them
   from the language.

## Map the parts that matter

For each area, record a path and one sentence:

- **Entry points.** Binaries, servers, CLIs, extension hosts.
- **Core loop.** Where a request becomes work becomes a result.
- **Composition.** The one place configuration becomes a running system.
- **State.** Sessions, persistence, migrations, locks, recovery.
- **Tools and permissions.** Registration, execution, approval, sandboxing.
- **Frontends.** TUI, headless, HTTP, RPC — and which one shares the core.
- **Tests.** Unit, integration, failure-path, fixtures, faux providers.
- **Operations.** Config files, cache dirs, logs, install path.

Then trace one real path end to end: a user action, through dispatch, the core,
a tool, and back to what the user sees. The trace finds the lies in the diagram.

## Read for invariants, not volume

Look for the rules the project refuses to break: one composition point, no
library depending on the CLI, settings writes that are atomic, refusals that
return errors instead of exiting. These are the constraints your change must
respect, and they are usually stated more clearly than the architecture.

## Deliverable

Produce a map with:

- the three commands you will actually run (build, test, verify);
- ten to twenty file paths with a phrase each;
- the invariants your change must keep;
- a short **unknowns** list, with the cheapest check for each.

Do not edit anything during onboarding. The first edit made before the map is
usually in the wrong layer, and it teaches you the architecture backwards.
