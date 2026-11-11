//! Linux worker-only filesystem/resource boundary. Never call in Godot.
use std::{
    ffi::CString,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

const EXECUTE: u64 = 1;
const WRITE: u64 = 2;
const READ: u64 = 4;
const READ_DIR: u64 = 8;

#[repr(C)]
struct Ruleset {
    handled: u64,
    network: u64,
    scoped: u64,
}
#[repr(C, packed)]
struct PathRule {
    allowed: u64,
    parent: i32,
}

pub fn install() -> io::Result<()> {
    // A decoder allocation failure kills only this disposable process. These
    // limits are inherited by resolver descendants, not the Godot client.
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            || libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0
        {
            return Err(io::Error::last_os_error());
        }
        for (resource, limit) in [
            (libc::RLIMIT_AS, 2 * 1024 * 1024 * 1024),
            (libc::RLIMIT_CORE, 0),
            (libc::RLIMIT_FSIZE, 0),
            (libc::RLIMIT_NOFILE, 128),
        ] {
            let limits = libc::rlimit {
                rlim_cur: limit,
                rlim_max: limit,
            };
            if libc::setrlimit(resource, &limits) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<u8>(),
            0,
            1,
        )
    };
    if abi < 6 {
        return Err(io::Error::other(
            "Media isolation requires Landlock ABI 6 or newer (Linux 6.12+)",
        ));
    }
    // ABI 3 includes cross-directory REFER and file TRUNCATE. All handled
    // operations are denied except explicit read/execute and pipe-device paths.
    let rules = Ruleset {
        handled: (1 << 15) - 1,
        network: 0,
        scoped: 3,
    };
    let raw = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &rules,
            std::mem::size_of::<Ruleset>(),
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let rules = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    for (path, access) in [
        ("/nix/store", READ | READ_DIR | EXECUTE),
        ("/etc/resolv.conf", READ),
        ("/etc/hosts", READ),
        ("/etc/nsswitch.conf", READ),
        ("/etc/ssl/certs", READ | READ_DIR),
        ("/etc/pki/tls/certs", READ | READ_DIR),
        ("/etc/localtime", READ),
        ("/proc/cpuinfo", READ),
        ("/proc/self/fd", READ | READ_DIR | WRITE),
        ("/dev/null", READ | WRITE),
        ("/dev/urandom", READ),
        ("/sys/devices/system/cpu", READ | READ_DIR),
    ] {
        let name = CString::new(path).unwrap();
        let raw = unsafe { libc::open(name.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if raw < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
                continue;
            }
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let rule = PathRule {
            allowed: access,
            parent: fd.as_raw_fd(),
        };
        if unsafe { libc::syscall(libc::SYS_landlock_add_rule, rules.as_raw_fd(), 1, &rule, 0) } < 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, rules.as_raw_fd(), 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // Signal scope prevents killing Godot/other user processes; deny alternate
    // process-memory/descriptor inspection and namespace/privileged escape APIs.
    let instruction = |code, k, jt, jf| libc::sock_filter { code, k, jt, jf };
    let deny = 0x00050000 | libc::EACCES as u32;
    let mut code = vec![
        instruction(0x20, 4, 0, 0),
        instruction(0x15, 0xc000003e, 1, 0),
        instruction(0x06, deny, 0, 0),
        instruction(0x20, 0, 0, 0),
        instruction(0x35, 0x40000000, 0, 1),
        instruction(0x06, deny, 0, 0),
    ];
    for syscall in [
        libc::SYS_ptrace,
        libc::SYS_setpgid,
        libc::SYS_setsid,
        libc::SYS_process_vm_writev,
        libc::SYS_pidfd_getfd,
        libc::SYS_kcmp,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_setns,
        libc::SYS_unshare,
        libc::SYS_bpf,
        libc::SYS_userfaultfd,
        libc::SYS_perf_event_open,
        libc::SYS_open_by_handle_at,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
    ] {
        code.push(instruction(0x15, syscall as u32, 0, 1));
        code.push(instruction(0x06, deny, 0, 0));
    }
    code.push(instruction(0x06, 0x7fff0000, 0, 0));
    let program = libc::sock_fprog {
        len: code.len() as u16,
        filter: code.as_mut_ptr(),
    };
    if unsafe { libc::syscall(libc::SYS_seccomp, 1, 0, &program) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn filesystem_signal_and_memory_limits_are_inherited_in_isolated_process() {
        let directory = tempfile::tempdir().unwrap();
        let canary = directory.path().join("private-canary");
        std::fs::write(&canary, "must not read").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process_sandbox::tests::sandbox_child",
                "--ignored",
                "--nocapture",
            ])
            .env("SSC_SANDBOX_CANARY", &canary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read_to_string(canary).unwrap(), "must not read");
    }
    #[test]
    #[ignore = "isolated process: installs irreversible Landlock and resource limits"]
    fn sandbox_child() {
        let canary = std::env::var_os("SSC_SANDBOX_CANARY").unwrap();
        super::install().unwrap();
        assert!(std::fs::read(&canary).is_err());
        assert!(std::fs::write(&canary, "overwritten").is_err());
        assert!(std::fs::read("/etc/hosts").is_ok());
        assert_eq!(unsafe { libc::kill(libc::getppid(), 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        );
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limit) }, 0);
        assert_eq!(limit.rlim_max, 2 * 1024 * 1024 * 1024);
        for syscall in [libc::SYS_setsid, libc::SYS_setpgid] {
            assert_eq!(unsafe { libc::syscall(syscall, 0, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EACCES)
            );
        }
        // The socket-notification broker needs readv of its own sandboxed
        // descendants. Landlock's ptrace domain rule must still deny the parent.
        let mut byte = 0u8;
        let local = libc::iovec {
            iov_base: (&mut byte as *mut u8).cast(),
            iov_len: 1,
        };
        let remote = libc::iovec {
            iov_base: (&mut byte as *mut u8).cast(),
            iov_len: 1,
        };
        assert_eq!(
            unsafe { libc::process_vm_readv(libc::getppid(), &local, 1, &remote, 1, 0) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        );
    }
}
