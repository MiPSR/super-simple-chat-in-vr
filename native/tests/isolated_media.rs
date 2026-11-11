//! Exercise the packaged helper, not the in-process codec fixture path.
use std::{
    io::{Read, Write},
    net::TcpListener,
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct Helper {
    child: Child,
    socket: UnixStream,
}
impl Drop for Helper {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}
impl Helper {
    fn start(origin: Option<String>) -> Self {
        let (socket, child_socket) = UnixStream::pair().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_ssc-media-worker"));
        command
            .args([
                "64".to_string(),
                "64".to_string(),
                std::process::id().to_string(),
            ])
            .env_clear()
            .env("HOME", "/nonexistent")
            .env("PATH", "")
            .process_group(0)
            .stdin(Stdio::from(OwnedFd::from(
                child_socket.try_clone().unwrap(),
            )))
            .stdout(Stdio::from(OwnedFd::from(child_socket)))
            .stderr(Stdio::inherit());
        if let Some(origin) = origin {
            command.env("SSC_TEST_MEDIA_ORIGIN", origin);
        }
        if let Some(resolver) = std::env::var_os("SSC_YTDLP") {
            command.env("SSC_YTDLP", resolver);
        }
        for name in ["SSL_CERT_FILE", "NIX_SSL_CERT_FILE"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        Self {
            child: command.spawn().unwrap(),
            socket,
        }
    }
    fn poll(&mut self, commands: serde_json::Value) -> (bool, bool, String) {
        let commands = serde_json::to_vec(&commands).unwrap();
        self.socket
            .write_all(&(commands.len() as u32).to_le_bytes())
            .unwrap();
        self.socket.write_all(&commands).unwrap();
        let mut header = [0; 20];
        self.socket.read_exact(&mut header).unwrap();
        let frame = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
        let audio = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
        let error = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
        assert!((frame == 0 || frame == 64 * 64 * 4) && audio <= 12000 && error <= 2048);
        let mut body = vec![0; frame + audio * 8 + error];
        self.socket.read_exact(&mut body).unwrap();
        let pcm = body[frame..frame + audio * 8]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|sample| f32::from_le_bytes(*sample).abs() > 0.001);
        (
            frame > 0,
            pcm,
            String::from_utf8(body[frame + audio * 8..].to_vec()).unwrap(),
        )
    }
}

#[test]
fn isolated_worker_initializes_and_rejects_filesystem_sources() {
    let mut helper = Helper::start(None);
    let (_, _, error) = helper.poll(serde_json::json!([]));
    assert!(error.is_empty(), "{error}");
    let (_, _, error) = helper.poll(serde_json::json!([{"Load":"file:///etc/hosts"}]));
    assert!(!error.is_empty());
}

#[test]
fn isolated_worker_decodes_public_route_fixture_without_client_filesystem_access() {
    let directory = tempfile::tempdir().unwrap();
    let video = directory.path().join("fixture.mkv");
    assert!(
        Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "2",
                "-c:v",
                "mpeg4",
                "-c:a",
                "pcm_s16le"
            ])
            .arg(&video)
            .status()
            .unwrap()
            .success()
    );
    let bytes = std::fs::read(video).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let running = stop.clone();
    let server = std::thread::spawn(move || {
        while !running.load(Ordering::Acquire) {
            if let Ok((mut socket, _)) = listener.accept() {
                let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
                let _ = socket.set_write_timeout(Some(Duration::from_millis(100)));
                let mut request = [0; 8192];
                let _ = socket.read(&mut request);
                let _ = write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: video/x-matroska\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = socket.write_all(&bytes);
            } else {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    });
    let mut helper = Helper::start(Some(format!("http://{address}")));
    let mut request = serde_json::json!([{"Load":format!("http://{address}/fixture.mkv")}]);
    let deadline = Instant::now() + Duration::from_secs(8);
    let (mut frame, mut pcm) = (false, false);
    let mut failure = String::new();
    while Instant::now() < deadline && !(frame && pcm) {
        let (got_frame, got_pcm, error) = helper.poll(request.take());
        request = serde_json::json!([]);
        frame |= got_frame;
        pcm |= got_pcm;
        if !error.is_empty() {
            failure = error;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(helper);
    stop.store(true, Ordering::Release);
    server.join().unwrap();
    assert!(
        failure.is_empty() && frame && pcm,
        "frame={frame}, pcm={pcm}, error={failure}"
    );
}

#[test]
#[ignore = "opt-in public-network/YouTube acceptance; needs SSC_TEST_PUBLIC_VIDEO"]
fn isolated_worker_external_media() {
    let source = std::env::var("SSC_TEST_PUBLIC_VIDEO").expect("Set a public MP4/HLS/YouTube URL");
    let mut helper = Helper::start(None);
    let mut request = serde_json::json!([{"Load":source}]);
    let deadline = Instant::now() + Duration::from_secs(45);
    let (mut frame, mut pcm) = (false, false);
    while Instant::now() < deadline && !(frame && pcm) {
        let (got_frame, got_pcm, error) = helper.poll(request.take());
        request = serde_json::json!([]);
        assert!(error.is_empty(), "{error}");
        frame |= got_frame;
        pcm |= got_pcm;
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        frame && pcm,
        "External source did not produce video and PCM"
    );
}
