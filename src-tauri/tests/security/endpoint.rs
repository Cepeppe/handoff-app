//! The endpoint and the token file belong to this user alone (§6.2, §11.7, SRV-04, SRV-07,
//! A-16, R-09).
//!
//! **Windows.** The pipe and the token file are created with the descriptor
//! `channel::endpoint::security` builds, and this module reads the DACL **back from the
//! objects**: the ACE list Windows actually attached, not the SDDL string the app handed it.
//! The unit tests of `endpoint.rs` prove the string; only a read-back proves the pipe got
//! it. A `CreateNamedPipe` handed no attributes by mistake still makes a working pipe, with
//! the default DACL, which grants read access to Everyone — and that default pipe is the
//! control here: created on purpose, it has to fail the same check.
//!
//! **POSIX** (the macOS leg of CI): the socket is `0600`, the folder it lives in `0700`, the
//! token `0600`.
//!
//! Whether a second account is really kept out needs a second account, which a test runner
//! does not have: that is the manual second-user test of T-057 (T-061 on macOS).

#[cfg(windows)]
mod on_windows {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::AsRawHandle as _;
    use std::path::Path;
    use std::time::Duration;

    use serde_json::json;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, ServerOptions};
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
        GetNamedSecurityInfoW, GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT, SE_KERNEL_OBJECT,
    };
    use windows::Win32::Security::{
        AclSizeInformation, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    };
    use windows::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    use handoff_app_lib::channel::endpoint::security::current_user_sid;
    use handoff_app_lib::channel::listener::{listen, ListenerConfig};
    use handoff_app_lib::channel::token::{self, Token};
    use handoff_app_lib::channel::Endpoint;

    use crate::report;
    use crate::support::{private_endpoint, TempDir};

    /// `ACCESS_ALLOWED_ACE_TYPE` (winnt.h): the only kind of ACE the DACL of §6.2 holds.
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

    /// `INHERITED_ACE` (winnt.h): an ACE that came from a parent, which a protected DACL of
    /// its own has none of.
    const INHERITED_ACE: u8 = 0x10;

    /// One ACE, as it was read back.
    struct Ace {
        kind: u8,
        flags: u8,
        mask: u32,
        /// The trustee, for an allow ACE.
        sid: Option<String>,
    }

    /// A DACL, as it was read back from an object.
    struct Dacl {
        protected: bool,
        aces: Vec<Ace>,
        sddl: String,
    }

    impl Dacl {
        /// Why this is not "access to the current user only" (§6.2), or nothing when it is.
        ///
        /// `GENERIC_ALL` is what the app asks for (`D:P(A;;GA;;;<sid>)`); Windows maps a
        /// generic right to the object's own rights when it creates the object, so what is
        /// read back from a pipe or a file is `FILE_ALL_ACCESS`.
        fn problems(&self, user: &str) -> Vec<String> {
            let mut problems = Vec::new();
            if !self.protected {
                problems.push("the DACL is not protected, so an inherited ACE can widen it".into());
            }
            if self.aces.len() != 1 {
                problems.push(format!("{} ACEs where §6.2 allows one", self.aces.len()));
            }
            for ace in &self.aces {
                if ace.kind != ACCESS_ALLOWED_ACE_TYPE {
                    problems.push(format!(
                        "an ACE of type {} where only an allow ACE belongs",
                        ace.kind
                    ));
                    continue;
                }
                if ace.flags & INHERITED_ACE != 0 {
                    problems.push("an inherited ACE".into());
                }
                if ace.sid.as_deref() != Some(user) {
                    problems.push(format!(
                        "an ACE for {} rather than for this user",
                        ace.sid.as_deref().unwrap_or("nobody")
                    ));
                }
                if ace.mask != FILE_ALL_ACCESS.0 {
                    problems.push(format!(
                        "an access mask of {:#010x} where {:#010x} is expected",
                        ace.mask, FILE_ALL_ACCESS.0
                    ));
                }
            }
            problems
        }

        /// The ACE list for the report, with this user's SID written as `<this user>`: the
        /// report leaves the machine as a CI artifact, and an account's SID is not its to
        /// publish.
        fn describe(&self, user: &str) -> Vec<String> {
            self.aces
                .iter()
                .map(|ace| {
                    let trustee = match ace.sid.as_deref() {
                        Some(sid) if sid == user => "<this user>",
                        Some(sid) => sid,
                        None => "<not an allow ACE>",
                    };
                    format!(
                        "type {} flags {:#04x} mask {:#010x} for {trustee}",
                        ace.kind, ace.flags, ace.mask
                    )
                })
                .collect()
        }
    }

    /// The DACL of the object behind an open handle.
    ///
    /// Asked as a kernel object — the descriptor the object itself carries, which is the
    /// question — rather than as a file: the file path of `GetSecurityInfo` answered
    /// `ERROR_INVALID_PARAMETER` for the pipe with the default DACL below, the one object
    /// the control needs to read.
    fn dacl_of_handle(handle: HANDLE) -> Dacl {
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: the handle is open for the duration of the call, and both out-pointers
        // point at locals that outlive it.
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&raw mut dacl),
                None,
                Some(&raw mut descriptor),
            )
        };
        assert_eq!(status, ERROR_SUCCESS, "GetSecurityInfo refused the handle");
        read(descriptor, dacl)
    }

    /// The DACL of a file, by its path.
    fn dacl_of_path(path: &Path) -> Dacl {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `wide` is NUL-terminated and outlives the call; both out-pointers point at
        // locals that outlive it.
        let status = unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&raw mut dacl),
                None,
                &raw mut descriptor,
            )
        };
        assert_eq!(
            status,
            ERROR_SUCCESS,
            "GetNamedSecurityInfoW refused {}",
            path.display()
        );
        read(descriptor, dacl)
    }

    /// The control flags, the ACEs and the SDDL of `descriptor`, which is then freed; `dacl`
    /// points inside it, as `Get*SecurityInfo` returns them.
    fn read(descriptor: PSECURITY_DESCRIPTOR, dacl: *mut ACL) -> Dacl {
        // SAFETY: `descriptor` and `dacl` come from one successful `Get*SecurityInfo` call
        // and are read here before the descriptor, which owns both, is freed at the end. Each
        // ACE pointer comes from `GetAce` on that DACL and is read as the type its header
        // names. Every string Windows allocates is copied out before it is freed.
        unsafe {
            let mut control = 0_u16;
            let mut revision = 0_u32;
            GetSecurityDescriptorControl(descriptor, &raw mut control, &raw mut revision)
                .expect("the control flags of the descriptor");

            let mut aces = Vec::new();
            if !dacl.is_null() {
                let mut size = ACL_SIZE_INFORMATION::default();
                GetAclInformation(
                    dacl,
                    (&raw mut size).cast(),
                    u32::try_from(size_of::<ACL_SIZE_INFORMATION>()).expect("a small struct"),
                    AclSizeInformation,
                )
                .expect("the size of the DACL");
                for index in 0..size.AceCount {
                    let mut ace: *mut c_void = std::ptr::null_mut();
                    GetAce(dacl, index, &raw mut ace).expect("an ACE of the DACL");
                    let header = *ace.cast::<ACE_HEADER>();
                    let (mask, sid) = if header.AceType == ACCESS_ALLOWED_ACE_TYPE {
                        let allowed = ace.cast::<ACCESS_ALLOWED_ACE>();
                        let sid = PSID((&raw mut (*allowed).SidStart).cast());
                        ((*allowed).Mask, Some(sid_string(sid)))
                    } else {
                        (0, None)
                    };
                    aces.push(Ace {
                        kind: header.AceType,
                        flags: header.AceFlags,
                        mask,
                        sid,
                    });
                }
            }

            let mut text = PWSTR::null();
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &raw mut text,
                None,
            )
            .expect("the SDDL of the descriptor");
            let sddl = text.to_string().unwrap_or_default();
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));

            Dacl {
                protected: control & SE_DACL_PROTECTED.0 != 0,
                aces,
                sddl,
            }
        }
    }

    /// `S-1-5-21-…` for a SID inside a DACL.
    fn sid_string(sid: PSID) -> String {
        let mut text = PWSTR::null();
        // SAFETY: `sid` points inside a live DACL (see `read`); the string Windows allocates
        // is copied out before it is freed.
        unsafe {
            ConvertSidToStringSidW(sid, &raw mut text).expect("a string SID");
            let sid = text.to_string().unwrap_or_default();
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            sid
        }
    }

    /// A client handle on `name`, retrying while the accept loop has no instance free (the
    /// T-031 rule: `ERROR_FILE_NOT_FOUND` or `ERROR_PIPE_BUSY` means "not yet").
    async fn open(name: &str) -> NamedPipeClient {
        for _ in 0..500 {
            match ClientOptions::new().open(name) {
                Ok(client) => return client,
                Err(error) if matches!(error.raw_os_error(), Some(2 | 231)) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("{name}: {error}"),
            }
        }
        panic!("{name} never had a free instance");
    }

    fn pipe_name(endpoint: &Endpoint) -> String {
        match endpoint {
            Endpoint::Pipe { name } => name.clone(),
            Endpoint::Unix { .. } => panic!("Windows listens on a pipe"),
        }
    }

    #[tokio::test]
    async fn the_pipe_carries_a_protected_dacl_for_this_user_alone() {
        let dir = TempDir::new("pipe");
        let endpoint = private_endpoint(dir.path());
        let (handle, _events) = listen(ListenerConfig::new(endpoint.clone(), Token::generate()))
            .await
            .expect("the listener binds");
        // Read through a client's handle: it names the same pipe object, and a client is
        // what an intruder would be.
        let client = open(&pipe_name(&endpoint)).await;
        let dacl = dacl_of_handle(HANDLE(client.as_raw_handle()));
        drop(client);
        handle.shutdown("the security suite is done").await;

        let user = current_user_sid().expect("this process has a user");
        let problems = dacl.problems(&user);
        report::record(
            "pipe_dacl",
            json!({
                "status": report::status(problems.is_empty()),
                "protected": dacl.protected,
                "aces": dacl.describe(&user),
                "problems": problems,
            }),
        );
        assert!(
            problems.is_empty(),
            "the pipe's DACL is not this user's alone: {problems:#?}\nread back: {}",
            dacl.sddl
        );
    }

    #[test]
    fn the_token_file_carries_the_same_dacl() {
        let dir = TempDir::new("tokfile");
        let path = dir.path().join("channel.token");
        token::ensure(&path).expect("the token is written");
        let dacl = dacl_of_path(&path);

        let user = current_user_sid().expect("this process has a user");
        let problems = dacl.problems(&user);
        report::record(
            "token_file_dacl",
            json!({
                "status": report::status(problems.is_empty()),
                "protected": dacl.protected,
                "aces": dacl.describe(&user),
                "problems": problems,
            }),
        );
        assert!(
            problems.is_empty(),
            "the token file's DACL is not this user's alone: {problems:#?}\nread back: {}",
            dacl.sddl
        );
    }

    #[tokio::test]
    async fn a_pipe_with_the_default_dacl_fails_the_same_check() {
        // The control. The descriptor a pipe gets when nobody hands it one grants the
        // creator, SYSTEM and the administrators full control and Everyone read access; if
        // the check above passed this too, passing it would prove nothing.
        let dir = TempDir::new("default");
        let name = pipe_name(&private_endpoint(dir.path()));
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .expect("a pipe with the default descriptor");
        let client = open(&name).await;
        let dacl = dacl_of_handle(HANDLE(client.as_raw_handle()));
        drop(client);
        drop(server);

        let user = current_user_sid().expect("this process has a user");
        let problems = dacl.problems(&user);
        assert!(
            !problems.is_empty(),
            "a pipe with the default descriptor passed the owner-only check, so passing it \
             proves nothing: {}",
            dacl.sddl
        );
        println!("the default DACL fails the check, as it must: {problems:?}");
    }
}

