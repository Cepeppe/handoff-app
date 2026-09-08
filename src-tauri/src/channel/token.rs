//! The per-installation channel token (§4.1, §5.8, §6.2, SRV-07, SRV-08, INST-07, FM-10).
//!
//! Thirty-two random bytes written as sixty-four lowercase hex characters in
//! `~/.handoff/channel.token`, with user-only permissions. The installer writes it (INST-07)
//! and the app makes sure it is there at every startup, because the token is what a fresh
//! installation, a repaired one and a folder the user deleted by hand all have to end up
//! with; the server re-reads the file at every connection attempt, so a token regenerated
//! from the settings screen is picked up without restarting anything.
//!
//! # The threat model, and what follows from it
//!
//! Verbatim from SRV-08: *the token protects against other users of the same machine and
//! against accidental connections. It does not protect against a malicious process already
//! running as the same user; that is the operating system's boundary.* Three consequences,
//! all of them visible in the code below:
//!
//! - **The comparison is constant-time** ([`Token::matches`]). The attacker the token
//!   defends against can reconnect as often as it likes, so a comparison that returned
//!   early on the first wrong nibble would hand out the token four bits at a time.
//! - **The file is owner-only on both platforms**, and on Windows that means an explicit
//!   ACL rather than the inherited default of the folder it lands in — the same descriptor
//!   the named pipe carries (`endpoint::security`, A-16).
//! - **Nothing here ever logs, formats or displays the token.** [`Token`] has no `Debug`
//!   that prints it and no `Display` at all; a refused attempt is logged by the listener
//!   with no token material (§6.2), and the only way to the characters is [`Token::as_str`],
//!   which exists for the one caller that has to compare them.
//!
//! A file that is present but not sixty-four hex characters is treated as absent and
//! rewritten: the app is the peer that owns this file, and a malformed one would make every
//! `hello` fail the schema and close as a framing violation, which is a failure nobody can
//! diagnose from the other end.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::LazyLock;

use rand::{rng, RngExt};
use regex::Regex;
use subtle::ConstantTimeEq;

/// The number of random bytes a token is made of (§4.1).
pub const TOKEN_BYTES: usize = 32;

/// Its length once hex-encoded: what `channel.v1.schema.json` requires of the `token` field.
pub const TOKEN_HEX_LENGTH: usize = TOKEN_BYTES * 2;

/// The mode the file carries on POSIX (INST-07).
pub const TOKEN_MODE: u32 = 0o600;

/// The shape the schema pins, anchored to the whole string.
///
/// `\A` and `\z` rather than `^` and `$`, for the reason `crate::ids` gives: the Rust
/// `regex` crate lets `$` match before a trailing newline and JavaScript's does not.
static TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[0-9a-f]{64}\z").expect("the token shape compiles"));

/// A channel token, known to be sixty-four lowercase hex characters.
///
/// `Debug` prints a placeholder: an error path that formats a struct holding one of these
/// must not put it in a log line or a crash file (R-19).
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl std::fmt::Debug for Token {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Token(<redacted>)")
    }
}

impl Token {
    /// A new random token.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; TOKEN_BYTES];
        rng().fill(bytes.as_mut_slice());
        let mut hex = String::with_capacity(TOKEN_HEX_LENGTH);
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        Self(hex)
    }

    /// The token a string holds, or nothing when it is not one.
    ///
    /// Trailing whitespace is tolerated because a text editor adds a newline and the
    /// installer's file is otherwise indistinguishable from an edited one; anything else is
    /// refused rather than repaired.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        TOKEN_RE.is_match(trimmed).then(|| Self(trimmed.to_owned()))
    }

    /// The characters, for the one caller that compares them.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `presented` is this token, compared in constant time (§6.2).
    ///
    /// The length is compared first and in the ordinary way. That leaks the length of the
    /// token, which is a published constant of the protocol and of the schema, and it is
    /// the only way to hand `subtle` two slices of equal length.
    #[must_use]
    pub fn matches(&self, presented: &str) -> bool {
        let expected = self.0.as_bytes();
        let offered = presented.as_bytes();
        if expected.len() != offered.len() {
            return false;
        }
        expected.ct_eq(offered).into()
    }
}

/// Reads the token file, or nothing when it is missing, unreadable or malformed.
#[must_use]
pub fn read(path: &Path) -> Option<Token> {
    fs::read_to_string(path)
        .ok()
        .as_deref()
        .and_then(Token::parse)
}

/// The token of this installation, written if it is not already there (INST-07).
///
/// # Errors
///
/// When the file has to be written and cannot be.
pub fn ensure(path: &Path) -> io::Result<Token> {
    match read(path) {
        Some(token) => Ok(token),
        None => regenerate(path),
    }
}

/// A new token, replacing whatever is there. The "repair token" action of the settings
/// screen (T-040) is this function and nothing else.
///
/// The old file is removed rather than truncated, because on Windows a security descriptor
/// is applied when a file is **created** and ignored when an existing one is opened: a
/// rewritten file would silently keep the ACL of the file it replaced.
///
/// # Errors
///
/// When the file cannot be removed or written.
pub fn regenerate(path: &Path) -> io::Result<Token> {
    let token = Token::generate();
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    write_private(path, token.as_str().as_bytes())?;
    Ok(token)
}

/// Creates a file only this user can read, and writes `contents` into it.
///
/// # Errors
///
/// When the file exists, or cannot be created or written.
#[cfg(unix)]
fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(TOKEN_MODE)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

