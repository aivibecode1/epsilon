//! Child-process environment hygiene (DSH `subprocess` defensive pattern).
//!
//! The problem this closes: a shell tool inherits the harness's whole
//! environment, so `env`, `printenv`, `set`, or a one-line script can print the
//! operator's API keys — and a tool *result* is not transient. It is written to
//! the model's context, to the JSONL session on disk, and into any transcript
//! the user exports or shares. One careless command permanently leaks every
//! credential the harness can see.
//!
//! The rule (ported from DSH's subprocess spawn path): before spawning a child,
//! drop every environment variable whose name looks like a credential. The
//! match is deliberately broad and cheap — a false positive costs a child
//! process one variable it probably did not need, a false negative costs the
//! operator every key on the machine.
//!
//! Escape hatch: `EPSILON_ENV_PASSTHROUGH=MY_TOKEN,OTHER_KEY` names variables
//! that are exempt, for the rare build or script that genuinely needs one.

use tokio::process::Command as TokioCommand;

/// Substrings that mark an environment variable as a credential (case-insensitive
/// on the upper-cased name, matching DSH's `*KEY*` / `*SECRET*` test).
const SECRET_MARKERS: &[&str] = &[
    "KEY",
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "APIKEY",
    "AUTH",
];

/// Variables exempt from scrubbing even when they match a marker.
///
/// `AUTH` is a broad marker, so a couple of widely-set, harmless names are
/// exempted explicitly rather than losing the marker's coverage.
const EXEMPT: &[&str] = &["GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "SSH_AUTH_SOCK"];

/// The passthrough list from `EPSILON_ENV_PASSTHROUGH` (comma-separated).
fn passthrough() -> Vec<String> {
    std::env::var("EPSILON_ENV_PASSTHROUGH")
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `name` should be removed from a child's environment.
///
/// Pure and public so the rule is testable without spawning anything.
pub fn is_secret_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if EXEMPT.contains(&upper.as_str()) {
        return false;
    }
    if passthrough().iter().any(|p| p.eq_ignore_ascii_case(name)) {
        return false;
    }
    SECRET_MARKERS.iter().any(|m| upper.contains(m))
}

/// Names in the current process environment that [`is_secret_env_name`] flags.
/// Used by tests and diagnostics — the values are never read out.
pub fn secret_env_names() -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(k, _)| k.to_str().map(str::to_string))
        .filter(|k| is_secret_env_name(k))
        .collect()
}

/// Remove every credential-looking variable from a tokio child command.
///
/// Called on every tool-spawned child. The child keeps PATH, HOME/TEMP, locale
/// and platform plumbing — everything a build needs — and loses the keys.
pub fn scrub_secret_env(cmd: &mut TokioCommand) {
    for name in secret_env_names() {
        cmd.env_remove(&name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_shaped_names_are_flagged() {
        for name in [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "DB_PASSWORD",
            "MYSQL_PASSWD",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "MY_APIKEY",
            "deepseek_token",
        ] {
            assert!(is_secret_env_name(name), "{name} must be scrubbed");
        }
    }

    #[test]
    fn ordinary_plumbing_survives() {
        // The scrub must not break the ability to build or run anything: these
        // are the variables a compiler, a package manager and the OS need.
        for name in [
            "PATH",
            "HOME",
            "USERPROFILE",
            "TEMP",
            "TMP",
            "SystemRoot",
            "windir",
            "COMSPEC",
            "PATHEXT",
            "LANG",
            "LC_ALL",
            "TERM",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "NODE_PATH",
            "PYTHONPATH",
            "EDITOR",
            "PWD",
            "SHELL",
            "NUMBER_OF_PROCESSORS",
        ] {
            assert!(!is_secret_env_name(name), "{name} must survive the scrub");
        }
    }

    #[test]
    fn git_identity_is_exempt_despite_matching_auth() {
        assert!(!is_secret_env_name("GIT_AUTHOR_NAME"));
        assert!(!is_secret_env_name("GIT_AUTHOR_EMAIL"));
        // ...but the actual credential is not.
        assert!(is_secret_env_name("GITLAB_TOKEN"));
        assert!(is_secret_env_name("GIT_SSH_COMMAND_KEY"));
    }

    #[test]
    fn a_helper_path_is_not_a_secret_and_is_left_alone() {
        // GIT_ASKPASS names a *program* that returns a password; the path itself
        // carries no credential, and removing it would break a legitimate
        // credential-helper based workflow without protecting anything.
        assert!(!is_secret_env_name("GIT_ASKPASS"));
    }

    #[tokio::test]
    async fn a_spawned_child_does_not_see_this_process_credentials() {
        // End-to-end proof with a real child process and a real environment:
        // compute the names this process would scrub, spawn a child whose only
        // job is to print its environment, and assert none of them appear.
        // Offline; no network; no secret value is ever read or printed.
        let scrubbed = secret_env_names();

        #[cfg(windows)]
        let (program, args) = (
            "powershell",
            vec![
                "-NoLogo".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                "Get-ChildItem Env: | ForEach-Object { $_.Name }".to_string(),
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = ("env", Vec::<String>::new());

        let mut cmd = TokioCommand::new(program);
        cmd.args(&args);
        scrub_secret_env(&mut cmd);
        let out = cmd.output().await.expect("spawn");
        let visible = String::from_utf8_lossy(&out.stdout);
        let lower = visible.to_ascii_lowercase();

        for name in &scrubbed {
            assert!(
                !lower.contains(&name.to_ascii_lowercase()),
                "credential-shaped variable {name} leaked into a child process"
            );
        }
        // The child still has a usable environment: scrubbing is not env_clear.
        assert!(
            visible.contains("PATH") || visible.contains("Path"),
            "PATH must survive the scrub (the child must still be able to run things)"
        );
        assert!(
            secret_env_names().len() == scrubbed.len(),
            "scrubbing a child must not mutate the parent's own environment"
        );
    }

    #[test]
    fn the_marker_set_covers_what_providers_actually_use() {
        // Every provider env key the catalog knows about must be caught.
        for key in [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "DEEPSEEK_API_KEY",
            "GEMINI_API_KEY",
            "XAI_API_KEY",
            "OPENROUTER_API_KEY",
            "MOONSHOT_API_KEY",
            "ZAI_API_KEY",
            "GROQ_API_KEY",
        ] {
            assert!(is_secret_env_name(key), "{key} must be scrubbed");
        }
    }
}
