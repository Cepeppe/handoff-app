//! The process table, and the ancestor chain completed from it (§5.8, DD-22, SRV-17, SRV-19).
//!
//! A peer states as much of its own chain as its platform gives cheaply: on macOS one `ps`
//! spawn, on Windows nothing at all, because `Get-CimInstance` costs hundreds of
//! milliseconds against a hook budget of under two seconds. DD-22 puts the rest of the work
//! here instead: the app has a native cross-platform process table and **always** resolves
//! the chain of the connected peer itself, then uses the union of the two.
//!
//! # Why a trait
//!
//! [`complete_chain`] is the part that has to be right — the cap, the cycle, the order, the
//! union — and it is the part a real process table cannot test. A test that walked the
//! machine's own processes would assert about whatever happened to be running. So the walk
//! reads a [`ProcessTable`], [`SystemProcessTable`] is the `sysinfo` one the app uses, and
//! [`SyntheticProcessTable`] is a tree written out in a test.
//!
//! # Parity with what the server sends
//!
//! `handoff-mcp` builds `identity.ancestors` with the same three rules and the same cap
//! (`src/platform/ancestors.ts`): nearest parent first, the peer's own pid is **not** in its
//! own chain (it travels as `identity.pid`), and a name is the last segment of the command,
//! never a full path. The union below keeps all three, so a chain the app completed and a
//! chain the peer sent are the same kind of list and can be merged by pid.

use std::collections::{HashMap, HashSet};

use crate::format::channel::{AncestorProcess, Identity};

/// How many generations are walked.
///
/// The same cap the server uses. A process tree is a few levels deep; the cap is there so
/// that a table with a cycle in it — which a snapshot taken during a reparenting can
/// produce — cannot turn into an endless walk. The cycle check below makes it belt and
/// braces, because a table read from a live machine is not a tree we control.
pub const MAX_ANCESTOR_DEPTH: usize = 32;

/// The two questions the ancestor walk asks about a process.
///
/// Deliberately the whole interface: a snapshot of the machine answers them from a map, and
/// a test answers them from three lines of a written-out tree.
pub trait ProcessTable {
    /// The parent of `pid`, or nothing when the table does not know the process or the
    /// process has no parent (it is the root, or its parent is outside what we can see).
    fn parent_of(&self, pid: u32) -> Option<u32>;

    /// The executable name of `pid`, without its path, or nothing when the table does not
    /// know the process.
    fn name_of(&self, pid: u32) -> Option<String>;
}

/// The completed ancestor chain of the peer that sent `identity` (DD-22).
///
/// The union of two lists: what the app walks in its own process table, which is the
/// authoritative one while the peer is alive, and what the peer sent, which is all there is
/// once it has gone. Nearest parent first in both halves, the peer's own pid in neither.
///
/// Three cases, one rule:
///
/// - the peer is alive and the table knows it — the walk produces the chain and the sent
///   list adds nothing (it is a subset, or a generation the table lost between the two
///   snapshots, which is appended after the walk);
/// - the peer has already gone — `parent_of` knows nothing, so the walk starts from the
///   `ppid` the peer stated, and the rest of the sent list is appended in its own order;
/// - nobody knows anything — the chain is empty, which costs a little precision in the
///   binding of §7.5 and nothing else. DD-22 is explicit that none of this is load-bearing.
#[must_use]
pub fn complete_chain(table: &dyn ProcessTable, identity: &Identity) -> Vec<AncestorProcess> {
    let sent: HashMap<u32, &str> = identity
        .ancestors
        .iter()
        .map(|ancestor| (ancestor.pid, ancestor.name.as_str()))
        .collect();

    let mut chain: Vec<AncestorProcess> = Vec::new();
    // The peer is never in its own chain, and starting `seen` with it is also what stops a
    // table that reports a process as its own ancestor.
    let mut seen: HashSet<u32> = HashSet::from([identity.pid]);
    // The peer's stated `ppid` is the fallback: the table has already forgotten a peer that
    // exited between its `hello` and this walk, and its parent is still worth having.
    let mut current = table.parent_of(identity.pid).unwrap_or(identity.ppid);

    while current > 0 && chain.len() < MAX_ANCESTOR_DEPTH && seen.insert(current) {
        // The table first, the peer's own list second: on macOS the two were taken at
        // different instants and the app's is the fresher one. An empty name is possible
        // and harmless — the binding of §7.5 intersects pids, and a name is for the log.
        let name = table
            .name_of(current)
            .or_else(|| sent.get(&current).map(|name| (*name).to_owned()))
            .unwrap_or_default();
        chain.push(AncestorProcess { pid: current, name });
        current = table.parent_of(current).unwrap_or(0);
    }

    for ancestor in &identity.ancestors {
        if chain.len() >= MAX_ANCESTOR_DEPTH {
            break;
        }
        if seen.insert(ancestor.pid) {
            chain.push(ancestor.clone());
        }
    }

    chain
}

