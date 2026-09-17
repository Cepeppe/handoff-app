//! The single network egress point (§7.13, NET-01, NET-02, UPD-01, UPD-02, DD-32, PRIN-05).
//!
//! [`get`] is the only function in this application that opens a network connection, and it
//! records `{ at, domain, bytes_sent, purpose }` in `network_events` **before** it sends
//! anything. That ordering is the whole reason the Network settings page is a record and not
//! a claim: a connection cannot happen without having been written down first, so a row that
//! is missing would have to be a connection that never left.
//!
//! **This build makes zero connections.** The update check is the only intended caller and it
//! is deferred (implementation decision 8, T-078), so nothing calls
//! [`get`] and the `net` feature that carries the client is off by default: the binary a
//! person installs links no HTTP stack at all, and [`get`] answers [`EgressError::NoClient`]
//! if anybody asks it to. NET-02's firewall test therefore expects zero domains until T-078.
//!
//! # The boundary, and the four things that hold it
//!
//! - **This file.** `clippy.toml` disallows `reqwest`, `hyper` and the raw TCP types, and the
//!   `#![allow]` below lifts that rule here and nowhere else. `cargo lint` is a CI gate.
//! - **The dependency graph.** `deny.toml` allows `reqwest` only as a direct dependency of
//!   this crate — that is, only through the `net` feature — and `hyper` only because that one
//!   client brought it. Anything else that tried to reach the network would be refused with
//!   the path that reached it.
//! - **The sources.** `tests/egress_boundary.rs` reads every `.rs` under `src/` and fails if
//!   any file but this one names one of those types.
//! - **The webview.** The CSP is `default-src 'self'` with no `connect-src`, asserted by the
//!   same test, so the frontend cannot open a connection either.
//!
//! # What this module refuses on its own
//!
//! An address that is not `https://`, one carrying credentials, and a redirect that would
//! leave the domain that was recorded. The last one is not a hardening flourish: the row was
//! written for a domain, and a redirect to another one would make the Network page describe a
//! connection that did not happen while hiding the one that did.
//!
//! The connection to the log is passed in rather than held here. §7.13 writes the signature
//! as `get(url) -> Result<Response>`, which is behaviour and not application code (§1.3); the
//! caller owns a [`Db`] already, and a module with a fourth connection of its own to
//! `handoff.sqlite` would be one more writer for two columns the store can hand over.
// The one place an HTTP client may be named (§7.13, DD-32). The lint that keeps every other
// module of this crate away from one is lifted here, for this file only, and the module
// comment above says what replaces it: three checks that no `#[allow]` can switch off.
#![allow(clippy::disallowed_types)]

use crate::log::network_events::{self, NetworkEventRow};
use crate::log::{Db, Timestamp};

/// How long one request may take, start to finish (§7.13: "10 s timeout").
///
/// A bound on one operation and not a wake-up: nothing arms it unless a request is in
/// flight, which is why `tests/timers.rs` has nothing to declare for this module.
pub const EGRESS_TIMEOUT_MS: u64 = 10_000;

/// How much of an answer is read before the rest is dropped.
///
/// UPD-01's response is four short fields (`latest`, `notes`, `download_url`), so this is
/// three orders of magnitude of headroom. It exists because a reply is written by whoever
/// answers the address, and an application that reads an unbounded body from the network has
/// handed them its memory.
pub const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

/// How many same-domain redirects are followed before the attempt is given up.
pub const MAX_REDIRECTS: usize = 3;

/// What this application calls itself when it asks something of the network.
///
/// The version is deliberately absent: UPD-01 sends it once, in the query string, where the
/// user can read it in the URL the Network page shows. A client's own default would name the
/// HTTP library and its version instead, which says more about this machine than the check
/// needs to.
pub const USER_AGENT: &str = "Baton";

