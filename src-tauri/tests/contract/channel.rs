//! The golden channel sequences (§6, `protocol/channel/`, §11.2 "Channel codec").
//!
//! `fixtures/channel/*.jsonl` holds one file per flow, each line
//! `{"dir": "→"|"←", "msg": {...}}`. Every message has to validate against
//! `channel.v1.schema.json` and to round-trip through the types of
//! [`handoff_app_lib::format::channel`], which is what the listener of T-031 will read and
//! write. The server replays the same goldens through its own codec, so a shape only one
//! side can carry fails on one side and is found.
//!
//! The suite also checks that the discrimination is unambiguous: a line is a request, a
//! notification, a response or an error response, and never two of them.

use std::collections::BTreeSet;

use handoff_app_lib::format::channel::{
    ChannelErrorCode, ChannelMessage, NotificationBody, RequestBody,
};
use handoff_app_lib::format::schema::{validate, Document};
use serde_json::Value;

use crate::support::{fixture_files, name_of, read_to_string};

/// One line of a golden: the direction, and the message.
struct Line {
    file: String,
    number: usize,
    direction: String,
    message: Value,
}

fn goldens() -> Vec<Line> {
    let mut lines = Vec::new();
    for fixture in fixture_files("fixtures/channel", ".jsonl") {
        let file = name_of(&fixture);
        for (index, raw) in read_to_string(&fixture).lines().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            let entry: Value = serde_json::from_str(raw)
                .unwrap_or_else(|error| panic!("{file}:{} is not JSON: {error}", index + 1));
            lines.push(Line {
                file: file.clone(),
                number: index + 1,
                direction: entry["dir"].as_str().expect("a direction").to_owned(),
                message: entry["msg"].clone(),
            });
        }
    }
    assert!(
        lines.len() >= 60,
        "the golden set shrank to {}",
        lines.len()
    );
    lines
}

#[test]
fn every_golden_line_validates_against_the_channel_schema() {
    for line in goldens() {
        validate(Document::ChannelMessage, &line.message).unwrap_or_else(|problems| {
            panic!("{}:{} was refused: {problems:?}", line.file, line.number)
        });
    }
}

#[test]
fn every_golden_line_round_trips_through_the_message_types() {
    for line in goldens() {
        let at = format!("{}:{}", line.file, line.number);
        let parsed: ChannelMessage = serde_json::from_value(line.message.clone())
            .unwrap_or_else(|error| panic!("{at} does not deserialise: {error}"));
        let written = serde_json::to_value(&parsed)
            .unwrap_or_else(|error| panic!("{at} does not serialise: {error}"));
        assert_eq!(written, line.message, "{at} does not round-trip");
    }
}

#[test]
fn every_line_is_exactly_one_kind_of_message() {
    for line in goldens() {
        let at = format!("{}:{}", line.file, line.number);
        let object = line.message.as_object().expect("a JSON object");
        let is_request = object.contains_key("method") && object.contains_key("id");
        let is_notification = object.contains_key("method") && !object.contains_key("id");
        let is_response = object.contains_key("result");
        let is_error = object.contains_key("error");
        let kinds = usize::from(is_request)
            + usize::from(is_notification)
            + usize::from(is_response)
            + usize::from(is_error);
        assert_eq!(kinds, 1, "{at} looks like {kinds} kinds of message");

        let parsed: ChannelMessage =
            serde_json::from_value(line.message.clone()).expect("it deserialises");
        let matched = match parsed {
            ChannelMessage::Request(_) => is_request,
            ChannelMessage::Notification(_) => is_notification,
            ChannelMessage::Response(_) => is_response,
            ChannelMessage::ErrorResponse(_) => is_error,
        };
        assert!(matched, "{at} was read as the wrong kind of message");
        assert!(
            line.direction == "\u{2192}" || line.direction == "\u{2190}",
            "{at} has the direction {}",
            line.direction
        );
    }
}

