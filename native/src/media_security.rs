//! Linux socket policy for a dedicated decoder thread and all its descendants.
//! This is not a general process/filesystem sandbox. A public-only HTTP proxy
//! must resolve names outside this filter; decoder-side private DNS is denied.
use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use crate::media_addresses as addresses;
pub use addresses::public_ip;

#[repr(C)]
#[derive(Default)]
struct Data {
    nr: i32,
    arch: u32,
    instruction_pointer: u64,
    args: [u64; 6],
}
#[repr(C)]
#[derive(Default)]
struct Notification {
    id: u64,
    pid: u32,
    flags: u32,
    data: Data,
}
#[repr(C)]
struct Response {
    id: u64,
    value: i64,
    error: i32,
    flags: u32,
}

const RECV: libc::c_ulong = 0xc0502100;
const SEND: libc::c_ulong = 0xc0182101;
const VALID: libc::c_ulong = 0x80082102;

pub struct NetworkGuard {
    stop: Arc<AtomicBool>,
    broker: Option<thread::JoinHandle<()>>,
}
impl Drop for NetworkGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(broker) = self.broker.take() {
            let _ = broker.join();
        }
    }
}
fn instruction(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

impl NetworkGuard {
    /// Must run on a disposable decoder thread, never on Godot's XR thread.
    /// Its seccomp filter cannot be removed and is inherited by fork/exec.
    #[cfg(target_arch = "x86_64")]
    pub fn install(proxy: SocketAddr, fixture: Option<SocketAddr>) -> io::Result<Self> {
        const DENY: u32 = 0x00050000 | libc::EACCES as u32;
        const ALLOW: u32 = 0x7fff0000;
        let mut code = vec![
            instruction(0x20, 4, 0, 0), // audit architecture
            instruction(0x15, 0xc000003e, 1, 0),
            instruction(0x06, DENY, 0, 0),
            instruction(0x20, 0, 0, 0),          // syscall number
            instruction(0x35, 0x40000000, 0, 1), // deny x32 ABI bypass
            instruction(0x06, DENY, 0, 0),
            instruction(0x15, libc::SYS_connect as u32, 0, 1),
            instruction(0x06, 0x7fc00000, 0, 0), // USER_NOTIF
        ];
        for syscall in [
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_sendmsg,
            libc::SYS_sendmmsg,
            libc::SYS_io_uring_setup,
            libc::SYS_io_uring_enter,
            libc::SYS_io_uring_register,
        ] {
            code.push(instruction(0x15, syscall as u32, 0, 1));
            code.push(instruction(0x06, DENY, 0, 0));
        }
        code.extend([
            instruction(0x15, libc::SYS_sendto as u32, 1, 0),
            instruction(0x06, ALLOW, 0, 0),
            instruction(0x20, 48, 0, 0), // sendto destination pointer low word
            instruction(0x15, 0, 1, 0),
            instruction(0x06, DENY, 0, 0),
            instruction(0x20, 52, 0, 0), // and high word; NULL permits connected TCP
            instruction(0x15, 0, 1, 0),
            instruction(0x06, DENY, 0, 0),
            instruction(0x06, ALLOW, 0, 0),
        ]);
        let program = libc::sock_fprog {
            len: code.len() as u16,
            filter: code.as_mut_ptr(),
        };
        // SAFETY: the kernel copies this valid BPF array during the call. No
        // TSYNC flag is used, so unrelated Godot/network threads are unaffected.
        let raw = unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            libc::syscall(libc::SYS_seccomp, 1, 8, &program)
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: NEW_LISTENER returns a newly owned file descriptor.
        let listener = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let broker = thread::Builder::new()
            .name("media-socket-policy".into())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    let mut poll = libc::pollfd {
                        fd: listener.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    // SAFETY: one valid pollfd, bounded wait; one broker owns recv.
                    if unsafe { libc::poll(&mut poll, 1, 100) } <= 0 {
                        continue;
                    }
                    let mut request = Notification::default();
                    if unsafe { libc::ioctl(listener.as_raw_fd(), RECV, &mut request) } != 0 {
                        continue;
                    }
                    let allowed = socket_address(&request).is_some_and(|address| {
                        address == proxy || fixture == Some(address) || public_ip(address.ip())
                    });
                    let response = Response {
                        id: request.id,
                        value: 0,
                        error: if allowed { 0 } else { -libc::EACCES },
                        flags: u32::from(allowed),
                    };
                    // CONTINUE allows the kernel to perform the actual connect;
                    // no other socket is substituted and the destination is an IP,
                    // so there is no second DNS lookup/rebinding opportunity.
                    if unsafe { libc::ioctl(listener.as_raw_fd(), VALID, &request.id) } == 0 {
                        unsafe { libc::ioctl(listener.as_raw_fd(), SEND, &response) };
                    }
                }
            })?;
        Ok(Self {
            stop,
            broker: Some(broker),
        })
    }
}