/// The `purpose` of the one connection the design foresees (§7.13, F-14).
///
/// A stable key rather than a sentence: the row outlives the language the window was in when
/// it was written, so the Network page turns this into a sentence when it draws it.
pub const PURPOSE_UPDATE_CHECK: &str = "update-check";

/// Why a connection did not happen, or did not finish.
///
/// Every one of them is silent for the user in this build (FM-29: an unreachable endpoint or
/// a disabled check is a log line and nothing else), so the texts are for the log.
#[derive(Debug, thiserror::Error)]
pub enum EgressError {
    /// The address is not one this module will reach.
    #[error("{0}")]
    Address(String),
    /// The connection could not be written down, so it was not made (NET-01).
    #[error("the connection could not be recorded, so it was not made: {0}")]
    NotRecorded(String),
    /// The request was made and did not come back.
    #[error("the request failed: {0}")]
    Failed(String),
    /// This build carries no client, because the `net` feature is off (implementation decision 8).
    #[error("this build has no network client")]
    NoClient,
}

/// What came back, as much of it as [`MAX_RESPONSE_BYTES`] allowed.
///
/// Deliberately ours and not the client's own type: a `reqwest::Response` in this signature
/// would put a disallowed type in the face of every caller, and the boundary of §7.13 is
/// exactly that no other module names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The HTTP status.
    pub status: u16,
    /// The body, decoded as UTF-8 with the invalid sequences replaced.
    pub body: String,
}

/// A connection that has been recorded and not yet made.
///
/// It is the return value of [`record_before_sending`] so that the ordering NET-01 rests on
/// is a shape and not a comment: there is no way to reach the sending half without holding
/// the proof that the row was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// The address, as it will be asked for.
    pub url: String,
    /// The host that was recorded, lower-cased and without its port.
    pub domain: String,
    /// The size of the request line and the headers this asks the client to send.
    pub bytes_sent: i64,
}

/// The host of an `https://` address, lower-cased and without its port.
///
/// # Errors
///
/// [`EgressError::Address`] for anything this module will not reach: another scheme, an
/// empty host, credentials in the authority, or whitespace.
pub fn domain_of(url: &str) -> Result<String, EgressError> {
    const SCHEME: &str = "https://";
    let refuse = |why: &str| EgressError::Address(format!("{why}: {url}"));

    if url.chars().any(char::is_whitespace) {
        return Err(refuse("an address may not contain whitespace"));
    }
    let rest = url
        .strip_prefix(SCHEME)
        .ok_or_else(|| refuse("only https:// addresses are reachable from here"))?;
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .expect("split always yields a first part");
    if authority.contains('@') {
        return Err(refuse("an address may not carry credentials"));
    }
    let host = authority
        .split(':')
        .next()
        .expect("split always yields a first part");
    if host.is_empty() {
        return Err(refuse("the address names no host"));
    }
    Ok(host.to_ascii_lowercase())
}

/// The request line and the headers this module asks its client to send.
///
/// What NET-01 shows the user is how much about them left, so the count is of the request we
/// compose — the address with its query, and the three headers named here — and not of the
/// bytes TLS put on the wire, which no caller could interpret and which include a handshake
/// that says nothing about the user. The frame is HTTP/1.1 because that is the shape a person
/// reading the Network page can check against the URL beside it.
fn request_head(url: &str, domain: &str) -> String {
    let target = url
        .strip_prefix("https://")
        .and_then(|rest| rest.find(['/', '?', '#']).map(|at| &rest[at..]))
        .unwrap_or("/");
    format!(
        "GET {target} HTTP/1.1\r\nhost: {domain}\r\nuser-agent: {USER_AGENT}\r\naccept: */*\r\n\r\n"
    )
}