/// A snapshot of this machine's processes, taken through `sysinfo`.
///
/// A snapshot rather than a live handle on purpose: the walk of one peer must see one
/// consistent tree, and a table refreshed between two questions can answer them about two
/// different machines. Taking it costs one process listing per registration, which is what
/// DD-22 budgets for.
#[derive(Debug, Default)]
pub struct SystemProcessTable {
    entries: HashMap<u32, Entry>,
}

#[derive(Debug)]
struct Entry {
    parent: Option<u32>,
    name: String,
}

impl SystemProcessTable {
    /// Lists the machine's processes now.
    ///
    /// Only the pid, the parent and the name are asked for (`ProcessRefreshKind::nothing`
    /// still returns those three); command lines, environments and working directories are
    /// deliberately not read, and `without_tasks` keeps the listing from descending into
    /// every thread of every process.
    #[must_use]
    pub fn snapshot() -> Self {
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing().without_tasks(),
        );
        let entries = system
            .processes()
            .iter()
            .map(|(pid, process)| {
                (
                    pid.as_u32(),
                    Entry {
                        parent: process.parent().map(sysinfo::Pid::as_u32),
                        name: process.name().to_string_lossy().into_owned(),
                    },
                )
            })
            .collect();
        Self { entries }
    }
}

impl ProcessTable for SystemProcessTable {
    fn parent_of(&self, pid: u32) -> Option<u32> {
        self.entries.get(&pid)?.parent
    }

    fn name_of(&self, pid: u32) -> Option<String> {
        Some(self.entries.get(&pid)?.name.clone())
    }
}

/// A process tree written out by hand, for tests and for the fakes of later tasks.
///
/// Not `cfg(test)`: the registry's own tests use it, and so will the store's (T-034) and the
/// hook's (T-035), which live in other test binaries.
#[derive(Debug, Default, Clone)]
pub struct SyntheticProcessTable {
    entries: HashMap<u32, (Option<u32>, String)>,
}

impl SyntheticProcessTable {
    /// An empty table: every question is answered with nothing, which is what the app sees
    /// when a peer has already exited.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one process. `parent` is `None` for a root.
    #[must_use]
    pub fn with(mut self, pid: u32, parent: Option<u32>, name: &str) -> Self {
        self.entries.insert(pid, (parent, name.to_owned()));
        self
    }

    /// Adds a chain `pids[0]` → `pids[1]` → … where each is the child of the next, naming
    /// them after their position. The shape most tests need, in one line.
    #[must_use]
    pub fn with_line(mut self, pids: &[u32]) -> Self {
        for (index, pid) in pids.iter().enumerate() {
            let parent = pids.get(index + 1).copied();
            self.entries
                .insert(*pid, (parent, format!("process-{pid}")));
        }
        self
    }
}

impl ProcessTable for SyntheticProcessTable {
    fn parent_of(&self, pid: u32) -> Option<u32> {
        self.entries.get(&pid)?.0
    }

