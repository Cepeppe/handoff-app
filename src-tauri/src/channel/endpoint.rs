//! Where the app listens, and what protects it (§4.1, §5.8, §6.2, DD-26, FM-12, A-16, A-17).
//!
//! The endpoint is the one name two independent implementations have to agree on without
//! talking to each other first. The server computes it in TypeScript from its own
//! environment, the app computes it here in Rust from the same environment, and if the two
//! differ by one byte nothing ever connects and neither side can say why. Everything below
//! is therefore a literal transcription of the design rather than a convenience:
//!
//! - **Windows.** `\\.\pipe\handoff-<h>`, `h` being the first 16 hex digits of the SHA-256
//!   of the lower-cased `USERDOMAIN\USERNAME`. The pipe namespace is machine-global, so the
//!   suffix is what keeps two users' apps apart (FM-12). When `HANDOFF_HOME` is set — tests
//!   and the e2e isolation of `TASKS.md` §0.4 item 4 — it is mixed in as `<user>|<home>`,
//!   so a test instance cannot land on the pipe of the app the owner is actually using. A
//!   variable that is not set contributes an **empty string**: any cleverness on one side,
//!   such as asking the OS for the user name, would move the endpoint out from under the
//!   other peer.
//! - **macOS.** `~/.handoff/app.sock`, mode `0600`. A Unix socket path lives in a
//!   `sun_path` of 104 **bytes** (A-17), and `HANDOFF_HOME` can be anywhere, so when the
//!   path does not fit the app binds a short one under the temporary directory and writes
//!   the real path into `~/.handoff/app.sock.path`, which is the pointer file the server
//!   reads (FM-12).
//!
//! Linux is not a supported platform (REQUIREMENTS §1.5); it gets the macOS shape because
//! the code is written once.
//!
//! # Both shapes exist on both platforms, on purpose
//!
//! [`resolve_for`] takes the platform as an argument and the two variants of [`Endpoint`]
//! are compiled everywhere, so the Windows digest is checked against the hex values
//! `handoff-mcp` pinned — in `test/unit/platform/paths.test.ts`, computed once and never
//! recomputed — by the unit tests of every host, including the macOS CI leg. A parity test
//! that only runs on the platform it describes is a parity test that runs after the damage.
//!
//! # The DACL (A-16)
//!
//! A named pipe created with no security attributes inherits the default DACL of the
//! process token, which on a domain-joined machine can be wider than the user. [`security`]
//! builds an explicit, protected DACL granting `GENERIC_ALL` to the SID of the account this
//! process runs as and to nobody else, and the listener hands it to `CreateNamedPipe`
//! through `SECURITY_ATTRIBUTES`. Whether it really keeps a second user out is a
//! second-user test the owner runs by hand (T-057); what is testable here is that the
//! descriptor is built, that it names this process's own SID, and that a pipe created with
//! it still accepts a connection from ourselves.

use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::paths;

/// The prefix of the named pipe (§4.1). The suffix is the digest below.
pub const PIPE_PREFIX: &str = r"\\.\pipe\handoff-";

/// How many hex digits of the digest name the pipe (§4.1: the first 16).
pub const PIPE_SUFFIX_LENGTH: usize = 16;

/// The size of `sun_path` on macOS, in bytes, as §5.8 states it (A-17).
///
/// A path is measured in UTF-8 bytes and not in characters: a home folder with an accented
/// letter spends two bytes on it, exactly as the kernel counts it.
pub const SUN_PATH_MAX_BYTES: usize = 104;

/// The mode the socket file carries (§6.2): readable and writable by its owner alone.
pub const SOCKET_MODE: u32 = 0o600;

/// The mode `~/.handoff/` carries on POSIX. The token and the socket live in it, so the
/// folder is closed as well; the server only ever reads files it owns the names of.
pub const HOME_MODE: u32 = 0o700;

/// Which shape of endpoint a platform uses. Taken as an argument rather than read from
/// `cfg!`, so both derivations are exercised by the unit tests of every host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// A named pipe.
    Windows,
    /// A Unix domain socket.
    Posix,
}

impl Platform {
    /// The platform this build runs on.
    #[must_use]
    pub fn host() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Posix
        }
    }
}

