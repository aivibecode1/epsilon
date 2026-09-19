//! Which files a workspace does not want walked.
//!
//! # Why this is its own module
//!
//! `.gitignore` is a promise the tools make and the repository records in a
//! language with real subtleties: negation, anchoring, directory-only patterns,
//! and — the part that decides most real cases — **the last matching pattern
//! wins**, including from a nested file. A substring check passes a toy test and
//! then lists `target/debug` and every file in a build output directory, which is
//! exactly the context a coding agent cannot afford to waste.
//!
//! It lives in `epsilon-core` rather than in the tools because two crates need
//! the same answer: the tool walkers, and the TUI's `@`-file completion. Two
//! copies of a rule this fiddly drift, and the symptom of drift is a file that
//! `grep` refuses to search appearing in the completion list.
//!
//! # What is implemented
//!
//! The subset of git's rules that decides real repositories:
//!
//! | Syntax | Meaning |
//! |---|---|
//! | `name` | any path component may match |
//! | `/name` | anchored at the directory holding the pattern |
//! | `dir/` | directories only |
//! | `*.ext`, `name*`, `a?c` | shell wildcards, `*` does not cross `/` |
//! | `**/x`, `x/**` | matches at any depth, or everything below |
//! | `!name` | re-includes a previously ignored path |
//! | `#`, blank | comment, ignorable |
//!
//! Not implemented, and named here rather than discovered later: character
//! classes (`[a-z]`), escaped metacharacters (`\*`), and the global
//! `core.excludesFile`. Each of those is rare in the files this walks, and a
//! silently wrong answer is worse than a documented gap.

use std::path::{Path, PathBuf};

/// Directories no walk should ever descend into, whatever `.gitignore` says.
///
/// These are not preferences: a build directory or a package manager's cache can
/// hold hundreds of thousands of files, and walking one turns a fast tool into a
/// stalled one. `.git` is here because its contents are not workspace content.
pub const ALWAYS_SKIP: &[&str] = &[".git", "target", "node_modules", "dist"];

/// One compiled pattern from a `.gitignore` file.
#[derive(Debug, Clone)]
struct Rule {
    /// The pattern, already normalised of its prefixes.
    pattern: String,
    /// Directory the pattern's file lived in, relative to the workspace root.
    /// `""` for the root itself.
    base: String,
    /// `true` for a leading `/`: match only directly under `base`.
    anchored: bool,
    /// `true` for a trailing `/`: match directories only.
    dir_only: bool,
    /// `true` for a leading `!`: re-include rather than ignore.
    negated: bool,
}

/// The ignore rules that apply to one workspace.
///
/// Built once per walk and then queried per path. Constructing it reads every
/// `.gitignore` in the tree (bounded by the same caps as the walk), because a
/// nested file changes the answer for the subtree below it and discovering that
/// lazily is how a walker ends up inconsistent about siblings.
#[derive(Debug, Clone, Default)]
pub struct IgnoreSet {
    rules: Vec<Rule>,
}

impl IgnoreSet {
    /// An empty set: nothing is ignored on the strength of a `.gitignore`.
    ///
    /// Used by callers that have no workspace root (a unit test, a one-off
    /// search of a directory that is not a repository).
    pub fn none() -> Self {
        Self::default()
    }

