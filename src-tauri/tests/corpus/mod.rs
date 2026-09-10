//! The synthetic screenshot corpus of T-048: what is in it, and how it is drawn.
//!
//! Every image is a dashboard-looking page rendered from the table in [`pages`], with the
//! text a program wrote and a label for every line. Nothing here comes off a real screen —
//! `TASKS.md` T-048 says real screenshots are never committed, and a corpus of real ones
//! could not be published with this repository anyway. The keys are fabricated to the
//! shapes of §4.6 and match nothing that was ever issued.
//!
//! # Why the labels are declared and not detected
//!
//! `labels.json` records, for every line, what the corpus **says** it is: a certain secret
//! of a family, a suspected string with the rule that ought to fire, or clean. It is
//! written by hand here, never by running the detector — a corpus derived from the detector
//! would agree with it whatever it did, and the metrics of §11.7 would be a tautology. The
//! consequence is worth stating: when the gate fails, the fault may be the detector's or
//! the corpus's, and both are read.
//!
//! # Determinism
//!
//! `tests/gen_corpus.rs` regenerates every image and compares it with the committed one,
//! pixel for pixel, on whichever platform it runs. That holds because the renderer
//! ([`font`]) is a stroke font of our own with no system font behind it, and because its
//! arithmetic uses nothing but the four operations and `sqrt`, which IEEE-754 pins exactly.
//!
//! # The corpus is small on purpose
//!
//! Flat backgrounds and thin strokes compress to about 75 KB each, so the 46 images weigh
//! 3.4 MB in total and the largest is 158 KB against the size gate of the task (300 KB
//! each). A photograph would blow both.

#![allow(dead_code)]

mod font;

use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

/// What the corpus says one line is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum Expect {
    /// No detector should say anything about it.
    Clean,
    /// A certain pattern of this family must match (§4.6).
    Certain {
        /// One of the six families.
        kind: String,
    },
    /// The suspected detector must pick it out, with this rule (§7.10).
    Suspected {
        /// `hex`, `base64`, `entropy` or `label`.
        reason: String,
    },
}

/// One line of one image: what it says, where it is, and what it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineLabel {
    /// The text as it was drawn. This is the ground truth, not what an engine read.
    pub text: String,
    /// Left edge, in pixels of the image.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// What the corpus says about it.
    pub expect: Expect,
}

/// One image of the corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageLabels {
    /// The file name, inside `tests/fixtures/screenshots/`.
    pub file: String,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// The spec values of the handoff this page belongs to (DET-03).
    pub exempt: Vec<String>,
    /// Every line, in reading order.
    pub lines: Vec<LineLabel>,
}

/// The whole corpus, as `labels.json` carries it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Labels {
    /// A sentence for whoever opens the file expecting real screenshots.
    pub about: String,
    /// The images, in the order [`pages`] declares them.
    pub images: Vec<ImageLabels>,
}

/// Where the committed corpus lives.
pub const DIRECTORY: &str = "tests/fixtures/screenshots";

/// The file the labels are in.
pub const LABELS: &str = "labels.json";

/// How a page arranges a label and its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Both on one line, the value in a second column.
    Inline,
    /// The label on one line, the value on the one below it (§7.10's second half).
    Stacked,
}

/// Light or dark, so that the corpus is not one background colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    fn page(self) -> Rgba<u8> {
        match self {
            Self::Light => Rgba([0xf4, 0xf5, 0xf7, 0xff]),
            Self::Dark => Rgba([0x15, 0x17, 0x1c, 0xff]),
        }
    }

    fn panel(self) -> Rgba<u8> {
        match self {
            Self::Light => Rgba([0xff, 0xff, 0xff, 0xff]),
            Self::Dark => Rgba([0x22, 0x25, 0x2c, 0xff]),
        }
    }

    fn ink(self) -> Rgba<u8> {
        match self {
            Self::Light => Rgba([0x1b, 0x1e, 0x23, 0xff]),
            Self::Dark => Rgba([0xe8, 0xea, 0xee, 0xff]),
        }
    }

    fn muted(self) -> Rgba<u8> {
        match self {
            Self::Light => Rgba([0x5c, 0x63, 0x70, 0xff]),
            Self::Dark => Rgba([0x9b, 0xa2, 0xaf, 0xff]),
        }
    }

    fn bar(self) -> Rgba<u8> {
        match self {
            Self::Light => Rgba([0x30, 0x3a, 0x5a, 0xff]),
            Self::Dark => Rgba([0x2e, 0x3a, 0x66, 0xff]),
        }
    }

    fn bar_ink(self) -> Rgba<u8> {
        Rgba([0xf2, 0xf4, 0xf8, 0xff])
    }
}