/// Where the app listens and the server connects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// `\\.\pipe\handoff-<h>`.
    Pipe {
        /// The full pipe name, prefix included.
        name: String,
    },
    /// A Unix domain socket, and the pointer file to write when it is not the path the
    /// server derives on its own (FM-12).
    Unix {
        /// What the listener binds.
        path: PathBuf,
        /// `~/.handoff/app.sock.path`, or nothing when `path` is the default and the
        /// server finds it without help.
        pointer: Option<PathBuf>,
    },
}

impl Endpoint {
    /// The endpoint of this process, from the environment (§5.8).
    #[must_use]
    pub fn resolve() -> Self {
        resolve_for(
            Platform::host(),
            &paths::handoff_home(),
            &windows_user_key(),
            paths::handoff_home_override().as_deref(),
            &std::env::temp_dir(),
        )
    }

    /// What a log line or a `doctor`-style report calls this endpoint.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Pipe { name } => name.clone(),
            Self::Unix { path, .. } => path.display().to_string(),
        }
    }

    /// The pointer file this endpoint needs, if any.
    #[must_use]
    pub fn pointer(&self) -> Option<&Path> {
        match self {
            Self::Pipe { .. } => None,
            Self::Unix { pointer, .. } => pointer.as_deref(),
        }
    }
}

/// The endpoint, from explicit inputs. The pure half of [`Endpoint::resolve`].
///
/// `home` is the contract folder, `user_key` the lower-cased `USERDOMAIN\USERNAME`,
/// `home_override` the raw value of `HANDOFF_HOME` when it is set, and `temp` the platform
/// temporary directory the fallback socket goes in.
#[must_use]
pub fn resolve_for(
    platform: Platform,
    home: &Path,
    user_key: &str,
    home_override: Option<&str>,
    temp: &Path,
) -> Endpoint {
    if platform == Platform::Windows {
        return Endpoint::Pipe {
            name: pipe_name(user_key, home_override),
        };
    }

    let default = home.join(paths::SOCKET_FILE_NAME);
    if !exceeds_sun_path(&default) {
        return Endpoint::Unix {
            path: default,
            pointer: None,
        };
    }

    Endpoint::Unix {
        path: short_socket_path(&default, temp),
        pointer: Some(home.join(paths::SOCKET_POINTER_FILE_NAME)),
    }
}

/// `\\.\pipe\handoff-<h>` for a user (§4.1, §5.8, DD-26).
#[must_use]
pub fn pipe_name(user_key: &str, home_override: Option<&str>) -> String {
    format!("{PIPE_PREFIX}{}", pipe_suffix(user_key, home_override))
}

/// The 16 hex digits that name the pipe.
///
/// The digest is over `<user>` or, when `HANDOFF_HOME` is set, `<user>|<home>`: nothing
/// else is added and nothing else is lower-cased. The server computes the same string in
/// `src/platform/paths.ts`.
#[must_use]
pub fn pipe_suffix(user_key: &str, home_override: Option<&str>) -> String {
    let material = match home_override {
        Some(home) => format!("{user_key}|{home}"),
        None => user_key.to_owned(),
    };
    short_digest(&material)
}

/// `USERDOMAIN\USERNAME` in lower case, from this process's environment.
///
/// An unset or blank variable contributes an empty string, so the key of a session with
/// neither is `\`. That is deliberate and it is the server's rule too: the two peers read
/// the same two variables and neither is allowed to be cleverer than the other (§5.8).
#[must_use]
pub fn windows_user_key() -> String {
    user_key_from(
        paths::env_value("USERDOMAIN").as_deref(),
        paths::env_value("USERNAME").as_deref(),
    )
}

/// The pure half of [`windows_user_key`].
#[must_use]
pub fn user_key_from(domain: Option<&str>, user: Option<&str>) -> String {
    format!("{}\\{}", domain.unwrap_or(""), user.unwrap_or("")).to_lowercase()
}

/// True when a socket path does not fit in `sun_path` and FM-12 applies (A-17).
#[must_use]
pub fn exceeds_sun_path(path: &Path) -> bool {
    path.as_os_str().as_encoded_bytes().len() > SUN_PATH_MAX_BYTES
}