    /// Load `.gitignore` files from `root` down, bounded by `max_files`.
    ///
    /// Bounded on purpose: a monorepo can hold thousands of these, and a tool
    /// that spends its whole budget reading ignore files has none left to do the
    /// work it was asked for. When the cap is hit, the rules found so far still
    /// apply — the walk is narrower than ideal, never wrong about what it
    /// searched.
    pub fn load(root: &Path, max_files: usize) -> Self {
        let mut set = Self::default();
        let mut seen = 0usize;
        let mut queue: Vec<PathBuf> = vec![root.to_path_buf()];
        while let Some(dir) = queue.pop() {
            if seen >= max_files {
                break;
            }
            let file = dir.join(".gitignore");
            if let Ok(text) = std::fs::read_to_string(&file) {
                seen += 1;
                let base = dir
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                set.rules.extend(parse(&text, &base));
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let name = entry.file_name().to_string_lossy().to_string();
                if skip_dir(&name) {
                    continue;
                }
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    queue.push(entry.path());
                }
            }
        }
        set
    }

    /// Does this path belong to the ignored set?
    ///
    /// `relative` is workspace-relative with `/` separators, and `is_dir` says
    /// whether the entry is a directory.
    ///
    /// # Why this walks the ancestors
    ///
    /// Git decides per path component, and the difference is not academic. With
    ///
    /// ```text
    /// dir/
    /// !dir/
    /// ```
    ///
    /// `dir/` is ignored, then `dir` itself is re-included — and because git
    /// never looks inside an ignored directory, **everything under `dir` comes
    /// back too**. Testing only the full path gets this wrong twice: the rule
    /// `dir/` looks for a path that *equals* `dir`, so `dir/keep.txt` is missed
    /// entirely, and the negation never reaches its children.
    ///
    /// So each ancestor is resolved in order. The first one that is ignored
    /// decides, and anything below it is ignored with it — which is exactly the
    /// behavior that makes `!` useful for exempting a whole tree.
    pub fn is_ignored(&self, relative: &str, is_dir: bool) -> bool {
        let clean = relative.trim_matches('/');
        if clean.is_empty() {
            return false;
        }
        let parts: Vec<&str> = clean.split('/').filter(|p| !p.is_empty()).collect();
        let mut ignored = false;
        for depth in 1..=parts.len() {
            let prefix = parts[..depth].join("/");
            let prefix_is_dir = depth < parts.len() || is_dir;
            let mut decision = None;
            for rule in &self.rules {
                if rule.matches(&prefix, prefix_is_dir) {
                    decision = Some(!rule.negated);
                }
            }
            if let Some(d) = decision {
                ignored = d;
            }
            if ignored {
                // Nothing below an ignored component can be re-included by a
                // rule that only names a deeper path: git never descends, so a
                // nested `.gitignore` never gets a chance to speak.
                return true;
            }
        }
        false
    }

    /// How many rules were loaded. Useful in a diagnostic, and in tests that
    /// assert a nested file was actually read.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether no rules were found at all.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

impl Rule {
    fn matches(&self, relative: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        // A rule only applies at or below the directory holding its file.
        let scoped = if self.base.is_empty() {
            relative
        } else {
            match relative.strip_prefix(&self.base) {
                Some(rest) => rest.trim_start_matches('/'),
                None => return false,
            }
        };
        if scoped.is_empty() {
            return false;
        }
        // A directory pattern without a slash names exactly one component. This
        // is what makes `dir/` and `dir` mean "that directory", not "any path
        // that starts with it" — the descendant case is handled by the ancestor
        // walk in `is_ignored`, which is where git handles it too.
        if self.dir_only && !self.pattern.contains('/') {
            return glob_match(&self.pattern, scoped) && !scoped.contains('/');
        }
        if self.anchored {
            return glob_match(&self.pattern, scoped);
        }
        // An unanchored pattern matches at any depth, so `target` ignores
        // `a/b/target`. The component-wise walk is what stops `*` from crossing
        // a separator, and it stops a name pattern from matching a deeper path
        // as a whole.
        let components: Vec<&str> = scoped.split('/').collect();
        for (index, component) in components.iter().enumerate() {
            if glob_match(&self.pattern, component) {
                return true;
            }
            // A pattern with a separator may also match a multi-component tail,
            // which is how `src/*.rs` works without being anchored.
            if self.pattern.contains('/')
                && glob_match(&self.pattern, &components[index..].join("/"))
            {
                return true;
            }
        }
        false
    }
}

/// Parse one `.gitignore` body into rules scoped to `base`.
fn parse(text: &str, base: &str) -> Vec<Rule> {
    let mut rules = Vec::new();
    for raw in text.lines() {
        // A trailing space is significant only when escaped, and that escape is
        // not supported here; trimming is the closer approximation for the files
        // this walks.
        let line = raw.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (mut pattern, negated) = match line.strip_prefix('!') {
            Some(rest) => (rest.to_string(), true),
            None => (line.to_string(), false),
        };
        if pattern.is_empty() {
            continue;
        }
        let dir_only = pattern.ends_with('/');
        if dir_only {
            pattern.pop();
        }
        let anchored = pattern.starts_with('/') || pattern.trim_end_matches('/').contains('/');
        if let Some(stripped) = pattern.strip_prefix('/') {
            pattern = stripped.to_string();
        }
        if pattern.is_empty() {
            continue;
        }
        rules.push(Rule {
            pattern,
            base: base.to_string(),
            anchored,
            dir_only,
            negated,
        });
    }
    rules
}

