//! A refused token is answered, logged, and leaves no token material behind (§6.2, §11.7,
//! SRV-07, SRV-08, FM-10).
//!
//! §6.2: a failed `hello` is answered `auth_failed`, the connection closes, and "the app logs
//! the attempt (no token material)". `tests/channel_listener.rs` proves the first two on the
//! wire. The third is about the log, and the log is written by the production subscriber
//! `crash::install` puts in place once per process — so the refusal happens in a child
//! process that installs exactly that (`child.rs`), and this test reads the child's stderr:
//! every field of every event, unfiltered, which is more than a crash file keeps. It looks
//! for both tokens — the installation's and the one the stranger offered — whole, and for
//! their first and last eight characters: "no token material, not even a length or a
//! prefix" is the listener's own comment, and this is where it is held to it.

use std::ffi::OsStr;
use std::path::PathBuf;

use serde_json::json;

use handoff_app_lib::channel::listener::{listen, ListenerConfig};
use handoff_app_lib::channel::token::Token;

use crate::fake_server::{hello_of, with_token, FakeServer};
use crate::support::{private_endpoint, TempDir};
use crate::{child, forbidden, report};

/// This test's own path, which is how the child is selected.
const TEST: &str = "token::a_refused_token_is_answered_logged_and_leaves_no_token_material";

/// What the parent hands the child.
const WORKSPACE: &str = "BATON_SECURITY_DIR";
const INSTALLATION_TOKEN: &str = "BATON_SECURITY_TOKEN";
const STRANGER_TOKEN: &str = "BATON_SECURITY_STRANGER_TOKEN";

/// The message the listener logs when it refuses a token (`channel::listener`). It is the
/// control: an output that does not hold it is an output the refusal never reached, and no
/// token found in it would prove nothing.
const REFUSAL: &str = "refused a channel connection presenting the wrong token";

/// How much of a token counts as material besides the whole: its first and its last this
/// many characters.
const FRAGMENT: usize = 8;

#[test]
fn a_refused_token_is_answered_logged_and_leaves_no_token_material() {
    if child::is_child() {
        refuse_a_stranger();
        return;
    }
    let dir = TempDir::new("token");
    let installation = Token::generate();
    let stranger = Token::generate();
    let output = child::run(
        TEST,
        &[
            (WORKSPACE, dir.path().as_os_str()),
            (INSTALLATION_TOKEN, OsStr::new(installation.as_str())),
            (STRANGER_TOKEN, OsStr::new(stranger.as_str())),
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    let logged = stderr.contains(REFUSAL);
    let material = material_of(&[&installation, &stranger]);
    let found = forbidden::leaks(&stderr, &material);
    report::record(
        "channel_token",
        json!({
            "status": report::status(logged && found.is_empty()),
            "answer": "auth_failed, then the connection closed",
            "refusal_logged": logged,
            "material_searched": material.len(),
            "material_found": found.len(),
        }),
    );
    assert!(
        logged,
        "the refusal was not logged, so the output searched below is not the one that \
         recorded it:\n{stderr}"
    );
    assert!(
        found.is_empty(),
        "the log of a refused connection carries token material:\n{}",
        found.join("\n")
    );
}

/// Both tokens whole, and the first and last [`FRAGMENT`] characters of each.
fn material_of(tokens: &[&Token]) -> Vec<String> {
    tokens
        .iter()
        .flat_map(|token| {
            let text = token.as_str();
            [
                text.to_owned(),
                text[..FRAGMENT].to_owned(),
                text[text.len() - FRAGMENT..].to_owned(),
            ]
        })
        .collect()
}

/// The child: the production subscriber, a listener, and a peer offering the wrong token.
fn refuse_a_stranger() {
    let variable =
        |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is set by the parent"));
    let dir = PathBuf::from(variable(WORKSPACE));
    let installation = Token::parse(&variable(INSTALLATION_TOKEN)).expect("the parent's token");
    let stranger = Token::parse(&variable(STRANGER_TOKEN)).expect("the stranger's token");

    // What `run()` installs first: the stderr subscriber, the crash ring and the panic hook.
    let _recent = handoff_app_lib::crash::install(dir.join("app-data"));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let endpoint = private_endpoint(&dir);
        let (handle, _events) = listen(ListenerConfig::new(endpoint.clone(), installation))
            .await
            .expect("the listener binds");

        let mut peer = FakeServer::connect(&endpoint).await;
        peer.send(with_token(&hello_of("f01-register"), &stranger))
            .await;
        let answer = peer.expect("the refusal").await;
        assert_eq!(answer["error"]["message"], json!("auth_failed"), "{answer}");
        peer.expect_closed().await;

        handle.shutdown("the security suite is done").await;
    });
    println!("{}", child::finished(TEST));
}
