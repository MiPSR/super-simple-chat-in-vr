//! Bounded IPC between Godot's relay thread and a disposable libmpv process.
use super::*;
use std::{
    io::{self, Write},
    os::fd::OwnedFd,
    os::unix::{net::UnixStream, process::CommandExt},
    process::{Child, Stdio},
    sync::mpsc::Receiver,
};

struct ChildGuard(Child);
fn parent_death_guard(parent: libc::pid_t) -> io::Result<()> {
    // Only async-signal-safe syscalls and raw OS errors: used after fork.
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // Parent death before prctl is not delivered retroactively. Do not start an
    // orphan decoder if the client disappeared in the fork-to-exec interval.
    if unsafe { libc::getppid() } != parent {
        return Err(io::Error::from_raw_os_error(libc::ECHILD));
    }
    Ok(())
}
struct Exchange<'a> {
    socket: &'a mut UnixStream,
    deadline: std::time::Instant,
}
impl Read for Exchange<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Isolated decoder response deadline exceeded",
                )
            })?;
        self.socket.set_read_timeout(Some(remaining))?;
        self.socket.read(buffer)
    }
}
impl Write for Exchange<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Isolated decoder request deadline exceeded",
                )
            })?;
        self.socket.set_write_timeout(Some(remaining))?;
        self.socket.write(buffer)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        // The worker and yt-dlp/timeout descendants share a private process group.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}

pub(super) fn run(
    width: i32,
    height: i32,
    receiver: &Receiver<Command>,
    stop: &AtomicBool,
    shared: &Mutex<Output>,
) -> io::Result<()> {
    let executable = std::env::var_os("SSC_MEDIA_WORKER").ok_or_else(|| {
        io::Error::other(
            "SSC_MEDIA_WORKER is missing; launch with the packaged client or Nix development shell",
        )
    })?;
    let (mut socket, child_socket) = UnixStream::pair()?;
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    socket.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut process = std::process::Command::new(executable);
    process
        .args([width.to_string(), height.to_string()])
        .env_clear()
        .env("HOME", "/nonexistent")
        .env("PATH", "")
        .stdin(Stdio::from(OwnedFd::from(child_socket.try_clone()?)))
        .stdout(Stdio::from(OwnedFd::from(child_socket)))
        .stderr(Stdio::null())
        .process_group(0);
    for name in [
        "SSC_YTDLP",
        "SSL_CERT_FILE",
        "NIX_SSL_CERT_FILE",
        "SSC_TEST_MEDIA_ORIGIN",
    ] {
        if let Some(value) = std::env::var_os(name) {
            process.env(name, value);
        }
    }
    let parent = std::process::id() as libc::pid_t;
    unsafe {
        process.pre_exec(move || {
            // Keep only mapped standard descriptors across exec, including any
            // Godot-owned descriptors that did not have CLOEXEC set originally.
            if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32) < 0 {
                return Err(io::Error::last_os_error());
            }
            parent_death_guard(parent)
        });
    }
    let _child = ChildGuard(process.spawn()?);
    while !stop.load(Ordering::Acquire) {
        let mut exchange = Exchange {
            socket: &mut socket,
            deadline: std::time::Instant::now() + Duration::from_secs(3),
        };
        let commands: Vec<_> = receiver.try_iter().take(32).collect();
        let reset_audio = commands
            .iter()
            .any(|command| matches!(command, Command::Load(_) | Command::Seek(_)));
        let request = serde_json::to_vec(&commands)?;
        exchange.write_all(&(request.len() as u32).to_le_bytes())?;
        exchange.write_all(&request)?;
        let decoded = read_output(&mut exchange, width, height)?;
        let mut output = shared.lock().unwrap();
        if reset_audio {
            output.audio.clear();
        }
        if !decoded.frame.is_empty() {
            output.frame = decoded.frame;
        }
        output.audio.extend(decoded.audio);
        while output.audio.len() > 12000 {
            output.audio.pop_front();
        }
        output.position = decoded.position;
        if !decoded.error.is_empty() {
            output.error = decoded.error;
        }
        drop(output);
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

pub(super) fn entry() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dimensions = || -> Option<(i32, i32)> {
        Some((args.first()?.parse().ok()?, args.get(1)?.parse().ok()?))
    };
    let (width, height) = dimensions()
        .filter(|(width, height)| (64..=1280).contains(width) && (64..=720).contains(height))
        .ok_or_else(|| io::Error::other("Invalid decoder dimensions"))?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut player = match crate::process_sandbox::install()
        .map_err(|error| error.to_string())
        .and_then(|()| Player::new(width, height, false))
    {
        Ok(player) => player,
        Err(error) => {
            write_output(
                &mut output,
                Output {
                    error: crate::error_message::bounded(error),
                    ..Default::default()
                },
            )?;
            return Ok(());
        }
    };
    loop {
        let mut length = [0; 4];
        match input.read_exact(&mut length) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
        let length = u32::from_le_bytes(length) as usize;
        if length > 256 * 1024 {
            return Err(io::Error::other("Media command budget exceeded"));
        }
        let mut request = vec![0; length];
        input.read_exact(&mut request)?;
        let commands: Vec<Command> = serde_json::from_slice(&request)?;
        if commands.len() > 32 {
            return Err(io::Error::other("Media command count exceeded"));
        }
        let mut decoded = Output::default();
        for command in commands {
            if let Err(error) = player.apply(command) {
                decoded.error = crate::error_message::bounded(error);
            }
        }
        player.poll(&mut decoded);
        write_output(&mut output, decoded)?;
    }
}