fn socket_address(request: &Notification) -> Option<SocketAddr> {
    let length = usize::try_from(request.data.args[2]).ok()?;
    if !(2..=128).contains(&length) {
        return None;
    }
    let mut bytes = [0u8; 128];
    let local = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: length,
    };
    let remote = libc::iovec {
        iov_base: request.data.args[1] as *mut libc::c_void,
        iov_len: length,
    };
    // A fork/exec resolver has a separate address space; process_vm_readv is
    // also checked by the kernel's normal same-user/parent ptrace permissions.
    let read = unsafe { libc::process_vm_readv(request.pid as i32, &local, 1, &remote, 1, 0) };
    if read != length as isize {
        return None;
    }
    let family = u16::from_ne_bytes([bytes[0], bytes[1]]) as i32;
    match family {
        libc::AF_INET if length >= std::mem::size_of::<libc::sockaddr_in>() => {
            let address =
                unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<libc::sockaddr_in>()) };
            Some(SocketAddr::new(
                Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()).into(),
                u16::from_be(address.sin_port),
            ))
        }
        libc::AF_INET6 if length >= std::mem::size_of::<libc::sockaddr_in6>() => {
            let address =
                unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<libc::sockaddr_in6>()) };
            Some(SocketAddr::new(
                Ipv6Addr::from(address.sin6_addr.s6_addr).into(),
                u16::from_be(address.sin6_port),
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::{TcpListener, TcpStream},
        time::Duration,
    };
    #[test]
    fn address_policy() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.2",
            "203.0.113.1",
            "::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "2001:2::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "1.1.1.1",
            "8.8.8.8",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
    #[test]
    fn thread_and_exec_descendants_are_guarded() {
        let allowed = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = allowed.local_addr().unwrap();
        let denied = TcpListener::bind("127.0.0.1:0").unwrap();
        let private = denied.local_addr().unwrap();
        thread::spawn(move || {
            let _guard = NetworkGuard::install(address, None)
                .expect("Linux seccomp user notifications are required");
            assert!(TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_ok());
            assert_eq!(
                TcpStream::connect_timeout(&private, Duration::from_secs(1))
                    .unwrap_err()
                    .raw_os_error(),
                Some(libc::EACCES)
            );
            assert_eq!(
                TcpListener::bind("127.0.0.1:0").unwrap_err().raw_os_error(),
                Some(libc::EACCES)
            );
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "media_security::tests::child_socket_probe",
                    "--nocapture",
                ])
                .env("SSC_GUARD_ALLOWED", address.to_string())
                .env("SSC_GUARD_DENIED", private.to_string())
                .output()
                .unwrap();
            assert!(
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        })
        .join()
        .unwrap();
        // The caller and unrelated Godot threads retain ordinary networking.
        assert!(TcpStream::connect_timeout(&private, Duration::from_secs(1)).is_ok());
    }
    #[test]
    #[ignore = "launched by the guarded parent to verify exec inheritance"]
    fn child_socket_probe() {
        let allowed: SocketAddr = std::env::var("SSC_GUARD_ALLOWED").unwrap().parse().unwrap();
        let denied: SocketAddr = std::env::var("SSC_GUARD_DENIED").unwrap().parse().unwrap();
        assert!(TcpStream::connect_timeout(&allowed, Duration::from_secs(1)).is_ok());
        assert_eq!(
            TcpStream::connect_timeout(&denied, Duration::from_secs(1))
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EACCES)
        );
    }
}