    fn name_of(&self, pid: u32) -> Option<String> {
        Some(self.entries.get(&pid)?.1.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(pid: u32, ppid: u32, ancestors: &[(u32, &str)]) -> Identity {
        Identity {
            pid,
            ppid,
            ancestors: ancestors
                .iter()
                .map(|(pid, name)| AncestorProcess {
                    pid: *pid,
                    name: (*name).to_owned(),
                })
                .collect(),
            cwd: "C:\\projects\\baton".to_owned(),
            project_dir: Some("C:\\projects\\baton".to_owned()),
        }
    }

    fn pids(chain: &[AncestorProcess]) -> Vec<u32> {
        chain.iter().map(|entry| entry.pid).collect()
    }

    #[test]
    fn the_app_walks_the_whole_chain_a_windows_peer_could_not_send() {
        // The Windows case of DD-22: the peer sends `pid` and `ppid` and no chain at all.
        let table = SyntheticProcessTable::new().with_line(&[500, 400, 300, 200, 100]);
        let chain = complete_chain(&table, &identity(500, 400, &[]));

        assert_eq!(pids(&chain), [400, 300, 200, 100]);
        assert_eq!(chain[0].name, "process-400");
    }

    #[test]
    fn the_peers_own_pid_is_never_in_its_own_chain() {
        let table = SyntheticProcessTable::new().with_line(&[500, 400, 300]);
        let chain = complete_chain(&table, &identity(500, 400, &[]));
        assert!(!pids(&chain).contains(&500));
    }

    #[test]
    fn a_generation_only_the_peer_saw_is_added_after_the_walk() {
        // The macOS case: the peer's `ps` snapshot and the app's listing were taken at
        // different instants, so each can hold a generation the other lost.
        let table = SyntheticProcessTable::new().with_line(&[500, 400]);
        let chain = complete_chain(
            &table,
            &identity(500, 400, &[(400, "zsh"), (300, "iTerm2")]),
        );

        assert_eq!(pids(&chain), [400, 300]);
        assert_eq!(chain[0].name, "process-400", "the table wins on a name");
        assert_eq!(chain[1].name, "iTerm2", "and the peer fills what it lacks");
    }

    #[test]
    fn a_dead_peer_leaves_its_ppid_and_its_own_list_behind() {
        // The app's table has already forgotten the peer, so the walk has nothing to start
        // from but the `ppid` the peer stated, and nothing to continue with but its list.
        let chain = complete_chain(
            &SyntheticProcessTable::new(),
            &identity(500, 400, &[(400, "zsh"), (300, "iTerm2")]),
        );
        assert_eq!(pids(&chain), [400, 300]);
        assert_eq!(chain[0].name, "zsh");
    }

    #[test]
    fn nothing_known_at_all_is_an_empty_chain_and_not_a_failure() {
        // DD-22: an empty list costs precision in the binding of §7.5 and nothing else.
        let chain = complete_chain(&SyntheticProcessTable::new(), &identity(500, 0, &[]));
        assert!(chain.is_empty());
    }

    #[test]
    fn a_cycle_in_the_table_stops_the_walk() {
        let table = SyntheticProcessTable::new()
            .with(500, Some(400), "child")
            .with(400, Some(300), "parent")
            .with(300, Some(400), "grandparent-pointing-back");
        let chain = complete_chain(&table, &identity(500, 400, &[]));
        assert_eq!(pids(&chain), [400, 300]);
    }

    #[test]
    fn a_process_that_is_its_own_parent_stops_the_walk() {
        let table = SyntheticProcessTable::new().with(500, Some(500), "itself");
        assert!(complete_chain(&table, &identity(500, 500, &[])).is_empty());
    }

    #[test]
    fn the_walk_and_the_union_both_stop_at_the_depth_cap() {
        let line: Vec<u32> = (0..60).map(|generation| 1000 + generation).collect();
        let table = SyntheticProcessTable::new().with_line(&line);
        let walked = complete_chain(&table, &identity(1000, 1001, &[]));
        assert_eq!(walked.len(), MAX_ANCESTOR_DEPTH);

        let sent: Vec<(u32, &str)> = line[1..].iter().map(|pid| (*pid, "sent")).collect();
        let merged = complete_chain(&SyntheticProcessTable::new(), &identity(1000, 1001, &sent));
        assert_eq!(merged.len(), MAX_ANCESTOR_DEPTH);
    }

    #[test]
    fn the_machines_own_table_answers_about_this_process() {
        // The one assertion a real snapshot can make without depending on what is running:
        // this process is in it, and it has a name.
        let table = SystemProcessTable::snapshot();
        let me = std::process::id();
        assert!(
            table.name_of(me).is_some_and(|name| !name.is_empty()),
            "the process table does not know the process asking it"
        );
    }
}