fn write_output(output: &mut impl Write, decoded: Output) -> io::Result<()> {
    let error = crate::error_message::bounded(decoded.error);
    let audio_count = decoded.audio.len().min(12000);
    let mut packet = Vec::with_capacity(20 + decoded.frame.len() + audio_count * 8 + error.len());
    packet.extend_from_slice(&decoded.position.to_le_bytes());
    packet.extend_from_slice(&(decoded.frame.len() as u32).to_le_bytes());
    packet.extend_from_slice(&(audio_count as u32).to_le_bytes());
    packet.extend_from_slice(&(error.len() as u32).to_le_bytes());
    packet.extend_from_slice(&decoded.frame);
    for sample in decoded.audio.iter().take(audio_count) {
        packet.extend_from_slice(&sample.x.to_le_bytes());
        packet.extend_from_slice(&sample.y.to_le_bytes());
    }
    packet.extend_from_slice(error.as_bytes());
    output.write_all(&packet)?;
    output.flush()
}

fn read_output(input: &mut impl Read, width: i32, height: i32) -> io::Result<Output> {
    let mut header = [0; 20];
    input.read_exact(&mut header)?;
    let position = f64::from_le_bytes(header[..8].try_into().unwrap());
    let frame = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let audio = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
    let error = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
    if !position.is_finite()
        || position < 0.0
        || (frame != 0 && frame != width as usize * height as usize * 4)
        || audio > 12000
        || error > 2048
    {
        return Err(io::Error::other("Invalid isolated decoder output"));
    }
    let mut decoded = Output {
        position,
        frame: vec![0; frame],
        ..Default::default()
    };
    input.read_exact(&mut decoded.frame)?;
    let mut pcm = vec![0; audio * 8];
    input.read_exact(&mut pcm)?;
    for bytes in pcm.as_chunks::<8>().0 {
        let left = f32::from_le_bytes(bytes[..4].try_into().unwrap());
        let right = f32::from_le_bytes(bytes[4..].try_into().unwrap());
        if !left.is_finite() || !right.is_finite() {
            return Err(io::Error::other("Non-finite isolated PCM"));
        }
        decoded
            .audio
            .push_back(Vector2::new(left.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0)));
    }
    let mut message = vec![0; error];
    input.read_exact(&mut message)?;
    decoded.error = crate::error_message::bounded(
        String::from_utf8(message).map_err(|_| io::Error::other("Invalid decoder diagnostic"))?,
    );
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parent_guard_fails_before_exec_if_expected_client_has_disappeared() {
        let mut child = std::process::Command::new("true");
        unsafe {
            child.pre_exec(|| parent_death_guard(0));
        }
        let error = child.spawn().unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
    }
    #[test]
    fn expired_exchange_deadlines_reject_both_reads_and_writes() {
        let (mut socket, _peer) = UnixStream::pair().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut exchange = Exchange {
            socket: &mut socket,
            deadline: std::time::Instant::now() - Duration::from_secs(1),
        };
        assert_eq!(
            exchange.read(&mut [0; 1]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(
            exchange.write(&[0]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
    #[test]
    fn worker_teardown_closes_descriptors_held_by_resolver_descendants() {
        let (mut socket, child_socket) = UnixStream::pair().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let child = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & printf ready; wait"])
            .stdout(Stdio::from(OwnedFd::from(child_socket)))
            .process_group(0)
            .spawn()
            .unwrap();
        let guard = ChildGuard(child);
        let mut ready = [0; 5];
        socket.read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"ready");
        drop(guard);
        // Killing just the direct worker leaves its resolver holding the socket
        // open for 30 seconds. Group teardown must close every inherited copy.
        assert_eq!(socket.read(&mut [0; 1]).unwrap(), 0);
    }
    #[test]
    fn ipc_checks_lengths_before_allocation_and_rejects_nonfinite_values() {
        let mut packet = Vec::new();
        write_output(
            &mut packet,
            Output {
                position: 1.25,
                frame: vec![0; 64 * 64 * 4],
                audio: VecDeque::from([Vector2::new(0.25, -0.5)]),
                error: "notice".into(),
            },
        )
        .unwrap();
        let decoded = read_output(&mut packet.as_slice(), 64, 64).unwrap();
        assert_eq!(decoded.position, 1.25);
        assert_eq!(decoded.audio[0], Vector2::new(0.25, -0.5));
        assert_eq!(decoded.error, "notice");
        for (offset, value) in [(8, u32::MAX), (12, 12001), (16, 2049)] {
            let mut invalid = packet.clone();
            invalid[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(read_output(&mut invalid.as_slice(), 64, 64).is_err());
        }
        packet[..8].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(read_output(&mut packet.as_slice(), 64, 64).is_err());
        packet[..8].copy_from_slice(&1.25f64.to_le_bytes());
        let pcm = 20 + 64 * 64 * 4;
        packet[pcm..pcm + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(read_output(&mut packet.as_slice(), 64, 64).is_err());
    }
}