/// One `label: value` of a page.
pub struct Row {
    pub label: &'static str,
    pub value: &'static str,
    pub expect: Expect,
}

/// One page of the corpus.
pub struct Page {
    pub name: &'static str,
    pub theme: Theme,
    pub layout: Layout,
    pub width: u32,
    pub height: u32,
    /// The cap height of the body text, in pixels.
    pub size: f64,
    pub title: &'static str,
    pub menu: &'static [&'static str],
    pub rows: Vec<Row>,
    /// Lines of prose under the rows. Always clean.
    pub notes: &'static [&'static str],
    /// The values the spec of this page's handoff declared (DET-03).
    pub exempt: &'static [&'static str],
}

/// A clean row.
fn clean(label: &'static str, value: &'static str) -> Row {
    Row {
        label,
        value,
        expect: Expect::Clean,
    }
}

/// A row whose value is a certain secret of `kind`.
fn certain(label: &'static str, value: &'static str, kind: &str) -> Row {
    Row {
        label,
        value,
        expect: Expect::Certain {
            kind: kind.to_owned(),
        },
    }
}

/// A row whose value the suspected detector must pick out, for `reason`.
fn suspected(label: &'static str, value: &'static str, reason: &str) -> Row {
    Row {
        label,
        value,
        expect: Expect::Suspected {
            reason: reason.to_owned(),
        },
    }
}

const BILLING_MENU: &[&str] = &["Home", "Payments", "Balances", "Customers", "Developers"];
const SETTINGS_MENU: &[&str] = &["General", "Members", "Webhooks", "API keys", "Billing"];
const CONSOLE_MENU: &[&str] = &["Overview", "Projects", "Deployments", "Logs", "Settings"];
const REPO_MENU: &[&str] = &["Code", "Issues", "Actions", "Packages", "Settings"];

