//! Agent Skills spec support (ADR-015): SKILL.md directories with YAML-lite frontmatter,
//! progressive disclosure — catalog (name + description) in the prompt, body on demand.

use std::path::PathBuf;

use epsilon_contracts::tool::ToolFailure;

/// Name rules per the spec (pi parity): <=64 chars, lowercase a-z/0-9/hyphen,
/// no leading/trailing/consecutive hyphens.
const MAX_NAME: usize = 64;
/// Description limit per the spec.
const MAX_DESC: usize = 1024;

/// One discovered skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Validated skill name (directory name unless frontmatter overrides).
    pub name: String,
    /// One-line purpose shown in the catalog.
    pub description: String,
    /// Directory containing SKILL.md.
    pub dir: PathBuf,
    /// When true, only slash-command style invocation may load it (not model choice).
    pub disable_model_invocation: bool,
}

impl Skill {
    /// Full SKILL.md contents minus the frontmatter block.
    pub fn load_body(&self) -> Result<String, ToolFailure> {
        let raw = std::fs::read_to_string(self.dir.join("SKILL.md"))
            .map_err(|e| ToolFailure::msg(format!("cannot read skill '{}': {e}", self.name)))?;
        Ok(strip_frontmatter(&raw))
    }
}

/// Parse `---` delimited frontmatter into key/value pairs (flat strings only).
/// Returns the pairs and the body AFTER the closing marker.
fn parse_frontmatter(raw: &str) -> (Vec<(String, String)>, String) {
    if raw.lines().next().map(str::trim) != Some("---") {
        return (Vec::new(), raw.to_string());
    }
    let mut pairs = Vec::new();
    let mut consumed = 1usize; // opening ---
    let mut lines = raw.lines();
    lines.next();
    for line in lines.by_ref() {
        consumed += 1;
        if line.trim() == "---" {
            let body: Vec<&str> = raw.lines().skip(consumed).collect();
            return (pairs, body.join("\n"));
        }
        if let Some((k, v)) = line.split_once(':') {
            pairs.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    (Vec::new(), raw.to_string()) // unterminated frontmatter: treat whole file as body
}

fn strip_frontmatter(raw: &str) -> String {
    parse_frontmatter(raw).1
}

/// Discover skills across root directories (each scanned one level deep for `SKILL.md`).
///
/// Later roots shadow earlier ones by name (project overrides global), mirroring pi.
pub fn discover(roots: &[PathBuf]) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for e in entries.filter_map(Result::ok) {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let skill_file = dir.join("SKILL.md");
            let Ok(raw) = std::fs::read_to_string(&skill_file) else {
                continue;
            };
            let (fm, _) = parse_frontmatter(&raw);
            let get = |k: &str| fm.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());

            let dir_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let name = get("name").unwrap_or(dir_name);
            if !valid_name(&name) {
                continue;
            }
            let description = get("description").unwrap_or_default();
            if description.is_empty() || description.len() > MAX_DESC {
                continue;
            }
            let skill = Skill {
                name,
                description,
                dir,
                disable_model_invocation: get("disable-model-invocation")
                    .map(|v| v == "true")
                    .unwrap_or(false),
            };
            // Shadowing: replace earlier-root skill with the same name.
            if let Some(slot) = out.iter_mut().find(|s| s.name == skill.name) {
                *slot = skill;
            } else {
                out.push(skill);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Catalog text for the prompt CONTEXT tier: one line per skill (progressive disclosure —
/// bodies cost nothing until invoked).
pub fn catalog(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = String::from("# Skills\nLoad a skill's full instructions before following them.\n");
    for sk in skills {
        s.push_str(&format!("- {}: {}\n", sk.name, sk.description));
    }
    s
}

fn valid_name(name: &str) -> bool {
    name.len() <= MAX_NAME
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_skill(root: &Path, dir: &str, fm: &str, body: &str) {
        let d = root.join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {dir}\ndescription: {fm}\n---\n{body}"),
        )
        .unwrap();
    }

    #[test]
    fn discovers_validates_and_shadows_by_priority_root_order() {
        let global = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();

        write_skill(global.path(), "commit-craft", "Craft commits.", "body-v1");
        write_skill(
            project.path(),
            "commit-craft",
            "Project override.",
            "body-v2",
        );
        write_skill(global.path(), "Bad_Name", "invalid name chars", "x");
        // missing description -> invalid
        let d = global.path().join("no-desc");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), "---\nname: no-desc\n---\nbody").unwrap();

        let skills = discover(&[global.path().to_path_buf(), project.path().to_path_buf()]);
        assert_eq!(
            skills.len(),
            1,
            "invalid skills skipped; same-name shadowed"
        );
        assert_eq!(skills[0].name, "commit-craft");
        assert_eq!(skills[0].description, "Project override.");
        assert_eq!(skills[0].load_body().unwrap().trim(), "body-v2");
    }

    #[test]
    fn catalog_lists_name_and_description_only() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "test-first", "Reproduce then fix.", "long body");
        let cat = catalog(&discover(&[dir.path().to_path_buf()]));
        assert!(cat.contains("- test-first: Reproduce then fix."));
        assert!(
            !cat.contains("long body"),
            "progressive disclosure: no bodies in catalog"
        );
    }

    #[test]
    fn frontmatter_edge_cases() {
        assert_eq!(
            strip_frontmatter("no frontmatter here"),
            "no frontmatter here"
        );
        let (fm, body) = parse_frontmatter("---\nname: x\n---\nBODY");
        assert_eq!(fm.len(), 1);
        assert_eq!(body.trim(), "BODY");
    }
}