/// The Windows half: `CreateFileW` with the same protected DACL the named pipe carries
/// (`endpoint::security`), because a file created without one inherits the folder's, and
/// `%USERPROFILE%\.handoff\` has whatever the profile hands down.
///
/// # Errors
///
/// When the descriptor cannot be built, or the file cannot be created or written.
#[cfg(windows)]
fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::{FromRawHandle as _, OwnedHandle};

    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, FILE_SHARE_MODE,
    };

    use super::endpoint::security::SecurityDescriptor;

    let mut descriptor = SecurityDescriptor::user_only()?;
    let mut attributes = descriptor.attributes();
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: `wide` is NUL-terminated and `attributes` points at `descriptor`; both
    // outlive the call. The handle Windows returns is adopted at once, so it is closed
    // exactly once, by `OwnedHandle`.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            Some(&raw mut attributes),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(io::Error::other)?
    };
    // SAFETY: the handle is valid, owned by us, and not used again through `handle`.
    let owned = unsafe { OwnedHandle::from_raw_handle(handle.0.cast()) };
    let mut file = fs::File::from(owned);
    file.write_all(contents)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary directory of this test binary's own, removed when the guard is dropped.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "handoff-token-{name}-{}-{}",
                std::process::id(),
                crate::ids::new_session_ref()
            ));
            fs::create_dir_all(&dir).expect("the temporary directory is created");
            Self(dir)
        }

        fn join(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_generated_token_is_sixty_four_lowercase_hex_characters() {
        let token = Token::generate();
        assert_eq!(token.as_str().len(), TOKEN_HEX_LENGTH);
        assert!(TOKEN_RE.is_match(token.as_str()));
    }

    #[test]
    fn two_generated_tokens_differ() {
        // Thirty-two bytes from the system generator; equality here would mean the source
        // of randomness is not one.
        assert_ne!(Token::generate().as_str(), Token::generate().as_str());
    }

    #[test]
    fn a_token_is_parsed_with_the_trailing_newline_an_editor_adds() {
        let token = Token::generate();
        let with_newline = format!("{}\n", token.as_str());
        assert_eq!(Token::parse(&with_newline).as_ref(), Some(&token));
    }

    #[test]
    fn anything_that_is_not_the_shape_is_not_a_token() {
        for text in [
            "",
            "not hex",
            &"F".repeat(64),                  // upper case: the schema pins lowercase
            &"a".repeat(63),                  // one short
            &"a".repeat(65),                  // one long
            &format!("{} x", "a".repeat(64)), // trailing rubbish, not whitespace
        ] {
            assert!(Token::parse(text).is_none(), "{text} was accepted");
        }
    }

    #[test]
    fn a_token_matches_itself_and_nothing_else() {
        let token = Token::generate();
        assert!(token.matches(token.as_str()));
        assert!(!token.matches(Token::generate().as_str()));
        assert!(!token.matches(""));
        // One nibble different at the very end: the comparison must not stop early, and
        // must not be fooled by a common prefix either.
        let mut nearly = token.as_str().to_owned();
        nearly.pop();
        nearly.push(if token.as_str().ends_with('0') {
            '1'
        } else {
            '0'
        });
        assert!(!token.matches(&nearly));
        assert!(!token.matches(&token.as_str()[..63]));
    }

    #[test]
    fn debug_never_prints_the_token() {
        let token = Token::generate();
        let printed = format!("{token:?}");
        assert!(!printed.contains(token.as_str()), "the token reached a log");
        assert_eq!(printed, "Token(<redacted>)");
    }

    #[test]
    fn ensure_writes_the_file_once_and_then_reads_it_back() {
        let dir = TempDir::new("ensure");
        let path = dir.join("channel.token");

        let first = ensure(&path).expect("the token is written");
        assert_eq!(
            fs::read_to_string(&path).expect("it is there").trim(),
            first.as_str()
        );

        let second = ensure(&path).expect("the token is read");
        assert_eq!(
            first.as_str(),
            second.as_str(),
            "ensure rewrote a good token"
        );
    }

    #[test]
    fn a_malformed_file_is_replaced_rather_than_used() {
        let dir = TempDir::new("malformed");
        let path = dir.join("channel.token");
        fs::write(&path, "this is not a token").expect("the file is written");

        let token = ensure(&path).expect("the token is repaired");
        assert!(TOKEN_RE.is_match(token.as_str()));
        assert_eq!(
            fs::read_to_string(&path).expect("it is there").trim(),
            token.as_str()
        );
    }

    #[test]
    fn regenerate_replaces_the_token() {
        let dir = TempDir::new("regenerate");
        let path = dir.join("channel.token");

        let first = ensure(&path).expect("the token is written");
        let second = regenerate(&path).expect("the token is replaced");
        assert_ne!(first.as_str(), second.as_str());
        assert_eq!(read(&path).expect("it is there").as_str(), second.as_str());
    }

    #[test]
    fn a_missing_file_reads_as_nothing_rather_than_failing() {
        let dir = TempDir::new("missing");
        assert!(read(&dir.join("nowhere.token")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only_on_posix() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new("mode");
        let path = dir.join("channel.token");
        ensure(&path).expect("the token is written");
        let mode = fs::metadata(&path)
            .expect("it is there")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, TOKEN_MODE, "the token is readable by someone else");
    }
}