#[cfg(unix)]
mod on_posix {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;

    use serde_json::json;

    use handoff_app_lib::channel::endpoint::{HOME_MODE, SOCKET_MODE};
    use handoff_app_lib::channel::listener::{listen, ListenerConfig};
    use handoff_app_lib::channel::token::{self, Token, TOKEN_MODE};
    use handoff_app_lib::channel::{ensure_home, Endpoint};

    use crate::report;
    use crate::support::TempDir;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            .permissions()
            .mode()
            & 0o777
    }

    #[tokio::test]
    async fn the_socket_its_folder_and_the_token_are_owner_only() {
        let dir = TempDir::new("modes");
        let home = dir.path().join("h");
        ensure_home(&home).expect("the home folder is created");
        let socket = home.join("app.sock");
        let (handle, _events) = listen(ListenerConfig::new(
            Endpoint::Unix {
                path: socket.clone(),
                pointer: None,
            },
            Token::generate(),
        ))
        .await
        .expect("the listener binds");
        let token_file = home.join("channel.token");
        token::ensure(&token_file).expect("the token is written");

        let found = [
            ("socket", mode(&socket), SOCKET_MODE),
            ("folder", mode(&home), HOME_MODE),
            ("token", mode(&token_file), TOKEN_MODE),
        ];
        handle.shutdown("the security suite is done").await;

        let wrong: Vec<String> = found
            .iter()
            .filter(|(_, mode, expected)| mode != expected)
            .map(|(what, mode, expected)| format!("the {what} is {mode:o} where {expected:o}"))
            .collect();
        report::record(
            "posix_modes",
            json!({
                "status": report::status(wrong.is_empty()),
                "socket": format!("{:o}", found[0].1),
                "folder": format!("{:o}", found[1].1),
                "token": format!("{:o}", found[2].1),
                "problems": wrong,
            }),
        );
        assert!(wrong.is_empty(), "{wrong:#?}");
    }
}
