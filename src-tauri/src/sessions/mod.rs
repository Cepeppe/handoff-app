//! The session registry (§7.5, SRV-17..20, DD-22).
//!
//! One record per connected server: connection, PID chain, working directory, the resolved
//! capability row, and the `session_id` bound to it once an agent claims it. The ancestor
//! chain is what lets a Stop hook find the session it belongs to, since the hook is not a
//! direct child of the agent (measured in T-023, A-11).
// TASK: T-032