/// The corpus, page by page.
///
/// The first block covers every family of §4.6, the second the four suspected rules, the
/// third the negatives that a heuristic detector is most likely to get wrong, and the
/// fourth the exemption of DET-03.
#[allow(clippy::too_many_lines)]
pub fn pages() -> Vec<Page> {
    vec![
        // ---- every certain family of §4.6, one page each -------------------------------
        Page {
            name: "certain-01-private-key",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1100,
            height: 620,
            size: 19.0,
            title: "Deploy keys",
            menu: REPO_MENU,
            rows: vec![
                clean("Name", "release-signing"),
                certain(
                    "Private key",
                    "-----BEGIN RSA PRIVATE KEY-----",
                    "private_key",
                ),
                clean("Added", "9 September 2026"),
            ],
            notes: &["Paste the key into the deploy settings of the release job."],
            exempt: &[],
        },
        Page {
            name: "certain-02-aws-access-key",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1180,
            height: 640,
            size: 20.0,
            title: "IAM console",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("User", "deploy-runner"),
                certain("Access key ID", "AKIAIOSFODNN7EXAMPLE", "api_key"),
                clean("Created", "2026-08-14"),
                clean("Status", "Active"),
            ],
            notes: &["Rotate the key every ninety days."],
            exempt: &[],
        },
        Page {
            name: "certain-03-stripe-secret",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1200,
            height: 660,
            size: 20.0,
            title: "API keys",
            menu: SETTINGS_MENU,
            rows: vec![
                suspected("Publishable key", "pk-live-onlyanexample", "label"),
                certain(
                    "Secret key",
                    "sk_live_51H8xQ2eZvKYlo2C0Sd8h4kL",
                    "api_key",
                ),
                clean("Last used", "3 hours ago"),
            ],
            notes: &["Reveal the secret key only when you are about to paste it."],
            exempt: &[],
        },
        Page {
            name: "certain-04-stripe-test-key",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1024,
            height: 600,
            size: 18.0,
            title: "Test mode keys",
            menu: SETTINGS_MENU,
            rows: vec![
                certain(
                    "Restricted key",
                    "rk_test_9Kb2LmQpXr4TuVwYz6Ab8Cd",
                    "api_key",
                ),
                clean("Scope", "Read only"),
            ],
            notes: &["Test keys never move money."],
            exempt: &[],
        },
        Page {
            name: "certain-05-stripe-webhook-secret",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1120,
            height: 620,
            size: 19.0,
            title: "Webhook endpoint",
            menu: SETTINGS_MENU,
            rows: vec![
                clean("Endpoint", "https://example.test/hooks/payments"),
                certain(
                    "Signing secret",
                    "whsec_9f86d081884c7d659a2feaa0c5",
                    "webhook_secret",
                ),
                clean("Events", "payment_intent.succeeded"),
            ],
            notes: &["The signing secret verifies that the call came from us."],
            exempt: &[],
        },
        Page {
            name: "certain-06-github-token",
            theme: Theme::Dark,
            layout: Layout::Inline,
            width: 1200,
            height: 640,
            size: 20.0,
            title: "Personal access tokens",
            menu: REPO_MENU,
            rows: vec![
                clean("Note", "release automation"),
                certain(
                    "Token",
                    "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8",
                    "token",
                ),
                clean("Expires", "in 90 days"),
            ],
            notes: &["Copy it now: it is shown once."],
            exempt: &[],
        },
        Page {
            name: "certain-07-github-fine-grained",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1400,
            height: 620,
            size: 18.0,
            title: "Fine-grained token",
            menu: REPO_MENU,
            rows: vec![
                certain(
                    "Token",
                    "github_pat_11ABCDEFG011ABCDEFG011ABCDEFG011ABCDEFG011ABCDEFG011ABCDEFG011ABCDEFG011ABCDEFG0",
                    "token",
                ),
                clean("Repositories", "handoff-app"),
            ],
            notes: &["Only the repositories you chose are reachable."],
            exempt: &[],
        },
        Page {
            name: "certain-08-slack-token",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1150,
            height: 620,
            size: 19.0,
            title: "App credentials",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("App", "handoff-notifier"),
                certain(
                    "Bot token",
                    "xoxb-2401234567-4707051479041-abcDEF123",
                    "token",
                ),
                clean("Scopes", "chat:write"),
            ],
            notes: &["Reinstall the app after changing a scope."],
            exempt: &[],
        },
        Page {
            name: "certain-09-slack-webhook-url",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1280,
            height: 600,
            size: 18.0,
            title: "Incoming webhooks",
            menu: CONSOLE_MENU,
            rows: vec![
                certain(
                    "Webhook URL",
                    "https://hooks.slack.com/services/T0123ABCD/B9876ZYXW/aBcDeF123456",
                    "webhook_url",
                ),
                clean("Channel", "alerts"),
            ],
            notes: &["Anyone holding this address can post to the channel."],
            exempt: &[],
        },
        Page {
            name: "certain-10-google-api-key",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1200,
            height: 640,
            size: 20.0,
            title: "Credentials",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("Project", "handoff-maps"),
                certain(
                    "API key",
                    "AIzaSyD4x7Qk2mNp0RtVwZ1bC3eF5gH8jK9lMnQ",
                    "api_key",
                ),
                clean("Restrictions", "HTTP referrers"),
            ],
            notes: &["An unrestricted key can be used from anywhere."],
            exempt: &[],
        },
        Page {
            name: "certain-11-anthropic-key",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1100,
            height: 600,
            size: 19.0,
            title: "Workspace keys",
            menu: CONSOLE_MENU,
            rows: vec![
                certain(
                    "API key",
                    "sk-ant-api03-A1b2C3d4E5f6G7h8I9j0KlMn",
                    "api_key",
                ),
                clean("Workspace", "Default"),
            ],
            notes: &["Keys are scoped to one workspace."],
            exempt: &[],
        },
        Page {
            name: "certain-12-openai-key",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1180,
            height: 620,
            size: 19.0,
            title: "Project keys",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("Owner", "platform team"),
                certain(
                    "Secret",
                    "sk-proj-A1b2C3d4E5f6G7h8I9j0KlMnOpQr",
                    "api_key",
                ),
                clean("Created", "12 August 2026"),
            ],
            notes: &["Delete a key you cannot account for."],
            exempt: &[],
        },
        Page {
            name: "certain-13-gitlab-pat",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1120,
            height: 600,
            size: 19.0,
            title: "Access tokens",
            menu: REPO_MENU,
            rows: vec![
                clean("Name", "ci-runner"),
                certain("Token", "glpat-A1b2C3d4E5f6G7h8I9j0", "token"),
                clean("Scopes", "read_registry"),
            ],
            notes: &["The token inherits your own permissions."],
            exempt: &[],
        },
        Page {
            name: "certain-14-npm-token",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1200,
            height: 600,
            size: 19.0,
            title: "Publishing tokens",
            menu: CONSOLE_MENU,
            rows: vec![
                certain(
                    "Token",
                    "npm_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8",
                    "token",
                ),
                clean("Type", "Automation"),
            ],
            notes: &["An automation token bypasses two-factor authentication."],
            exempt: &[],
        },
        Page {
            name: "certain-15-sendgrid-key",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1400,
            height: 620,
            size: 18.0,
            title: "Mail settings",
            menu: SETTINGS_MENU,
            rows: vec![
                certain(
                    "API key",
                    "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",
                    "api_key",
                ),
                clean("Sender", "receipts at example test"),
            ],
            notes: &["Full access keys can send on behalf of every sender."],
            exempt: &[],
        },
        Page {
            name: "certain-16-huggingface-token",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1150,
            height: 600,
            size: 19.0,
            title: "Access tokens",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("Name", "model-download"),
                certain("Token", "hf_QzWxEcRvTyBnUmIoLpAsDfGhJkZxCv3456", "token"),
                clean("Role", "read"),
            ],
            notes: &["A read token cannot push a model."],
            exempt: &[],
        },
        Page {
            name: "certain-17-digitalocean-token",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1400,
            height: 600,
            size: 18.0,
            title: "Applications and API",
            menu: CONSOLE_MENU,
            rows: vec![
                certain(
                    "Personal access token",
                    "dop_v1_9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
                    "token",
                ),
                clean("Scope", "read and write"),
            ],
            notes: &["Regenerate the token if a laptop is lost."],
            exempt: &[],
        },
        Page {
            name: "certain-18-jwt",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1500,
            height: 620,
            size: 18.0,
            title: "Session inspector",
            menu: CONSOLE_MENU,
            rows: vec![
                certain(
                    "Bearer",
                    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r-wW1gFWFOEjXk",
                    "jwt",
                ),
                clean("Expires", "in 14 minutes"),
            ],
            notes: &["A bearer token is a password with an expiry date."],
            exempt: &[],
        },
        Page {
            name: "certain-19-two-secrets-one-page",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1300,
            height: 680,
            size: 19.0,
            title: "Environment",
            menu: CONSOLE_MENU,
            rows: vec![
                certain("STRIPE_KEY", "sk_test_4eC39HqLyjWDarjtT1zdp7dc", "api_key"),
                certain("AWS_ACCESS_KEY_ID", "ASIAY34FZKBOKMUTVV7A", "api_key"),
                clean("REGION", "eu-west-1"),
                clean("LOG_LEVEL", "info"),
            ],
            notes: &["Variables are read at boot and never re-read."],
            exempt: &[],
        },
        Page {
            name: "certain-20-large-console",
            theme: Theme::Dark,
            layout: Layout::Inline,
            width: 1920,
            height: 1080,
            size: 26.0,
            title: "Production console",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("Service", "payments-api"),
                certain(
                    "Signing secret",
                    "whsec_ab12CD34ef56GH78ij90KL12mn",
                    "webhook_secret",
                ),
                clean("Replicas", "6"),
                clean("Region", "eu-central-1"),
            ],
            notes: &[
                "This page is captured at more than 1600 pixels on the long side,",
                "so the burn-in has to rescale every box onto the reduced image.",
            ],
            exempt: &[],
        },
        // ---- the four suspected rules ---------------------------------------------------
        Page {
            name: "suspected-01-hex-digest",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1300,
            height: 620,
            size: 19.0,
            title: "Build artefact",
            menu: CONSOLE_MENU,
            rows: vec![
                suspected(
                    "Digest",
                    "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
                    "hex",
                ),
                clean("Size", "18.4 MB"),
            ],
            notes: &["A digest is not a secret, and the detector cannot know that."],
            exempt: &[],
        },
        Page {
            name: "suspected-02-base64-blob",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1200,
            height: 600,
            size: 19.0,
            title: "Encoded configuration",
            menu: CONSOLE_MENU,
            rows: vec![
                suspected("Payload", "ZHVtbXlUb2tlblZhbHVlMTIzNDU2Nzg5", "base64"),
                clean("Encoding", "base64"),
            ],
            notes: &["Decode it before pasting it anywhere."],
            exempt: &[],
        },
        Page {
            name: "suspected-03-random-string",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1100,
            height: 600,
            size: 19.0,
            title: "Device registration",
            menu: CONSOLE_MENU,
            rows: vec![
                suspected("Pairing code", "Zq7Z4tR2xL9pV0mW3kBn", "entropy"),
                clean("Valid for", "10 minutes"),
            ],
            notes: &["Type the code into the device within ten minutes."],
            exempt: &[],
        },
        Page {
            name: "suspected-04-short-password",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1050,
            height: 580,
            size: 19.0,
            title: "Router admin",
            menu: SETTINGS_MENU,
            rows: vec![
                clean("User name", "admin"),
                suspected("Password", "hunter2-tango", "label"),
                clean("Interface", "eth0"),
            ],
            notes: &["Change the password before the router faces the internet."],
            exempt: &[],
        },
        Page {
            name: "suspected-05-label-on-the-line-above",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1100,
            height: 600,
            size: 19.0,
            title: "Service account",
            menu: CONSOLE_MENU,
            rows: vec![
                suspected("Client secret", "rotate-me-4711", "label"),
                clean("Client id", "handoff-app"),
            ],
            notes: &["The secret is shown once, on the line under its label."],
            exempt: &[],
        },
        Page {
            name: "suspected-06-uuid",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1200,
            height: 600,
            size: 19.0,
            title: "Resource",
            menu: CONSOLE_MENU,
            rows: vec![
                suspected(
                    "Identifier",
                    "550e8400-e29b-41d4-a716-446655440000",
                    // The URL-safe base64 alphabet has the hyphen in it, so this rule
                    // reaches the shape before the entropy one does.
                    "base64",
                ),
                clean("Kind", "Deployment"),
            ],
            notes: &["An identifier that long is indistinguishable from a credential."],
            exempt: &[],
        },
        Page {
            name: "suspected-07-bearer-header",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1300,
            height: 620,
            size: 18.0,
            title: "Request inspector",
            menu: CONSOLE_MENU,
            rows: vec![
                suspected("Authorization bearer", "opaque-4711-zulu", "label"),
                clean("Method", "POST"),
                clean("Status", "204 No Content"),
            ],
            notes: &["Headers are copied with the request when you share it."],
            exempt: &[],
        },
        Page {
            name: "suspected-08-mixed-page",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1300,
            height: 700,
            size: 19.0,
            title: "Integration",
            menu: SETTINGS_MENU,
            rows: vec![
                clean("Name", "Nightly export"),
                suspected(
                    "Checksum",
                    "a3bf4f1b2b0b822cd15d6c15b0f00a089f86d081884c7d659a2feaa0c55ad015",
                    "hex",
                ),
                suspected("Secret phrase", "correct-horse-99", "label"),
                clean("Schedule", "Every night at 02:00"),
            ],
            notes: &["Two different reasons on one page."],
            exempt: &[],
        },
        // ---- negatives: what a heuristic gets wrong -------------------------------------
        Page {
            name: "negative-01-plain-settings",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1100,
            height: 620,
            size: 19.0,
            title: "General settings",
            menu: SETTINGS_MENU,
            rows: vec![
                clean("Organisation", "Example Limited"),
                clean("Time zone", "Europe/Rome"),
                clean("Language", "English"),
                clean("Support plan", "Standard"),
            ],
            notes: &["Nothing on this page is a credential."],
            exempt: &[],
        },
        Page {
            name: "negative-02-long-addresses",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1300,
            height: 640,
            size: 19.0,
            title: "Documentation",
            menu: SETTINGS_MENU,
            rows: vec![
                clean("Guide", "dashboard.stripe.com/settings/webhooks"),
                clean("Reference", "docs.example.test/reference/payments/intents"),
            ],
            notes: &[
                "A long address has as much entropy as a random string,",
                "which is why the entropy rule asks for digits as well as letters.",
            ],
            exempt: &[],
        },
        Page {
            name: "negative-03-file-paths",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1300,
            height: 620,
            size: 19.0,
            title: "Build output",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("Bundle", "target/release/handoff-app.exe"),
                clean("Manifest", "src-tauri/tauri.conf.json"),
            ],
            notes: &["Paths are not credentials, however deep they are."],
            exempt: &[],
        },
        Page {
            name: "negative-04-prose",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1100,
            height: 640,
            size: 19.0,
            title: "Release notes",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("Version", "2.14.3"),
                clean("Released", "9 September 2026"),
            ],
            notes: &[
                "Open the API key page in the dashboard and press Reveal.",
                "The certain patterns did not change in this release.",
                "Nothing in these three sentences is a secret.",
            ],
            exempt: &[],
        },
        Page {
            name: "negative-05-invoice",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1150,
            height: 660,
            size: 19.0,
            title: "Invoice",
            menu: BILLING_MENU,
            rows: vec![
                clean("Number", "INV-2026-0914"),
                clean("Amount", "1.240,00 EUR"),
                clean("Due", "30 September 2026"),
                clean("Status", "Open"),
            ],
            notes: &["Numbers and dates are the ordinary content of a dashboard."],
            exempt: &[],
        },
        Page {
            name: "negative-06-members",
            theme: Theme::Dark,
            layout: Layout::Inline,
            width: 1200,
            height: 660,
            size: 19.0,
            title: "Members",
            menu: SETTINGS_MENU,
            rows: vec![
                clean("Giuseppe", "Administrator"),
                clean("Renata", "Developer"),
                clean("Tommaso", "Read only"),
                clean("Invitations", "None pending"),
            ],
            notes: &["Names and roles, nothing else."],
            exempt: &[],
        },
        Page {
            name: "negative-07-words-that-look-like-labels",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1150,
            height: 620,
            size: 19.0,
            title: "Inventory",
            menu: BILLING_MENU,
            rows: vec![
                clean("Keyboard", "Mechanical, Italian layout"),
                clean("Monkey wrench", "Two sizes"),
                clean("Tokenizer", "Version 3"),
            ],
            notes: &["A word containing key is not a label."],
            exempt: &[],
        },
        Page {
            name: "negative-08-short-identifiers",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1100,
            height: 620,
            size: 19.0,
            title: "Orders",
            menu: BILLING_MENU,
            rows: vec![
                clean("Order", "A-4711"),
                clean("Batch", "B-9931"),
                clean("Shipment", "S-0042"),
                clean("Carrier", "Local"),
            ],
            notes: &["Short identifiers are below every threshold."],
            exempt: &[],
        },
        Page {
            name: "negative-09-numbers",
            theme: Theme::Dark,
            layout: Layout::Inline,
            width: 1150,
            height: 640,
            size: 19.0,
            title: "Usage",
            menu: BILLING_MENU,
            rows: vec![
                clean("Requests", "1.204.881"),
                clean("Errors", "0,04 per cent"),
                clean("Latency", "184 ms"),
                clean("Window", "Last 24 hours"),
            ],
            notes: &["A page of digits is not a page of secrets."],
            exempt: &[],
        },
        Page {
            name: "negative-10-large-empty",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1920,
            height: 1080,
            size: 26.0,
            title: "Dashboard",
            menu: BILLING_MENU,
            rows: vec![
                clean("Payments today", "412"),
                clean("Refunds", "3"),
                clean("Disputes", "0"),
            ],
            notes: &["A wide page with nothing to redact still has to survive the resize."],
            exempt: &[],
        },
        Page {
            name: "negative-11-small-text",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 900,
            height: 520,
            size: 14.0,
            title: "Compact table",
            menu: BILLING_MENU,
            rows: vec![
                clean("Row one", "Value one"),
                clean("Row two", "Value two"),
                clean("Row three", "Value three"),
            ],
            notes: &["Small text is where R-06 lives: an engine that cannot read it."],
            exempt: &[],
        },
        Page {
            name: "negative-12-dark-compact",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 960,
            height: 560,
            size: 16.0,
            title: "Logs",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("Last entry", "started listening on the channel"),
                clean("Level", "info"),
            ],
            notes: &["Log lines are prose with punctuation."],
            exempt: &[],
        },
        // ---- the exemption of DET-03 ----------------------------------------------------
        Page {
            name: "exempt-01-endpoint-id",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1200,
            height: 620,
            size: 19.0,
            title: "Webhook endpoints",
            menu: SETTINGS_MENU,
            rows: vec![
                clean("Endpoint", "we_1P9xTz2eZvKYlo2C0Sd8h4kL"),
                clean("Status", "Enabled"),
            ],
            notes: &["The agent put this identifier in the spec, so it is not a suspicion."],
            exempt: &["we_1P9xTz2eZvKYlo2C0Sd8h4kL"],
        },
        Page {
            name: "exempt-02-reference-number",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1150,
            height: 600,
            size: 19.0,
            title: "Transfer",
            menu: BILLING_MENU,
            rows: vec![
                clean("Reference", "REF-8Kd3Nq7Xz2Mv5Pw9Ct"),
                clean("Amount", "480,00 EUR"),
            ],
            notes: &["The reference is a declared value of the handoff."],
            exempt: &["REF-8Kd3Nq7Xz2Mv5Pw9Ct"],
        },
        Page {
            name: "exempt-03-digest-declared",
            theme: Theme::Dark,
            layout: Layout::Stacked,
            width: 1300,
            height: 620,
            size: 18.0,
            title: "Artefact to verify",
            menu: CONSOLE_MENU,
            rows: vec![
                clean(
                    "Digest",
                    "3bf4f1b2b0b822cd15d6c15b0f00a0899f86d081884c7d659a2feaa0c55ad015",
                ),
                clean("Format", "sha256"),
            ],
            notes: &["A digest the agent asked the user to compare is not redacted."],
            exempt: &["3bf4f1b2b0b822cd15d6c15b0f00a0899f86d081884c7d659a2feaa0c55ad015"],
        },
        Page {
            name: "exempt-04-value-and-secret",
            theme: Theme::Light,
            layout: Layout::Inline,
            width: 1300,
            height: 660,
            size: 19.0,
            title: "Connection",
            menu: SETTINGS_MENU,
            rows: vec![
                clean("Account", "acct-4711-alpha-zulu"),
                certain(
                    "Secret key",
                    "sk_live_7Hb3JkQ9zXc2VnMp5RtY8Uw",
                    "api_key",
                ),
                clean("Mode", "Live"),
            ],
            notes: &["The account is exempt; the key beside it is not, and cannot be."],
            exempt: &["acct-4711-alpha-zulu"],
        },
        Page {
            name: "exempt-05-declared-secret-is-not-exempt",
            theme: Theme::Light,
            layout: Layout::Stacked,
            width: 1200,
            height: 620,
            size: 19.0,
            title: "Paste this key",
            menu: SETTINGS_MENU,
            rows: vec![
                certain(
                    "API key",
                    "AIzaSyQ9zXc2VnMp5RtY8Uw4eC39HqLyjWDarjt",
                    "api_key",
                ),
                clean("Target", "the deployment settings page"),
            ],
            notes: &["DET-03 exempts a value that passed the certain check, and no other."],
            exempt: &["AIzaSyQ9zXc2VnMp5RtY8Uw4eC39HqLyjWDarjt"],
        },
        Page {
            name: "exempt-06-two-values",
            theme: Theme::Dark,
            layout: Layout::Inline,
            width: 1250,
            height: 660,
            size: 19.0,
            title: "Migration",
            menu: CONSOLE_MENU,
            rows: vec![
                clean("From", "cluster-7f3a-old-2026"),
                clean("To", "cluster-91bd-new-2026"),
                clean("Window", "Sunday 02:00"),
            ],
            notes: &["Both cluster names are declared values."],
            exempt: &["cluster-7f3a-old-2026", "cluster-91bd-new-2026"],
        },
    ]
}