#[test]
fn the_goldens_exercise_every_method_of_the_protocol() {
    let methods: BTreeSet<String> = goldens()
        .iter()
        .filter_map(|line| line.message.get("method"))
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let expected: BTreeSet<String> = [
        "hello",
        "handoff.open",
        "handoff.continue",
        "handoff.resume",
        "handoff.verify",
        "handoff.detach_call",
        "hook.stop",
        "session.bye",
        "handoff.event",
        "app.shutdown",
        "ping",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(methods, expected);
}

#[test]
fn both_hello_shapes_and_both_wire_errors_are_covered() {
    let mut server_hello = false;
    let mut hook_hello = false;
    let mut errors: BTreeSet<i64> = BTreeSet::new();

    for line in goldens() {
        if let Some(role) = line.message.pointer("/params/role").and_then(Value::as_str) {
            match role {
                "server" => server_hello = true,
                "hook" => hook_hello = true,
                other => panic!("unknown role {other}"),
            }
        }
        if let Some(code) = line.message.pointer("/error/code").and_then(Value::as_i64) {
            errors.insert(code);
        }
    }

    assert!(server_hello && hook_hello, "a hello shape is not covered");
    // The two the design fixes; the five application errors have no golden of their own,
    // and their codes are pinned by the enum below.
    assert!(errors.contains(&i64::from(ChannelErrorCode::AuthFailed.code())));
    assert!(errors.contains(&i64::from(ChannelErrorCode::ProtocolUnsupported.code())));
}

#[test]
fn the_error_codes_are_the_ones_the_protocol_readme_assigns() {
    let pairs: Vec<(i32, &str)> = ChannelErrorCode::all()
        .into_iter()
        .map(|code| (code.code(), code.name()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            (-32001, "auth_failed"),
            (-32002, "protocol_unsupported"),
            (-32010, "unknown_value_key"),
            (-32011, "not_waiting"),
            (-32012, "final"),
            (-32013, "no_verify_in_spec"),
            (-32014, "not_found"),
        ]
    );
}

#[test]
fn a_hello_claiming_another_protocol_version_still_parses() {
    // §6.5: the app must be able to answer `protocol_unsupported`, which it cannot do if
    // the message was dropped as a framing error. `protocol-mismatch.jsonl` is that case.
    let mismatch: Vec<Line> = goldens()
        .into_iter()
        .filter(|line| line.file == "protocol-mismatch.jsonl")
        .collect();
    assert_eq!(mismatch.len(), 2);
    let hello = &mismatch[0];
    assert_eq!(
        hello.message.pointer("/params/protocol_version"),
        Some(&Value::from(2))
    );
    validate(Document::ChannelMessage, &hello.message).expect("a version-2 hello still validates");
    let parsed: ChannelMessage =
        serde_json::from_value(hello.message.clone()).expect("it deserialises");
    match parsed {
        ChannelMessage::Request(request) => match request.body {
            RequestBody::Hello(_) => {}
            other => panic!("read as {other:?} instead of a hello"),
        },
        other => panic!("read as {other:?} instead of a request"),
    }
}

#[test]
fn an_outcome_delivered_by_an_event_is_the_published_outcome() {
    // `handoff.event` carries the outcome as it is published, which is what makes the app's
    // builder and the log agree with the server's reader (§6.3).
    let events: Vec<Line> = goldens()
        .into_iter()
        .filter(|line| line.message.get("method").and_then(Value::as_str) == Some("handoff.event"))
        .collect();
    assert!(!events.is_empty(), "no golden delivers an outcome");
    for line in events {
        let at = format!("{}:{}", line.file, line.number);
        let outcome = line
            .message
            .pointer("/params/outcome")
            .unwrap_or_else(|| panic!("{at} has no outcome"));
        validate(Document::Outcome, outcome)
            .unwrap_or_else(|problems| panic!("{at} carries an invalid outcome: {problems:?}"));
        let parsed: ChannelMessage =
            serde_json::from_value(line.message.clone()).expect("it deserialises");
        match parsed {
            ChannelMessage::Notification(notification) => match notification.body {
                NotificationBody::HandoffEvent(event) => {
                    assert!(
                        event.image.is_none(),
                        "{at} carries pixels; no golden should"
                    );
                }
                other => panic!("{at} read as {other:?}"),
            },
            other => panic!("{at} read as {other:?}"),
        }
    }
}
