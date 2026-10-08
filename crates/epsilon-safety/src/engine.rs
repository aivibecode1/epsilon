//! The ruleset engine: stacked layers, last-match-wins evaluation.

use serde::{Deserialize, Serialize};

use crate::wildcards::glob_match;

/// What a rule decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Proceed without consulting anyone.
    Allow,
    /// Route through the approval UI.
    Ask,
    /// Refuse.
    Deny,
}

/// One permission rule. `capability` is a tool name (`read`, `write`, `bash`, …) or
/// pseudo-capability (`external_dir`, `doom_loop`); `pattern` is a glob over the call's
/// subject (path / command head / `*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Capability this rule governs; `*` matches all capabilities.
    pub capability: String,
    /// Glob matched against the call subject; `*` matches every subject.
    pub pattern: String,
    /// What happens on match.
    pub action: Action,
}

impl Rule {
    /// Convenience constructor.
    pub fn new(capability: &str, pattern: &str, action: Action) -> Self {
        Self {
            capability: capability.to_string(),
            pattern: pattern.to_string(),
            action,
        }
    }

    fn matches(&self, capability: &str, subject: &str) -> bool {
        (self.capability == "*" || self.capability == capability)
            && glob_match(&self.pattern, subject)
    }
}

/// A named set of rules forming one layer (defaults / global config / project / CLI).
#[derive(Debug, Clone)]
pub struct Ruleset {
    /// Layer name for diagnostics (`"defaults"`, `"project"`…).
    pub name: String,
    /// Rules in priority order (later rules in the layer override earlier ones).
    pub rules: Vec<Rule>,
}

/// The engine: flattened evaluation across layers, later layers winning.
///
/// This mirrors opencode's `rulesets.flat().findLast(...)` with an important addition:
/// **any deny wins immediately** regardless of position. A project cannot accidentally
/// re-allow something the global config denied — security decisions are order-independent
/// (the DSH monotonic-guard principle applied to rule stacking).
///
/// `Clone` because a subagent must evaluate the **same** rules while holding a **different**
/// session memory (`PermissionGate::child_gate`). A shared engine would have forced a choice
/// between leaking the memory and re-deriving the rules; a cloned one is a copy of a value that
/// never changes after construction.
#[derive(Debug, Clone)]
pub struct PermissionEngine {
    layers: Vec<Ruleset>,
}

impl PermissionEngine {
    /// Engine with only the built-in defaults.
    pub fn with_defaults() -> Self {
        Self {
            layers: vec![default_ruleset()],
        }
    }

    /// Engine with defaults plus extra layers, evaluated after them (later wins).
    pub fn layered(extra: Vec<Ruleset>) -> Self {
        let mut layers = vec![default_ruleset()];
        layers.extend(extra);
        Self { layers }
    }

    /// Evaluate a capability/subject pair.
    pub fn evaluate(&self, capability: &str, subject: &str) -> Action {
        let mut decision: Option<Action> = None;
        for layer in &self.layers {
            for rule in &layer.rules {
                if !rule.matches(capability, subject) {
                    continue;
                }
                if rule.action == Action::Deny {
                    // Monotonic deny: immediate, order-independent refusal.
                    return Action::Deny;
                }
                decision = Some(rule.action);
            }
        }
        decision.unwrap_or(Action::Ask)
    }
}

/// Built-in defaults. Philosophy (opencode-grade maturity, sandbox-aware posture):
/// reads are free inside the workspace shape we can see; secrets ask; writes allow because
/// workspace enforcement arrives with the M10 confinement layer (documented gap); shell asks
/// by default because Layer 2 does not exist yet — safe until proven otherwise.
pub fn default_ruleset() -> Ruleset {
    Ruleset {
        name: "defaults".into(),
        rules: vec![
            Rule::new("read", "*", Action::Allow),
            Rule::new("grep", "*", Action::Allow),
            Rule::new("glob", "*", Action::Allow),
            Rule::new("ls", "*", Action::Allow),
            // Secrets: reading environment material requires approval…
            Rule::new("read", "*.env", Action::Ask),
            Rule::new("read", "*.env.*", Action::Ask),
            // …but examples/templates are public by design.
            Rule::new("read", "*.env.example", Action::Allow),
            Rule::new("write", "*", Action::Allow),
            Rule::new("edit", "*", Action::Allow),
            // Anything outside the declared workspace asks (tools downgrade capability).
            Rule::new("external_dir", "*", Action::Ask),
            // Shell and network: ask-first until confinement exists.
            Rule::new("bash", "*", Action::Ask),
            Rule::new("webfetch", "*", Action::Ask),
        ],
    }
}