/// The first [`PIPE_SUFFIX_LENGTH`] hex digits of the SHA-256 of `material`.
fn short_digest(material: &str) -> String {
    let digest = Sha256::digest(material.as_bytes());
    let mut hex = String::with_capacity(PIPE_SUFFIX_LENGTH);
    for byte in digest.iter().take(PIPE_SUFFIX_LENGTH.div_ceil(2)) {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex.truncate(PIPE_SUFFIX_LENGTH);
    hex
}

/// A socket path short enough for `sun_path`, for the home folder whose default does not
/// fit (FM-12).
///
/// The design says the app writes the pointer file and never says where it puts the socket,
/// so this is the app's choice (`DEVIATIONS.md`): `<temp>/handoff-<h>.sock`, `h` being the
/// digest of the path that did not fit — one name per home folder, so two isolated
/// instances do not collide, and short enough that the temporary directory would have to be
/// eighty characters deep to be a problem. If even that does not fit, `/tmp` does.
fn short_socket_path(too_long: &Path, temp: &Path) -> PathBuf {
    let name = format!("handoff-{}.sock", short_digest(&too_long.to_string_lossy()));
    let candidate = temp.join(&name);
    if exceeds_sun_path(&candidate) {
        return Path::new("/tmp").join(name);
    }
    candidate
}

/// What a liveness probe found where the app wants to bind (FM-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleCheck {
    /// Nothing is there.
    Absent,
    /// A socket file was there, nothing answered on it, and it has been removed.
    Removed,
    /// Something answered: another instance of the app is listening.
    Live,
}

/// Removes a socket file left behind by a crash, after a failed liveness connect (FM-12).
///
/// The order matters and is the only one that is safe: a socket file whose owner is gone
/// refuses a connection at once (`ECONNREFUSED`), and one whose owner is alive accepts it.
/// Deleting first and asking later would take the endpoint away from a running instance.
///
/// **Only `ECONNREFUSED` means stale.** A connect can also fail because the listener's
/// backlog is momentarily full (`EAGAIN` on macOS, measured on the CI runner), because the
/// path is not a socket, or because permissions refuse it — and none of those means nobody
/// is there. Every other error is therefore reported as [`StaleCheck::Live`], which makes
/// this instance refuse to start rather than delete the endpoint of an app that is running:
/// a failure to start is visible and recoverable, a stolen endpoint is neither.
///
/// # Errors
///
/// When the file exists, nothing answers on it, and it cannot be removed.
#[cfg(unix)]
pub fn clear_stale_socket(path: &Path) -> io::Result<StaleCheck> {
    use std::os::unix::net::UnixStream;

    if !path.exists() {
        return Ok(StaleCheck::Absent);
    }
    match UnixStream::connect(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(StaleCheck::Absent),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
            std::fs::remove_file(path)?;
            Ok(StaleCheck::Removed)
        }
        Ok(_) => Ok(StaleCheck::Live),
        Err(_) => Ok(StaleCheck::Live),
    }
}

/// Windows has no socket file to go stale: a pipe name exists only while its server does.
///
/// # Errors
///
/// Never; the signature matches the POSIX one so the listener has one call site.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
pub fn clear_stale_socket(_path: &Path) -> io::Result<StaleCheck> {
    Ok(StaleCheck::Absent)
}

/// Creates `~/.handoff/` and `~/.handoff/runbooks/` if they are not there (§7.2).
///
/// On POSIX the folder is narrowed to `0700` whether it was just created or not: the token
/// and the socket live in it, and a folder the installer of an older version left at `0755`
/// would keep them readable for ever.
///
/// # Errors
///
/// When a folder cannot be created, or its mode cannot be set.
pub fn ensure_home(home: &Path) -> io::Result<()> {
    std::fs::create_dir_all(home.join(paths::RUNBOOKS_FOLDER_NAME))?;
    narrow_dir(home)
}

#[cfg(unix)]
fn narrow_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(HOME_MODE))
}

#[cfg(not(unix))]
fn narrow_dir(_dir: &Path) -> io::Result<()> {
    // Windows has no mode bits; the token file carries an explicit owner-only ACL instead
    // (`channel::token`), which is the stronger statement of the two anyway.
    Ok(())
}

/// Narrows a freshly bound socket file to [`SOCKET_MODE`] (§6.2).
///
/// # Errors
///
/// When the mode cannot be set.
#[cfg(unix)]
pub fn narrow_socket(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(SOCKET_MODE))
}

