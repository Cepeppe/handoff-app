//! The channel listener: the socket the server and the hook connect to (§7.3, §6).
//!
//! One task accepts connections; each connection gets a reader (NDJSON to JSON-RPC,
//! validated against `channel.v1.schema.json`), a writer, and a `Peer` record. `hello` is
//! answered within 2 s: the token is compared, the protocol version has to match exactly,
//! the ancestor chain is completed from the process table (DD-22), and the peer is handed
//! to `sessions::register` or to `hook::decide` depending on its role. Requests are
//! dispatched to the store on a single actor task, so every state mutation is serialised.
//!
//! Named pipe with a DACL on Windows, Unix socket with the pointer file of FM-12
//! elsewhere. The endpoint name is derived in `paths` and has to match, byte for byte,
//! what `handoff-mcp` derives on its side.
// TASK: T-031
