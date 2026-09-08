//! Installation adapters (§7.15, INST-01..08, SRV-07, SRV-25).
//!
//! One `InstallAdapter` per agent — detect, plan, apply, verify, uninstall — over that
//! agent's own configuration files. The plan is what the consent screen renders, so it has
//! to name the exact file and the exact change; `apply` writes a backup, edits the JSON
//! keeping unrelated keys, and re-reads to verify.
//!
//! Claude Code writes three modifications (T-026, Option B): the MCP entry with the fixed
//! launcher path and the per-server `timeout`, and the Stop and SubagentStop hooks.
//! `env.MCP_TOOL_TIMEOUT` is never written and never restored.
// TASK: T-039 (Claude Code adapter and its golden files), T-067 and T-074 (the others)
