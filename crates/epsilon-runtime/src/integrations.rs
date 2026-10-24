//! Curated third-party integrations that ship as one switch.
//!
//! An integration here is not vendored code and not a second plugin system. It
//! is a **known-good configuration** for a component that is already reachable
//! through an existing seam (MCP, a skill, a shell command), with the version,
//! platform wrapper and deadline chosen once instead of left to each user.
//!
//! Why a module: the alternative is a documented `[[mcp_servers]]` snippet that
//! every user must retype, and a Windows user who retypes the Unix form gets a
//! spawn failure that looks like a harness bug. The values live here so the
//! documentation, the flag and the tests read the same constants.

/// REA (Reverse Engineer Anything): <https://github.com/morluto/rea>.
///
/// MIT-licensed, by morluto. Epsilon invokes it as an external MCP server and
/// does not vendor its source. Pinned to an exact version for the same reason
/// every dependency is: an unpinned `npx latest` changes under a session that
/// was verified against a different one.
pub const REA_PACKAGE: &str = "rea-agents@6.7.0";

/// The server name REA's tools register under (`mcp__rea__*`).
pub const REA_SERVER_NAME: &str = "rea";

/// Per-call deadline for REA.
///
/// Ghidra/Hopper startup and real analysis legitimately outlast the 120 s
/// default for MCP tools. Fifteen minutes is a bound, not a promise; a wedged
/// analysis still ends.
pub const REA_TIMEOUT_SECS: u64 = 900;

/// The stdio MCP entry that launches REA on this platform.
///
/// REA's own installation guide documents the Windows wrapper (`cmd.exe /d /c
/// npx … mcp`) because a `.cmd` shim is not a directly spawnable executable from
/// every stdio client. Encoding it here is the whole point of the integration.
pub fn rea_mcp_server() -> epsilon_mcp::McpServerConfig {
    #[cfg(windows)]
    let (command, args) = (
        "cmd.exe".to_string(),
        vec![
            "/d".to_string(),
            "/c".to_string(),
            "npx".to_string(),
            "-y".to_string(),
            REA_PACKAGE.to_string(),
            "mcp".to_string(),
        ],
    );
    #[cfg(not(windows))]
    let (command, args) = (
        "npx".to_string(),
        vec!["-y".to_string(), REA_PACKAGE.to_string(), "mcp".to_string()],
    );
    epsilon_mcp::McpServerConfig {
        name: REA_SERVER_NAME.to_string(),
        command,
        args,
        timeout_secs: REA_TIMEOUT_SECS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rea_is_one_pinned_stdio_server_with_a_generous_deadline() {
        let cfg = rea_mcp_server();
        assert_eq!(cfg.name, "rea");
        assert!(cfg.timeout_secs >= 300, "analysis outlasts the default");
        assert!(
            cfg.args.iter().any(|a| a == REA_PACKAGE),
            "the package is pinned in argv: {:?}",
            cfg.args
        );
        assert_eq!(cfg.args.last().map(String::as_str), Some("mcp"));
        #[cfg(windows)]
        {
            assert_eq!(cfg.command, "cmd.exe");
            assert!(cfg.args.iter().any(|a| a == "npx"));
        }
        #[cfg(not(windows))]
        assert_eq!(cfg.command, "npx");
    }
}