/// Nothing to narrow: on Windows the endpoint is a pipe with a DACL, not a file.
///
/// # Errors
///
/// Never.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
pub fn narrow_socket(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// The explicit DACL the named pipe and the token file are created with (A-16).
#[cfg(windows)]
pub mod security {
    use std::io;
    use std::ptr;

    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// A self-relative security descriptor, owned, with the DACL of [`user_only_sddl`].
    ///
    /// The bytes are held in a `Vec<u64>` rather than a `Vec<u8>` because a security
    /// descriptor has to be aligned and `u8` is aligned to one; the length is kept
    /// separately because the vector is rounded up to whole words.
    ///
    /// It is built once and read by every `CreateNamedPipe` of the accept loop, which is
    /// why it is `Send + Sync`: nothing mutates it after construction, and Windows only
    /// reads it.
    #[derive(Debug)]
    pub struct SecurityDescriptor {
        words: Vec<u64>,
        sddl: String,
    }

    impl SecurityDescriptor {
        /// The descriptor that grants this process's account everything and nobody else
        /// anything.
        ///
        /// # Errors
        ///
        /// When the process token cannot be read, or the SDDL cannot be converted — both of
        /// which mean the pipe must not be created, because it would then carry the default
        /// DACL and A-16 would be silently untrue.
        pub fn user_only() -> io::Result<Self> {
            let sddl = user_only_sddl()?;
            let words = from_sddl(&sddl)?;
            Ok(Self { words, sddl })
        }

        /// The SDDL the descriptor was built from, for the log line and for the tests.
        #[must_use]
        pub fn sddl(&self) -> &str {
            &self.sddl
        }

        /// `SECURITY_ATTRIBUTES` pointing at this descriptor.
        ///
        /// Built fresh per call and never stored: it holds a pointer into `self`, so it
        /// must not outlive the borrow. The handle is never inheritable — a child process
        /// of the app has no business holding the listener.
        pub fn attributes(&mut self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                    .unwrap_or(u32::MAX),
                lpSecurityDescriptor: self.words.as_mut_ptr().cast(),
                bInheritHandle: false.into(),
            }
        }
    }

    /// `D:P(A;;GA;;;<sid>)`: a protected DACL — `P`, so nothing is inherited from anywhere
    /// — with one allow ACE granting `GENERIC_ALL` to the SID of this process's user, and
    /// no other ACE at all. Administrators and `SYSTEM` are deliberately not on the list:
    /// §6.2 says "access to the current user only".
    ///
    /// # Errors
    ///
    /// When the process token or its user SID cannot be read.
    pub fn user_only_sddl() -> io::Result<String> {
        Ok(format!("D:P(A;;GA;;;{})", current_user_sid()?))
    }

    /// The string SID of the account this process runs as, e.g. `S-1-5-21-…-1001`.
    ///
    /// # Errors
    ///
    /// When the process token cannot be opened or queried.
    pub fn current_user_sid() -> io::Result<String> {
        // SAFETY: every call below is checked, and every pointer handed to Windows points
        // at storage that outlives the call. `token` is closed by its own `Drop`.
        unsafe {
            let mut raw = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut raw)
                .map_err(io::Error::other)?;
            let token = OwnedHandle(raw);

            let mut needed = 0_u32;
            // The first call fails with ERROR_INSUFFICIENT_BUFFER and fills `needed`; that
            // is the documented way to size the buffer, so its error is not an error.
            let _ = GetTokenInformation(token.0, TokenUser, None, 0, &raw mut needed);
            if needed == 0 {
                return Err(io::Error::other("the process token reports no user"));
            }

            let mut buffer = vec![0_u64; (needed as usize).div_ceil(8)];
            GetTokenInformation(
                token.0,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                needed,
                &raw mut needed,
            )
            .map_err(io::Error::other)?;

            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut text = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &raw mut text).map_err(io::Error::other)?;
            let sid = text.to_string().map_err(io::Error::other);
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            sid
        }
    }

    /// The self-relative descriptor an SDDL string describes, copied into storage we own.
    ///
    /// Copying rather than keeping the `LocalAlloc`ed pointer is what makes the result an
    /// ordinary `Send + Sync` value: a self-relative descriptor holds offsets, not
    /// pointers, so moving it is defined.
    fn from_sddl(sddl: &str) -> io::Result<Vec<u64>> {
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let mut size = 0_u32;

        // SAFETY: `wide` is NUL-terminated and outlives the call; the descriptor Windows
        // allocates is freed below, after its bytes have been copied out.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                SDDL_REVISION_1,
                &raw mut descriptor,
                Some(&raw mut size),
            )
            .map_err(io::Error::other)?;

            let len = size as usize;
            let mut words = vec![0_u64; len.div_ceil(8)];
            ptr::copy_nonoverlapping(descriptor.0.cast::<u8>(), words.as_mut_ptr().cast(), len);
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            Ok(words)
        }
    }

    /// A process token handle that closes itself.
    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: the handle came from `OpenProcessToken` and is closed once.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_dacl_names_this_process_and_is_protected() {
            let sid = current_user_sid().expect("this process has a user");
            assert!(sid.starts_with("S-1-"), "{sid} is not a string SID");
            let sddl = user_only_sddl().expect("the SDDL is built");
            // `P` is what stops an inherited ACE from widening the pipe, and one ACE is
            // what "the current user only" means. A regression that dropped either would
            // still create a working pipe, which is why it is asserted rather than trusted.
            assert_eq!(sddl, format!("D:P(A;;GA;;;{sid})"));
            assert_eq!(sddl.matches("(A;").count(), 1);
        }

        #[test]
        fn the_descriptor_is_built_and_is_not_empty() {
            let mut descriptor = SecurityDescriptor::user_only().expect("it is built");
            assert!(descriptor.sddl().starts_with("D:P("));
            let attributes = descriptor.attributes();
            assert!(!attributes.lpSecurityDescriptor.is_null());
            assert!(!attributes.bInheritHandle.as_bool());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Windows user, as the two variables the pipe name is derived from carry it.
    const DOMAIN: &str = "ACME";
    const USER: &str = "Giuse";
    const POSIX_HOME: &str = "/tmp/handoff-test";
    const WINDOWS_HOME: &str = r"C:\tmp\hh";

    // The three digests `handoff-mcp` pinned in `test/unit/platform/paths.test.ts`. They
    // are literals on both sides on purpose: a test that hashes the same string with the
    // same algorithm agrees with any change, including a wrong one.
    const PIPE_HASH: &str = "8fb9ebc1757c4335";
    const PIPE_HASH_WITH_POSIX_HOME: &str = "020dcfd9686704a6";
    const PIPE_HASH_WITH_WINDOWS_HOME: &str = "70e80b37886e24a4";

    #[test]
    fn the_user_key_is_the_lower_cased_domain_and_name() {
        assert_eq!(user_key_from(Some(DOMAIN), Some(USER)), r"acme\giuse");
        assert_eq!(user_key_from(Some("acme"), Some("giuse")), r"acme\giuse");
    }

    #[test]
    fn an_unset_variable_contributes_an_empty_string_and_not_a_substitute() {
        // The server does the same. Anything else — the OS user database, a default —
        // would move the endpoint out from under the peer that did not do it (§5.8).
        assert_eq!(user_key_from(None, Some("giuse")), r"\giuse");
        assert_eq!(user_key_from(Some("acme"), None), r"acme\");
        assert_eq!(user_key_from(None, None), r"\");
    }

    #[test]
    fn the_pipe_digest_reproduces_the_values_the_server_pinned() {
        assert_eq!(pipe_suffix(r"acme\giuse", None), PIPE_HASH);
        assert_eq!(
            pipe_suffix(r"acme\giuse", Some(POSIX_HOME)),
            PIPE_HASH_WITH_POSIX_HOME
        );
        assert_eq!(
            pipe_suffix(r"acme\giuse", Some(WINDOWS_HOME)),
            PIPE_HASH_WITH_WINDOWS_HOME
        );
    }

    #[test]
    fn the_pipe_name_is_the_prefix_and_sixteen_hex_digits() {
        let name = pipe_name(r"acme\giuse", None);
        assert_eq!(name, format!(r"\\.\pipe\handoff-{PIPE_HASH}"));
        let suffix = pipe_suffix(r"acme\giuse", None);
        assert_eq!(suffix.len(), PIPE_SUFFIX_LENGTH);
        assert!(suffix
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }

    #[test]
    fn a_different_user_gets_a_different_pipe_which_is_what_it_is_for() {
        assert_ne!(
            pipe_name(r"acme\giuse", None),
            pipe_name(r"acme\other", None)
        );
        assert_ne!(
            pipe_name(r"acme\giuse", None),
            pipe_name(r"acme\giuse", Some(POSIX_HOME))
        );
    }

    #[test]
    fn the_sun_path_limit_is_measured_in_bytes_and_not_in_characters() {
        let short = format!("/{}", "a".repeat(SUN_PATH_MAX_BYTES - 1));
        assert_eq!(short.chars().count(), SUN_PATH_MAX_BYTES);
        assert!(!exceeds_sun_path(Path::new(&short)));
        // The same number of characters, one of which costs two bytes.
        let accented = format!("é{}", &short[1..]);
        assert_eq!(accented.chars().count(), SUN_PATH_MAX_BYTES);
        assert!(exceeds_sun_path(Path::new(&accented)));
    }

    #[test]
    fn the_limit_is_exceeded_only_above_it() {
        assert!(!exceeds_sun_path(Path::new(
            &"a".repeat(SUN_PATH_MAX_BYTES)
        )));
        assert!(exceeds_sun_path(Path::new(
            &"a".repeat(SUN_PATH_MAX_BYTES + 1)
        )));
    }

    #[test]
    fn windows_gets_the_pipe_and_reads_no_file() {
        let endpoint = resolve_for(
            Platform::Windows,
            Path::new(r"C:\tmp\hh"),
            r"acme\giuse",
            Some(WINDOWS_HOME),
            Path::new(r"C:\tmp"),
        );
        assert_eq!(
            endpoint,
            Endpoint::Pipe {
                name: format!(r"\\.\pipe\handoff-{PIPE_HASH_WITH_WINDOWS_HOME}"),
            }
        );
        assert_eq!(endpoint.pointer(), None);
    }

    #[test]
    fn posix_gets_the_default_socket_and_no_pointer_file_while_it_fits() {
        let endpoint = resolve_for(
            Platform::Posix,
            Path::new(POSIX_HOME),
            r"acme\giuse",
            Some(POSIX_HOME),
            Path::new("/var/tmp"),
        );
        assert_eq!(
            endpoint,
            Endpoint::Unix {
                path: Path::new(POSIX_HOME).join("app.sock"),
                pointer: None,
            }
        );
    }

    #[test]
    fn a_home_too_deep_moves_the_socket_and_writes_the_pointer_file() {
        let deep = PathBuf::from(format!("/tmp/{}", "d".repeat(SUN_PATH_MAX_BYTES)));
        let endpoint = resolve_for(
            Platform::Posix,
            &deep,
            r"acme\giuse",
            None,
            Path::new("/var/tmp"),
        );
        let Endpoint::Unix { path, pointer } = endpoint else {
            panic!("a POSIX host binds a socket");
        };
        // The whole point of the fallback: the path the app binds fits, and the server is
        // told where it is by the pointer file the design names (FM-12).
        assert!(!exceeds_sun_path(&path));
        assert!(path.starts_with("/var/tmp"));
        assert_eq!(pointer, Some(deep.join("app.sock.path")));
    }

    #[test]
    fn a_temporary_directory_too_deep_falls_back_to_tmp() {
        let deep = PathBuf::from(format!("/tmp/{}", "d".repeat(SUN_PATH_MAX_BYTES)));
        let endpoint = resolve_for(
            Platform::Posix,
            &deep,
            r"acme\giuse",
            None,
            Path::new(&format!("/var/{}", "t".repeat(SUN_PATH_MAX_BYTES))),
        );
        let Endpoint::Unix { path, .. } = endpoint else {
            panic!("a POSIX host binds a socket");
        };
        assert!(path.starts_with("/tmp"));
        assert!(!exceeds_sun_path(&path));
    }

    #[test]
    fn two_home_folders_that_do_not_fit_get_two_different_sockets() {
        let one = PathBuf::from(format!("/tmp/{}", "a".repeat(SUN_PATH_MAX_BYTES)));
        let two = PathBuf::from(format!("/tmp/{}", "b".repeat(SUN_PATH_MAX_BYTES)));
        let of =
            |home: &Path| match resolve_for(Platform::Posix, home, "", None, Path::new("/var/tmp"))
            {
                Endpoint::Unix { path, .. } => path,
                Endpoint::Pipe { .. } => panic!("a POSIX host binds a socket"),
            };
        assert_ne!(of(&one), of(&two));
    }
}