/// Glob match where `*` and `?` do not cross `/`, and `**` matches across it.
///
/// Iterative with backtracking rather than recursive: a pathological pattern
/// against a long path is otherwise a stack-overflow away from taking the
/// process down, and a tool that can be crashed by a `.gitignore` is a tool that
/// will be.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    // `**/` at the front may match nothing at all, which is git's rule and the
    // one users rely on: `**/generated` has to ignore a top-level `generated`
    // directory, not only a nested one. Retrying without the prefix is cheaper
    // and clearer than teaching the scanner a zero-width case.
    if let Some(rest) = pattern.strip_prefix("**/")
        && !rest.is_empty()
        && glob_match(rest, text)
    {
        return true;
    }
    glob_match_inner(pattern, text)
}

fn glob_match_inner(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // Where to resume if the current `*` turns out to have consumed too little.
    let mut star: Option<(usize, usize)> = None;

    while ti < t.len() {
        if pi < p.len() {
            match p[pi] {
                '*' => {
                    // `**` behaves like `*` here but is allowed to span `/`.
                    let double = p.get(pi + 1) == Some(&'*');
                    star = Some((pi, ti));
                    pi += if double { 2 } else { 1 };
                    continue;
                }
                '?' if t[ti] != '/' => {
                    pi += 1;
                    ti += 1;
                    continue;
                }
                c if c == t[ti] => {
                    pi += 1;
                    ti += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some((sp, st)) => {
                // The star that could still absorb one more character.
                let double = p.get(sp + 1) == Some(&'*');
                let next = st + 1;
                if !double && t.get(st) == Some(&'/') {
                    // A single star cannot cross a separator: give up on it.
                    return false;
                }
                star = Some((sp, next));
                pi = sp + if double { 2 } else { 1 };
                ti = next;
            }
            None => return false,
        }
    }
    // Trailing `*`/`**` may match nothing.
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Should a directory with this name be skipped without consulting any rule?
pub fn skip_dir(name: &str) -> bool {
    ALWAYS_SKIP.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(rules: &str) -> IgnoreSet {
        IgnoreSet {
            rules: parse(rules, ""),
        }
    }

    #[test]
    fn a_bare_name_matches_at_any_depth() {
        let s = set("target\n");
        assert!(s.is_ignored("target", true));
        assert!(s.is_ignored("crates/a/target", true));
        assert!(s.is_ignored("target/debug/build.rs", false));
        assert!(!s.is_ignored("src/targets.rs", false));
    }

    #[test]
    fn a_leading_slash_anchors_the_pattern() {
        let s = set("/build\n");
        assert!(s.is_ignored("build", true));
        assert!(
            !s.is_ignored("crates/a/build", true),
            "an anchored pattern must not match deeper"
        );
    }

    #[test]
    fn a_trailing_slash_matches_directories_only() {
        let s = set("logs/\n");
        assert!(s.is_ignored("logs", true));
        assert!(
            !s.is_ignored("logs", false),
            "a directory-only pattern must not match a file"
        );
    }

    #[test]
    fn a_wildcard_does_not_cross_a_separator() {
        let s = set("*.log\n");
        assert!(s.is_ignored("app.log", false));
        assert!(s.is_ignored("var/app.log", false));
        let nested = set("src/*.rs\n");
        assert!(nested.is_ignored("src/main.rs", false));
        assert!(
            !nested.is_ignored("src/deep/main.rs", false),
            "`*` must not span `/`"
        );
    }

    #[test]
    fn a_double_star_crosses_separators() {
        let s = set("**/generated\n");
        assert!(s.is_ignored("generated", true));
        assert!(s.is_ignored("a/b/generated", true));
        let below = set("vendor/**\n");
        assert!(below.is_ignored("vendor/a/b/c.rs", false));
    }

    #[test]
    fn the_last_matching_rule_wins() {
        // The rule that decides real cases: a negation after a broad ignore.
        let s = set("*.txt\n!keep.txt\n");
        assert!(s.is_ignored("notes.txt", false));
        assert!(!s.is_ignored("keep.txt", false));

        // And the reverse order re-ignores it.
        let re = set("!keep.txt\n*.txt\n");
        assert!(re.is_ignored("keep.txt", false));
    }

    #[test]
    fn comments_and_blank_lines_are_not_patterns() {
        let s = set("# a comment\n\n   \nreal\n");
        assert_eq!(s.len(), 1);
        assert!(s.is_ignored("real", false));
        assert!(!s.is_ignored("# a comment", false));
    }

    #[test]
    fn a_nested_file_scopes_its_own_rules() {
        let rules = parse("private\n", "crates/api");
        let s = IgnoreSet { rules };
        assert!(s.is_ignored("crates/api/private/keys.rs", false));
        assert!(
            !s.is_ignored("crates/web/private/keys.rs", false),
            "a nested .gitignore must not leak outside its directory"
        );
    }

    #[test]
    fn a_question_mark_matches_one_non_separator_character() {
        let s = set("a?c\n");
        assert!(s.is_ignored("abc", false));
        assert!(!s.is_ignored("ac", false));
        assert!(
            !s.is_ignored("a/c", false),
            "`?` must not match a separator"
        );
    }

    #[test]
    fn the_always_skip_list_covers_the_directories_that_break_walks() {
        for name in [".git", "target", "node_modules", "dist"] {
            assert!(skip_dir(name), "{name} must never be walked");
        }
        assert!(!skip_dir("src"));
        // A dot-directory that is real content is not on the list.
        assert!(!skip_dir(".github"));
    }

    #[test]
    fn loading_reads_nested_files_and_respects_the_cap() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join(".gitignore"), "*.log\n").expect("root ignore");
        let nested = tmp.path().join("crates").join("a");
        std::fs::create_dir_all(&nested).expect("nested");
        std::fs::write(nested.join(".gitignore"), "secret.txt\n").expect("nested ignore");

        let s = IgnoreSet::load(tmp.path(), 64);
        assert!(s.is_ignored("app.log", false), "root rule missing");
        assert!(
            s.is_ignored("crates/a/secret.txt", false),
            "nested rule missing"
        );
        assert!(
            !s.is_ignored("crates/b/secret.txt", false),
            "nested rule leaked upward"
        );

        // A cap of zero loads nothing rather than reading the whole tree.
        assert!(IgnoreSet::load(tmp.path(), 0).is_empty());
    }

    #[test]
    fn loading_never_descends_into_always_skipped_directories() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ignored = tmp.path().join("target");
        std::fs::create_dir_all(&ignored).expect("target");
        std::fs::write(ignored.join(".gitignore"), "*.o\n").expect("ignore");
        let s = IgnoreSet::load(tmp.path(), 64);
        assert!(
            s.is_empty(),
            "a rule inside target/ must never be read: it cannot apply to real content"
        );
    }

    #[test]
    fn a_directory_negation_restores_everything_below_it() {
        // Git resolves per component and never descends into an ignored
        // directory, so re-including the directory re-includes its children.
        // Testing only the full path gets this wrong in both directions.
        let s = set("/.github/\n!/.github/\n");
        assert!(
            !s.is_ignored(".github", true),
            "the directory is re-included"
        );
        assert!(
            !s.is_ignored(".github/workflows/ci.yml", false),
            "and so is everything under it: the whole point of the escape hatch"
        );

        // Without the negation, the directory and all its contents go.
        let plain = set(".github/\n");
        assert!(plain.is_ignored(".github", true));
        assert!(plain.is_ignored(".github/workflows/ci.yml", false));
    }

    #[test]
    fn a_directory_pattern_matches_that_directory_and_not_a_prefix_of_a_name() {
        // `dir/` must not ignore `directory/`, which a careless prefix test does.
        let s = set("dir/\n");
        assert!(s.is_ignored("dir", true));
        assert!(s.is_ignored("dir/inside.txt", false));
        assert!(!s.is_ignored("directory", true));
        assert!(!s.is_ignored("directory/file.txt", false));
    }

    #[test]
    fn an_ignored_parent_decides_for_its_children() {
        // `vendor/**` and a nested negation: the parent is ignored, so the walk
        // never sees the child and the negation cannot rescue it — which is git's
        // behavior, not an omission.
        let s = set("vendor/\n");
        assert!(s.is_ignored("vendor/deep/keep.rs", false));
    }

    #[test]
    fn a_glob_that_could_backtrack_forever_terminates() {
        // A tool that a `.gitignore` can hang is a tool that will be hung.
        assert!(!glob_match("****a", &"b".repeat(200)));
        assert!(glob_match("**", &"b".repeat(200)));
        assert!(glob_match("*", "src"));
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("a*b*c", "axxbyy"));
    }
}