/// Plan-mode ruleset (opencode's `plan` agent pattern): read-only research.
///
/// Mutating capabilities are DENIED — not asked — because a plan run must never touch
/// the tree even if the user approves mid-run; the correct escape is to exit plan mode,
/// which keeps the decision out of per-call approval fatigue. Note the engine's
/// monotonic-deny semantics make this boundary hold no matter what later layers allow.
/// Internal session bookkeeping (compaction summaries, JSONL appends) does not travel
/// through the tool gate at all, so long planning runs still persist their log.
pub fn plan_ruleset() -> Ruleset {
    Ruleset {
        name: "plan".into(),
        rules: vec![
            Rule::new("write", "*", Action::Deny),
            Rule::new("edit", "*", Action::Deny),
            // Command heads cannot establish read-only shell behavior. Refuse
            // shell execution instead of classifying arbitrary command strings.
            Rule::new("bash", "*", Action::Deny),
            Rule::new("shell", "*", Action::Deny),
        ],
    }
}

/// A named permission posture (ADR-032): a bundle of allows, never a pardon.
///
/// Project and user deny rules layer above every preset and win — deny is
/// monotonic, so `FullAccess` means "allow what nothing forbids", never
/// "forbid nothing". Parsed from the `--preset` flag and (later) the front-end
/// pickers, which is why the names are kebab-case words, not enum-case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// Today's posture under a name: the defaults plus project rules, asking
    /// where they ask. The default because it changes nothing.
    #[default]
    Auto,
    /// Reads and writes under the workspace; asks outside it (via the
    /// `external_dir` downgrade the tools already apply), and asks shell and
    /// secrets like the defaults. Pinned explicitly rather than inherited, so
    /// a future defaults-tightening does not silently move it.
    WorkspaceWrite,
    /// Deny every side-effecting capability; allow reads. Unknown future
    /// tools fall through to `Ask` (the engine default), never to allow.
    ReadOnly,
    /// Allow everything nothing forbids. Choosing it is the
    /// privilege-raising act, so the front ends gate it behind the risk
    /// confirmation — the engine itself just allows.
    FullAccess,
}

impl Preset {
    /// Parse a `--preset` value. Unknown names are an error naming the four,
    /// never a silent fallback (a fallback would run a different policy than
    /// the one the reader asked for).
    pub fn parse(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Preset::Auto),
            "workspace-write" => Ok(Preset::WorkspaceWrite),
            "read-only" => Ok(Preset::ReadOnly),
            "danger-full-access" => Ok(Preset::FullAccess),
            other => Err(format!(
                "unknown preset '{other}' — one of: auto, workspace-write, read-only, danger-full-access"
            )),
        }
    }

    /// The kebab-case name (flag values, notices, session records).
    pub fn name(self) -> &'static str {
        match self {
            Preset::Auto => "auto",
            Preset::WorkspaceWrite => "workspace-write",
            Preset::ReadOnly => "read-only",
            Preset::FullAccess => "danger-full-access",
        }
    }

    /// Whether switching *to* this preset raises privilege and must confirm
    /// (D-080: only the raising direction gates; switching down submits
    /// immediately).
    pub fn raises_privilege(self) -> bool {
        matches!(self, Preset::FullAccess)
    }
}

impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// The ruleset a preset contributes, layered over the defaults (later wins;
/// any deny still wins immediately — ADR-006 stacking).
///
/// `workspace` scopes the workspace-write allows; the other presets do not
/// read it. Kept as one function rather than four so the preset table has
/// exactly one row per name.
pub fn preset_ruleset(preset: Preset, workspace: &std::path::Path) -> Ruleset {
    let ws = workspace.to_string_lossy().replace('\\', "/");
    let under_ws = format!("{ws}/*");
    match preset {
        Preset::Auto => Ruleset {
            // Today's posture under a name: no extra rules. Named for
            // diagnostics (a layer called "preset:auto" reads better than a
            // layer called "defaults" when the reader chose it deliberately).
            name: "preset:auto".into(),
            rules: vec![],
        },
        Preset::WorkspaceWrite => Ruleset {
            // Explicit, not inherited: these allows survive a future
            // defaults-tightening, which is the point of choosing this
            // preset deliberately rather than running unmarked defaults.
            name: "preset:workspace-write".into(),
            rules: vec![
                Rule::new("read", "*", Action::Allow),
                Rule::new("grep", "*", Action::Allow),
                Rule::new("glob", "*", Action::Allow),
                Rule::new("ls", "*", Action::Allow),
                Rule::new("write", &under_ws, Action::Allow),
                Rule::new("edit", &under_ws, Action::Allow),
            ],
        },
        Preset::ReadOnly => Ruleset {
            name: "preset:read-only".into(),
            rules: vec![
                Rule::new("write", "*", Action::Deny),
                Rule::new("edit", "*", Action::Deny),
                Rule::new("bash", "*", Action::Deny),
                Rule::new("shell", "*", Action::Deny),
            ],
        },
        Preset::FullAccess => Ruleset {
            name: "preset:danger-full-access".into(),
            rules: vec![Rule::new("*", "*", Action::Allow)],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_match_wins_within_a_layer() {
        let engine = PermissionEngine::with_defaults();
        assert_eq!(engine.evaluate("read", "src/main.rs"), Action::Allow);
        assert_eq!(engine.evaluate("read", ".env"), Action::Ask);
        assert_eq!(engine.evaluate("read", ".env.example"), Action::Allow);
    }

    #[test]
    fn deny_is_monotonic_across_layers() {
        // A later layer cannot re-allow what an earlier layer denied.
        let engine = PermissionEngine::layered(vec![
            Ruleset {
                name: "g".into(),
                rules: vec![Rule::new("bash", "*", Action::Deny)],
            },
            Ruleset {
                name: "project".into(),
                rules: vec![Rule::new("bash", "cargo", Action::Allow)],
            },
        ]);
        assert_eq!(engine.evaluate("bash", "cargo"), Action::Deny);
    }

    #[test]
    fn unknown_capabilities_ask_by_default() {
        assert_eq!(
            PermissionEngine::with_defaults().evaluate("mystery_tool", "*"),
            Action::Ask
        );
    }

    #[test]
    fn later_layers_override_allows() {
        let engine = PermissionEngine::layered(vec![Ruleset {
            name: "cli".into(),
            rules: vec![Rule::new("bash", "*", Action::Allow)],
        }]);
        assert_eq!(engine.evaluate("bash", "git"), Action::Allow);
        assert_eq!(engine.evaluate("webfetch", "*"), Action::Ask); // untouched default
    }

    #[test]
    fn plan_mode_denies_mutations_even_when_project_reallows() {
        // opencode plan-agent parity: research runs cannot touch the tree, and the
        // monotonic-deny rule means no lower layer can punch through.
        let engine = PermissionEngine::layered(vec![
            Ruleset {
                name: "plan".into(),
                rules: crate::plan_ruleset().rules,
            },
            Ruleset {
                name: "project".into(),
                rules: vec![Rule::new("write", "*", Action::Allow)],
            },
        ]);
        assert_eq!(engine.evaluate("write", "src/main.rs"), Action::Deny);
        assert_eq!(engine.evaluate("edit", "src/main.rs"), Action::Deny);
        // Reads and search stay free — planning is a research activity.
        assert_eq!(engine.evaluate("read", "src/main.rs"), Action::Allow);
        assert_eq!(engine.evaluate("grep", "*"), Action::Allow);
    }

    #[test]
    fn plan_mode_blocks_commit_and_push_commands() {
        let engine = PermissionEngine::layered(vec![Ruleset {
            name: "plan".into(),
            rules: crate::plan_ruleset().rules,
        }]);
        assert_eq!(engine.evaluate("bash", "git commit -m x"), Action::Deny);
        assert_eq!(
            engine.evaluate("bash", "git push origin main"),
            Action::Deny
        );
        // Even apparently read-only commands are refused: there is no portable
        // read-only shell boundary inferred from command strings.
        assert_eq!(engine.evaluate("bash", "cargo test"), Action::Deny);
    }

    /// Preset names parse round-trip and reject unknowns by naming the four.
    ///
    /// A silent fallback would run a different policy than the one asked for.
    #[test]
    fn preset_names_parse_and_reject() {
        for (text, want) in [
            ("auto", Preset::Auto),
            ("workspace-write", Preset::WorkspaceWrite),
            ("read-only", Preset::ReadOnly),
            ("danger-full-access", Preset::FullAccess),
            ("  Auto  ", Preset::Auto),
        ] {
            let got = Preset::parse(text).expect("a known preset parses");
            assert_eq!(got, want, "{text}");
            assert_eq!(got.name(), got.to_string());
        }
        let err = Preset::parse("yolo").expect_err("unknown must fail");
        assert!(
            err.contains("auto") && err.contains("read-only"),
            "the error must name the choices: {err}"
        );
        assert_eq!(Preset::default(), Preset::Auto);
        assert!(Preset::FullAccess.raises_privilege());
        assert!(!Preset::WorkspaceWrite.raises_privilege());
        assert!(!Preset::ReadOnly.raises_privilege());
        assert!(!Preset::Auto.raises_privilege());
    }

    /// `read-only` denies mutation and exec, allows reads, and leaves the
    /// unknown to `Ask` — never to allow.
    #[test]
    fn read_only_denies_mutation_and_exec() {
        let ws = std::path::Path::new("/ws");
        let engine = PermissionEngine::layered(vec![preset_ruleset(Preset::ReadOnly, ws)]);
        assert_eq!(engine.evaluate("write", "/ws/a.txt"), Action::Deny);
        assert_eq!(engine.evaluate("edit", "/ws/a.txt"), Action::Deny);
        assert_eq!(engine.evaluate("bash", "echo hi"), Action::Deny);
        assert_eq!(engine.evaluate("shell", "echo hi"), Action::Deny);
        assert_eq!(engine.evaluate("read", "/ws/a.txt"), Action::Allow);
        assert_eq!(
            engine.evaluate("mystery_tool", "*"),
            Action::Ask,
            "a future tool must ask, never inherit allow"
        );
    }

    /// `workspace-write` pins its allows explicitly under the workspace.
    #[test]
    fn workspace_write_allows_under_the_workspace() {
        let ws = std::path::Path::new("/ws");
        let engine = PermissionEngine::layered(vec![preset_ruleset(Preset::WorkspaceWrite, ws)]);
        assert_eq!(engine.evaluate("write", "/ws/a.txt"), Action::Allow);
        assert_eq!(engine.evaluate("edit", "/ws/sub/b.txt"), Action::Allow);
        assert_eq!(engine.evaluate("read", "/ws/a.txt"), Action::Allow);
        assert_eq!(
            engine.evaluate("bash", "echo hi"),
            Action::Ask,
            "shell keeps the default ask posture"
        );
        // Outside-workspace containment is the tools' `external_dir`
        // downgrade, not this layer: the preset pins what it promises
        // (under-workspace allows) and inherits the rest from the defaults.
        assert_eq!(
            engine.evaluate("write", "/elsewhere/a.txt"),
            Action::Allow,
            "engine-level: defaults allow; tools downgrade outside to external_dir/Ask"
        );
    }

    /// `danger-full-access` allows what nothing forbids — and a project deny
    /// still forbids it. Monotonic deny is the point of the engine.
    #[test]
    fn full_access_allows_but_project_deny_still_wins() {
        let ws = std::path::Path::new("/ws");
        let engine = PermissionEngine::layered(vec![preset_ruleset(Preset::FullAccess, ws)]);
        assert_eq!(engine.evaluate("bash", "rm -rf /"), Action::Allow);
        assert_eq!(engine.evaluate("read", ".env"), Action::Allow);
        let guarded = PermissionEngine::layered(vec![
            preset_ruleset(Preset::FullAccess, ws),
            Ruleset {
                name: "project".into(),
                rules: vec![Rule::new("bash", "*", Action::Deny)],
            },
        ]);
        assert_eq!(
            guarded.evaluate("bash", "rm -rf /"),
            Action::Deny,
            "a preset is a bundle of allows, never a pardon"
        );
    }

    /// `auto` adds no rules: today's posture under a name.
    #[test]
    fn auto_is_todays_posture() {
        let ws = std::path::Path::new("/ws");
        let engine = PermissionEngine::layered(vec![preset_ruleset(Preset::Auto, ws)]);
        assert_eq!(engine.evaluate("read", "src/main.rs"), Action::Allow);
        assert_eq!(engine.evaluate("bash", "git"), Action::Ask);
        assert_eq!(engine.evaluate("write", "/ws/a.txt"), Action::Allow);
    }
}