/// Writes the row NET-01 promises, and answers what may then be sent.
///
/// The order is the point: this runs to completion before any socket is opened, and a write
/// that fails stops the connection instead of losing it (a connection nobody could see is
/// worse than an update check that did not happen — PRIN-05, FM-29).
///
/// # Errors
///
/// [`EgressError::Address`] when the address is not reachable from here, and
/// [`EgressError::NotRecorded`] when the row could not be written.
pub fn record_before_sending(db: &Db, url: &str, purpose: &str) -> Result<Outgoing, EgressError> {
    let domain = domain_of(url)?;
    let bytes_sent = i64::try_from(request_head(url, &domain).len()).unwrap_or(i64::MAX);
    network_events::append(
        db,
        &NetworkEventRow {
            id: 0,
            at: Timestamp::now(),
            domain: domain.clone(),
            bytes_sent,
            purpose: purpose.to_owned(),
        },
    )
    .map_err(|error| EgressError::NotRecorded(error.to_string()))?;
    Ok(Outgoing {
        url: url.to_owned(),
        domain,
        bytes_sent,
    })
}

/// The **only** function of this application that opens a network connection (§7.13).
///
/// It records the connection first ([`record_before_sending`]), then asks for `url` over
/// HTTPS with no cookies, no referrer, a [`EGRESS_TIMEOUT_MS`] bound and no redirect that
/// would leave the recorded domain.
///
/// # Errors
///
/// [`EgressError::NoClient`] in a build without the `net` feature — this one, where nothing
/// is recorded either, because nothing was sent. Otherwise the errors of
/// [`record_before_sending`], and [`EgressError::Failed`] when the request does not come back.
#[cfg(feature = "net")]
pub fn get(db: &Db, url: &str, purpose: &str) -> Result<Response, EgressError> {
    use std::io::Read as _;
    use std::time::Duration;

    let outgoing = record_before_sending(db, url, purpose)?;
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_millis(EGRESS_TIMEOUT_MS))
        // No cookie store is compiled in at all (the `cookies` feature is off), so there is
        // nothing to carry between two launches even by accident; `referer(false)` is the
        // same rule for the one header a redirect would otherwise add.
        .referer(false)
        .https_only(true)
        .redirect(same_domain_only())
        .build()
        .map_err(|error| EgressError::Failed(error.to_string()))?;

    let response = client
        .get(&outgoing.url)
        .send()
        .map_err(|error| EgressError::Failed(error.to_string()))?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES)
        .read_to_end(&mut body)
        .map_err(|error| EgressError::Failed(error.to_string()))?;
    Ok(Response {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// The build with no client: nothing is sent, so nothing is recorded (implementation decision 8).
#[cfg(not(feature = "net"))]
pub fn get(_db: &Db, _url: &str, _purpose: &str) -> Result<Response, EgressError> {
    Err(EgressError::NoClient)
}

/// A redirect is followed only while it stays on the domain that was recorded.
///
/// Off the domain the answer is an error rather than the 3xx itself: a caller handed a
/// redirect it did not ask for would have to know to look, and the Network page would be
/// naming a host that was not the one reached.
#[cfg(feature = "net")]
fn same_domain_only() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let previous = attempt.previous();
        let from = previous
            .last()
            .and_then(|url| url.host_str().map(str::to_owned));
        if previous.len() > MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match (from.as_deref(), attempt.url().host_str()) {
            (Some(from), Some(to)) if from.eq_ignore_ascii_case(to) => attempt.follow(),
            _ => attempt.error("a redirect off the recorded domain is refused (§7.13)"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Db {
        Db::open_in_memory().expect("a database")
    }

    #[test]
    fn the_domain_is_the_host_lower_cased_and_without_its_port() {
        assert_eq!(
            domain_of("https://Updates.Example.Test/v1/check?app=0.1.0&os=win32")
                .expect("a domain"),
            "updates.example.test"
        );
        assert_eq!(
            domain_of("https://updates.example.test:8443/v1/check").expect("a domain"),
            "updates.example.test"
        );
        assert_eq!(
            domain_of("https://updates.example.test").expect("a domain"),
            "updates.example.test"
        );
    }

    #[test]
    fn an_address_this_module_will_not_reach_is_refused_before_anything_is_recorded() {
        let db = database();
        for url in [
            "http://updates.example.test/v1/check",
            "ftp://updates.example.test/v1/check",
            "https://user:secret@updates.example.test/v1/check",
            "https:///v1/check",
            "https://updates.example.test/v1/check?note=two words",
            "/v1/check",
        ] {
            // `record_before_sending` and not `get`, so the case says the same thing in both
            // builds: without the `net` feature `get` refuses everything before it looks at
            // the address, and the rule being asserted here is about the address.
            let refused =
                record_before_sending(&db, url, PURPOSE_UPDATE_CHECK).expect_err("refused");
            assert!(
                matches!(refused, EgressError::Address(_)),
                "{url} was answered with {refused:?}"
            );
            assert!(
                network_events::list(&db).expect("the events").is_empty(),
                "{url} left a row behind although nothing was sent"
            );
        }
    }

    #[test]
    fn the_row_is_written_before_anything_could_have_been_sent() {
        // NET-01 rests on this order, and it is the one thing about egress that can be
        // asserted without a network: `record_before_sending` is the only way to reach the
        // sending half, and by the time it answers the row is already in the table.
        let db = database();
        let outgoing = record_before_sending(
            &db,
            "https://updates.example.test/v1/check?app=0.1.0&os=win32",
            PURPOSE_UPDATE_CHECK,
        )
        .expect("recorded");

        let rows = network_events::list(&db).expect("the events");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].domain, "updates.example.test");
        assert_eq!(rows[0].purpose, PURPOSE_UPDATE_CHECK);
        assert_eq!(rows[0].bytes_sent, outgoing.bytes_sent);
        assert!(outgoing.bytes_sent > 0);
    }

    #[test]
    fn what_is_counted_is_the_request_and_it_grows_with_the_address() {
        let short = request_head("https://a.test/v1", "a.test");
        let long = request_head("https://a.test/v1/check?app=0.1.0&os=win32", "a.test");
        assert!(short.starts_with("GET /v1 HTTP/1.1\r\n"));
        assert!(long.contains("?app=0.1.0&os=win32"));
        assert!(long.len() > short.len());
        // The three headers this module asks for, and no fourth one it forgot to count.
        assert_eq!(long.matches("\r\n").count(), 5);
        assert!(long.contains(&format!("user-agent: {USER_AGENT}")));
        // An address with no path still asks for one.
        assert!(request_head("https://a.test", "a.test").starts_with("GET / HTTP/1.1"));
    }

    #[cfg(not(feature = "net"))]
    #[test]
    fn a_build_without_the_client_cannot_connect_and_records_nothing() {
        // implementation decision 8 and NET-02: this is the build a person installs today. The address is
        // a perfectly good one, so the only reason there is no row is that there is no
        // client — which is what the firewall test observes from the outside.
        let db = database();
        let refused = get(
            &db,
            "https://updates.example.test/v1/check?app=0.1.0&os=win32",
            PURPOSE_UPDATE_CHECK,
        )
        .expect_err("this build has no client");
        assert!(matches!(refused, EgressError::NoClient));
        assert!(network_events::list(&db).expect("the events").is_empty());
    }

    #[cfg(feature = "net")]
    #[test]
    fn the_client_is_built_with_the_bounds_of_the_design() {
        // The builder is what carries the four rules of §7.13, and a `Client` cannot be
        // asked what it was built with. What is checkable is that it builds at all with
        // them — a rustls provider that failed to install, the usual way this breaks, fails
        // here rather than on the first real connection.
        use std::time::Duration;
        reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_millis(EGRESS_TIMEOUT_MS))
            .referer(false)
            .https_only(true)
            .redirect(same_domain_only())
            .build()
            .expect("the client of §7.13 builds");
    }
}