/// Fills a rectangle with one colour.
fn fill(image: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32, colour: Rgba<u8>) {
    for row in y..(y + height).min(image.height()) {
        for column in x..(x + width).min(image.width()) {
            image.put_pixel(column, row, colour);
        }
    }
}

/// One line as it was drawn, before it is labelled.
struct Drawn {
    text: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// Draws one line with its baseline at `(x, baseline)` and answers where the ink can be.
///
/// The rectangle is the glyph box rather than a measurement of the pixels: the topmost
/// glyph of the font reaches eight units above the cap line and the lowest eight below the
/// descender, so the box is the cap height stretched by those two and by the pen's radius.
/// It is what a well-behaved OCR engine would report for the line, and it is what the
/// geometry test of `tests/security/redaction.rs` requires the burn to cover.
fn line(
    image: &mut RgbaImage,
    text: &str,
    x: f64,
    baseline: f64,
    size: f64,
    ink: Rgba<u8>,
) -> Drawn {
    let width = font::draw(image, text, x, baseline, size, ink);
    let pen = size * 0.06 + 1.0;
    let top = baseline - size * 1.10 - pen;
    let bottom = baseline + size * 0.34 + pen;
    #[allow(clippy::cast_possible_truncation)]
    Drawn {
        text: text.to_owned(),
        x: (x - pen).floor() as i32,
        y: top.floor() as i32,
        width: (width + 2.0 * pen).ceil() as u32,
        height: (bottom - top).ceil() as u32,
    }
}

/// Renders one page and answers its pixels and its labels.
pub fn render(page: &Page) -> (RgbaImage, ImageLabels) {
    let theme = page.theme;
    let size = page.size;
    let mut image = RgbaImage::from_pixel(page.width, page.height, theme.page());

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bar_height = (size * 2.6) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let menu_width = (f64::from(page.width) * 0.22) as u32;
    let pad = size * 1.4;

    fill(&mut image, 0, 0, page.width, bar_height, theme.bar());
    fill(
        &mut image,
        0,
        bar_height,
        menu_width,
        page.height - bar_height,
        theme.panel(),
    );
    fill(
        &mut image,
        menu_width + 1,
        bar_height + 1,
        page.width - menu_width - 2,
        page.height - bar_height - 2,
        theme.page(),
    );

    let mut lines: Vec<LineLabel> = Vec::new();
    let clean_line = |image: &mut RgbaImage, text: &str, x: f64, y: f64, ink: Rgba<u8>| {
        let drawn = line(image, text, x, y, size, ink);
        LineLabel {
            text: drawn.text,
            x: drawn.x,
            y: drawn.y,
            width: drawn.width,
            height: drawn.height,
            expect: Expect::Clean,
        }
    };

    lines.push(clean_line(
        &mut image,
        page.title,
        pad,
        f64::from(bar_height) - size * 0.7,
        theme.bar_ink(),
    ));

    let mut menu_y = f64::from(bar_height) + pad + size;
    for entry in page.menu {
        lines.push(clean_line(&mut image, entry, pad, menu_y, theme.muted()));
        menu_y += size * 2.0;
    }

    let content_x = f64::from(menu_width) + pad;
    let value_x = content_x + f64::from(page.width) * 0.22;
    let mut y = f64::from(bar_height) + pad + size;
    for row in &page.rows {
        match page.layout {
            Layout::Inline => {
                let label = line(&mut image, row.label, content_x, y, size, theme.muted());
                let value = line(&mut image, row.value, value_x, y, size, theme.ink());
                let whole = format!("{}   {}", label.text, value.text);
                #[allow(clippy::cast_possible_truncation)]
                lines.push(LineLabel {
                    text: whole,
                    x: label.x,
                    y: label.y.min(value.y),
                    width: (value.x - label.x) as u32 + value.width,
                    height: label.height.max(value.height),
                    expect: row.expect.clone(),
                });
                y += size * 2.4;
            }
            Layout::Stacked => {
                lines.push(clean_line(
                    &mut image,
                    row.label,
                    content_x,
                    y,
                    theme.muted(),
                ));
                y += size * 1.8;
                let value = line(&mut image, row.value, content_x, y, size, theme.ink());
                lines.push(LineLabel {
                    text: value.text,
                    x: value.x,
                    y: value.y,
                    width: value.width,
                    height: value.height,
                    expect: row.expect.clone(),
                });
                y += size * 2.4;
            }
        }
    }

    y += size * 1.2;
    for note in page.notes {
        lines.push(clean_line(&mut image, note, content_x, y, theme.muted()));
        y += size * 1.8;
    }

    let labels = ImageLabels {
        file: format!("{}.png", page.name),
        width: page.width,
        height: page.height,
        exempt: page
            .exempt
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        lines,
    };
    (image, labels)
}

/// Every character the corpus draws, for the glyph-coverage check.
pub fn every_text(page: &Page) -> Vec<String> {
    let mut texts = vec![page.title.to_owned()];
    texts.extend(page.menu.iter().map(|entry| (*entry).to_owned()));
    for row in &page.rows {
        texts.push(row.label.to_owned());
        texts.push(row.value.to_owned());
    }
    texts.extend(page.notes.iter().map(|note| (*note).to_owned()));
    texts
}

/// Whether the font can draw every character of `text`.
pub fn covered(text: &str) -> bool {
    font::covers(text)
}

/// The sentence `labels.json` opens with.
pub const ABOUT: &str = "Synthetic corpus generated by `cargo test --test gen_corpus`. \
Every page, every label and every key in it was written by a program; no real screen and \
no issued credential is here. Regenerate with HANDOFF_WRITE_CORPUS=1.";
