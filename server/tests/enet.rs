//! Real Godot ↔ Rust ENet tests; runs headless, without a headset or OAuth bypass.
#[path = "../examples/support/world.rs"]
mod example_world;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Cursor},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
/// True when a user systemd manager can actually run scopes. Headless test
/// machines often have none; the media worker's fail-closed confinement
/// (Landlock/seccomp/rlimits) is independent of the optional cgroup scope.
fn user_systemd() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("systemd-run")
            .args(["--user", "--scope", "--quiet", "--collect", "--", "true"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    })
}

struct Api {
    client: reqwest::Client,
    base: String,
    admin: String,
}
impl Api {
    async fn upload(&self, kind: &str, bytes: &[u8], token: &str) -> Result<Value> {
        let boundary = "ssc-upload";
        let mut body=format!("--{boundary}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"package.zip\"\r\nContent-Type: application/zip\r\n\r\n").into_bytes();
        body.extend(bytes);
        body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
        Ok(self
            .client
            .post(format!("{}/api/v1/assets?kind={kind}", self.base))
            .bearer_auth(token)
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    async fn response(
        &self,
        path: &str,
        body: Option<Value>,
        token: &str,
    ) -> Result<reqwest::Response> {
        let request = match body {
            Some(body) => self.client.post(format!("{}{path}", self.base)).json(&body),
            None => self.client.get(format!("{}{path}", self.base)),
        };
        Ok(request.bearer_auth(token).send().await?)
    }

    async fn json(&self, path: &str, body: Option<Value>, token: &str) -> Result<Value> {
        let response = self.response(path, body, token).await?;
        let status = response.status();
        let data: Value = response.json().await?;
        ensure!(status.is_success(), "{path}: HTTP {status}: {data}");
        Ok(data)
    }
}

struct Peer {
    process: Process,
    events: Vec<Value>,
    receiver: Receiver<Value>,
    commands: PathBuf,
    serial: u32,
    errors: Arc<Mutex<Vec<String>>>,
    stderr_reader: Option<std::thread::JoinHandle<()>>,
}
impl Peer {
    fn start(directory: &Path, name: &str, mut grant: Value) -> Result<Self> {
        grant["host"] = json!("127.0.0.1");
        let commands = directory.join(format!("{name}-commands.json"));
        let config = directory.join(format!("{name}.json"));
        std::fs::write(
            &config,
            serde_json::to_vec(&json!({"commands": commands, "grant": grant}))?,
        )?;
        let project = std::env::var_os("SSC_TEST_CLIENT")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../client"));
        let mut command = Command::new("godot4");
        command
            .args(["--headless", "--path"])
            .arg(project)
            .args(["--script", "res://tests/peer.gd", "--xr-mode", "off"])
            .env("SSC_TEST_CONFIG", config)
            .env_remove("no_proxy")
            .env_remove("NO_PROXY");
        // Packaged launchers always set the scope runner, but headless test
        // machines may have no user systemd manager; the media worker's own
        // Landlock/seccomp/rlimit confinement does not depend on it.
        if !user_systemd() {
            command.env_remove("SSC_MEDIA_SCOPE");
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Start Godot using nix develop or nix-shell")?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let captured = errors.clone();
        let stderr_reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr)
                .lines()
                .map_while(std::result::Result::ok)
            {
                eprintln!("{line}");
                if line.starts_with("ERROR:") || line.starts_with("SCRIPT ERROR:") {
                    let mut errors = captured.lock().unwrap();
                    if errors.len() < 64 {
                        errors.push(line.chars().take(4096).collect());
                    }
                }
            }
        });
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if let Some(data) = line.strip_prefix("SSCTEST ")
                    && let Ok(event) = serde_json::from_str(data)
                    && sender.send(event).is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            process: Process(child),
            events: Vec::new(),
            receiver,
            commands,
            serial: 0,
            errors,
            stderr_reader: Some(stderr_reader),
        })
    }

    fn drain(&mut self) {
        self.events.extend(self.receiver.try_iter());
    }

    async fn decoded_video(&mut self, playing: bool) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.command(json!({"action":"video_probe"}))?;
            let serial = self.serial;
            let report = self
                .wait(|e| {
                    e["kind"].as_f64() == Some(250.0)
                        && e["data"]["serial"].as_f64() == Some(f64::from(serial))
                })
                .await?;
            self.drain();
            ensure!(
                !self
                    .events
                    .iter()
                    .any(|e| e["kind"].as_f64() == Some(251.0)),
                "Decoded video failed: {:?}",
                self.events
                    .iter()
                    .find(|e| e["kind"].as_f64() == Some(251.0))
            );
            let data = &report["data"];
            if data["ready"] == true
                && data["playing"] == playing
                && (playing || data["expected"].as_f64() == Some(2.0))
                && data["frames"].as_f64().unwrap_or(0.0) > 0.0
                && (!playing || data["pcm"].as_f64().unwrap_or(0.0) > 0.0)
                && data["position"]
                    .as_f64()
                    .zip(data["expected"].as_f64())
                    .is_some_and(|(actual, expected)| (actual - expected).abs() < 0.5)
            {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "Video did not converge to server timeline: {data}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn wait(&mut self, predicate: impl Fn(&Value) -> bool) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.drain();
            let errors = self.errors.lock().unwrap().clone();
            ensure!(errors.is_empty(), "Godot peer reported errors: {errors:?}");
            if let Some(event) = self.events.iter().find(|e| predicate(e)) {
                return Ok(event.clone());
            }
            ensure!(
                self.process.0.try_wait()?.is_none(),
                "Godot exited: {:?}",
                self.events
            );
            ensure!(
                Instant::now() < deadline,
                "Timed out waiting for Godot event (command {}); last call: {:?}; last voice: {:?}; last event: {:?}",
                self.serial,
                self.events
                    .iter()
                    .rev()
                    .find(|e| e["kind"].as_f64() == Some(21.0)),
                self.events
                    .iter()
                    .rev()
                    .find(|e| e["kind"].as_f64() == Some(20.0)),
                self.events.last()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn command(&mut self, mut command: Value) -> Result<()> {
        self.serial += 1;
        command["serial"] = json!(self.serial);
        // One immutable file per command: a slower Godot frame must not lose
        // an earlier command when the test submits another one.
        let destination = PathBuf::from(format!("{}.{}", self.commands.display(), self.serial));
        let temp = destination.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec(&command)?)?;
        std::fs::rename(temp, destination)?;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.command(json!({"action":"quit"}))?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.process.0.try_wait()?.is_none() {
            ensure!(Instant::now() < deadline, "Godot did not stop gracefully");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if let Some(reader) = self.stderr_reader.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !reader.is_finished() {
                ensure!(
                    Instant::now() < deadline,
                    "Godot stderr remained open after process exit"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            reader
                .join()
                .map_err(|_| anyhow::anyhow!("Godot stderr reader panicked"))?;
        }
        let errors = self.errors.lock().unwrap();
        ensure!(
            errors.is_empty(),
            "Godot peer cleanup reported errors: {errors:?}"
        );
        Ok(())
    }
}

fn token() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}
fn radio_package(bytes: &[u8], radios: Value) -> Result<Vec<u8>> {
    let mut source = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..source.len() {
        let mut file = source.by_index(index)?;
        if file.is_dir() {
            continue;
        }
        output.start_file(file.name(), zip::write::SimpleFileOptions::default())?;
        if file.name() == "manifest.json" {
            let mut manifest: Value = serde_json::from_reader(&mut file)?;
            manifest["radios"] = radios.clone();
            serde_json::to_writer(&mut output, &manifest)?;
        } else {
            std::io::copy(&mut file, &mut output)?;
        }
    }
    Ok(output.finish()?.into_inner())
}
fn snapshot(event: &Value) -> &Value {
    &event["data"]["snapshot"]
}
async fn settle() {
    tokio::time::sleep(Duration::from_millis(200)).await;
}

#[test]
fn deterministic_reference_package() -> Result<()> {
    let mut a = Cursor::new(Vec::new());
    let mut b = Cursor::new(Vec::new());
    example_world::write(&mut a)?;
    example_world::write(&mut b)?;
    ensure!(a.into_inner() == b.into_inner());
    Ok(())
}

#[test]
fn website_administrator_controls_are_authenticated_and_failure_safe() -> Result<()> {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/tests/admin.cjs");
    let output = Command::new("node").arg("--test").arg(tests).output()?;
    ensure!(
        output.status.success(),
        "Website controls failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[tokio::test]
async fn encrypted_godot_enet() -> Result<()> {
    let temp_root = std::env::var_os("SSC_TEST_TMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if Path::new("/tmp/opencode").is_dir() {
                "/tmp/opencode".into()
            } else {
                std::env::temp_dir()
            }
        });
    let directory = tempfile::Builder::new()
        .prefix("ssc-test-")
        .tempdir_in(temp_root)?;
    // Ephemeral ports keep concurrent tests independent.
    let http_socket = std::net::TcpListener::bind("127.0.0.1:0")?;
    let http_address = http_socket.local_addr()?;
    let udp_socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let udp_address = udp_socket.local_addr()?;
    let api = Api {
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?,
        base: format!("http://{http_address}"),
        admin: token(),
    };
    drop(http_socket);
    drop(udp_socket);
    let abandoned_upload = directory
        .path()
        .join(format!("upload-{}.tmp", "a".repeat(64)));
    std::fs::write(&abandoned_upload, b"abandoned upload")?;
    let _server = Process(
        Command::new(env!("CARGO_BIN_EXE_super-simple-chat-server"))
            .env("SSC_ADMIN_TOKEN", &api.admin)
            .env("SSC_DATA", directory.path())
            .env("SSC_LISTEN", http_address.to_string())
            .env("SSC_ENET", udp_address.to_string())
            .env("SSC_PUBLIC_URL", &api.base)
            .env("SSC_TPS", "60")
            .env("SSC_RADIO_CHANNELS", "1,2,3,4")
            .env("SSC_RADIO_BITRATE", "8000")
            .env("SSC_RADIO_PRESETS", "standard,narrow,weak")
            .env("SSC_RADIO_FT8_INTERVAL", "2")
            .env("SSC_RADIO_SSTV_INTERVAL", "30")
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if api.json("/health", None, &api.admin).await.is_ok() {
            break;
        }
        ensure!(Instant::now() < deadline, "Backend did not start");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ensure!(
        !abandoned_upload.exists(),
        "Exclusive startup did not recover a crash-orphaned upload"
    );
    let duplicate = Command::new("timeout")
        .args(["3", env!("CARGO_BIN_EXE_super-simple-chat-server")])
        .env("SSC_ADMIN_TOKEN", &api.admin)
        .env("SSC_DATA", directory.path())
        .env("SSC_LISTEN", "127.0.0.1:0")
        .env("SSC_ENET", "127.0.0.1:0")
        .output()?;
    ensure!(
        !duplicate.status.success()
            && String::from_utf8_lossy(&duplicate.stderr)
                .contains("data directory is already in use"),
        "A second server did not honor exclusive directory ownership"
    );
    for path in ["/api/v1/me", "/api/v1/assets", "/api/v1/instances"] {
        ensure!(api.response(path, None, "invalid").await?.status() == 401);
    }
    api.json("/api/v1/me", None, &api.admin).await?;
    let users = ["123456789", "987654321", "555555555"];
    let tokens = [token(), token(), token()];
    for user in users {
        api.json("/api/v1/admin/users", Some(json!({"id":user})), &api.admin)
            .await?;
    }
    {
        // Test-only fixtures, not a production development-login endpoint.
        let db = rusqlite::Connection::open(directory.path().join("platform.sqlite"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64
            + 3600;
        for (user, token) in users.iter().zip(&tokens) {
            db.execute(
                "INSERT INTO sessions VALUES(?,?,?)",
                rusqlite::params![hex::encode(Sha256::digest(token.as_bytes())), user, expires],
            )?;
        }
    }
    {
        use tokio::io::AsyncWriteExt;
        let mut stalled = Vec::new();
        for _ in 0..2 {
            let mut stream = tokio::net::TcpStream::connect(http_address).await?;
            stream.write_all(format!("POST /api/v1/assets?kind=world HTTP/1.1\r\nHost: {http_address}\r\nAuthorization: Bearer {}\r\nContent-Type: multipart/form-data; boundary=hold\r\nContent-Length: 1000\r\n\r\n--hold\r\nContent-Disposition: form-data; name=\"package\"; filename=\"partial.zip\"\r\n\r\npartial",tokens[0]).as_bytes()).await?;
            stalled.push(stream);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let health = api.json("/api/v1/admin/health", None, &api.admin).await?;
            ensure!(health["realtime_output"]["peer_limit_bytes"] == 4 * 1024 * 1024);
            ensure!(health["realtime_output"]["process_limit_bytes"] == 64 * 1024 * 1024);
            ensure!(
                health["realtime_output"]["reserved_bytes"]
                    .as_u64()
                    .is_some_and(|bytes| bytes <= 64 * 1024 * 1024)
            );
            if health["uploads"]["active"] == 2 {
                ensure!(health["uploads"]["reserved_bytes"] == 2_000_000_000u64);
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Uploads did not acquire bounded reservations"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let rejected = api
            .client
            .post(format!("{}/api/v1/assets?kind=world", api.base))
            .bearer_auth(&tokens[0])
            .header("Content-Type", "multipart/form-data; boundary=hold")
            .body("--hold--\r\n")
            .send()
            .await?;
        ensure!(
            rejected.status() == 429,
            "Concurrent upload admission did not reject excess work"
        );
        drop(stalled);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let health = api.json("/api/v1/admin/health", None, &api.admin).await?;
            if health["uploads"]["active"] == 0 {
                ensure!(health["uploads"]["reserved_bytes"] == 0);
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Cancelled upload reservations survived disconnect"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        ensure!(
            !std::fs::read_dir(directory.path())?.any(|entry| entry.is_ok_and(|entry| entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("upload-") && name.ends_with(".tmp")))),
            "Cancelled upload files survived cleanup"
        );
        println!(
            "PASS exclusive server ownership, crash-stage recovery, bounded concurrent uploads and disconnect cleanup"
        );
    }
    let mut settings = api.json("/api/v1/me", None, &tokens[0]).await?["preferences"].clone();
    settings["resolution_scale"] = json!(0.75);
    settings["voice_volume"] = json!(0.25);
    settings["antialiasing"] = json!("msaa_4x");
    settings["radio_preset"] = json!("narrow");
    api.client
        .put(format!("{}/api/v1/preferences", api.base))
        .bearer_auth(&tokens[0])
        .json(&settings)
        .send()
        .await?
        .error_for_status()?;
    let saved = api.json("/api/v1/me", None, &tokens[0]).await?;
    ensure!(
        saved["preferences"]["resolution_scale"] == 0.75
            && saved["preferences"]["voice_volume"] == 0.25
            && saved["preferences"]["antialiasing"] == "msaa_4x"
            && saved["preferences"]["radio_preset"] == "narrow"
    );
    settings["resolution_scale"] = json!(1.6);
    ensure!(
        api.client
            .put(format!("{}/api/v1/preferences", api.base))
            .bearer_auth(&tokens[0])
            .json(&settings)
            .send()
            .await?
            .status()
            == 400
    );
    ensure!(
        api.json("/api/v1/me", None, &tokens[0]).await?["preferences"]["resolution_scale"] == 0.75
    );
    let mut package = Cursor::new(Vec::new());
    example_world::write_tanks(&mut package)?;
    let package_path = directory.path().join("tank-yard.zip");
    std::fs::write(&package_path, package.get_ref())?;
    let manifest: Value = {
        let mut archive = zip::ZipArchive::new(Cursor::new(package.get_ref()))?;
        serde_json::from_reader(archive.by_name("manifest.json")?)?
    };
    let config_path = directory.path().join("asset-test.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec(
            &json!({"package":package_path,"hash":hex::encode(Sha256::digest(package.get_ref())),"manifest":manifest}),
        )?,
    )?;
    let project = std::env::var_os("SSC_TEST_CLIENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../client"));
    let output = Command::new("timeout")
        .args(["25", "godot4"])
        .args(["--headless", "--xr-mode", "off", "--path"])
        .arg(&project)
        .args(["--script", "res://tests/assets.gd"])
        .env("SSC_TEST_CONFIG", config_path)
        .output()?;
    ensure!(
        output.status.success() && !String::from_utf8_lossy(&output.stderr).contains("ERROR"),
        "Godot asset test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let output = Command::new("timeout")
        .args(["25", "godot4"])
        .args(["--headless", "--xr-mode", "off", "--path"])
        .arg(&project)
        .args(["--script", "res://tests/native.gd"])
        .output()?;
    ensure!(
        output.status.success() && !String::from_utf8_lossy(&output.stderr).contains("ERROR"),
        "Native extension test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let mut avatar_package = Cursor::new(Vec::new());
    let output = Command::new("timeout")
        .args(["25", "godot4"])
        .args(["--headless", "--xr-mode", "off", "--path"])
        .arg(&project)
        .args(["--script", "res://tests/radio.gd"])
        .output()?;
    ensure!(
        output.status.success()
            && !String::from_utf8_lossy(&output.stderr).contains("ERROR")
            && !String::from_utf8_lossy(&output.stderr).contains("leaked"),
        "Radio presentation test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let media_file = directory.path().join("video.mkv");
    let output = Command::new("timeout")
        .args(["20", "godot4", "--headless", "--xr-mode", "off", "--path"])
        .arg(&project)
        .args(["--script", "res://tests/preferences_http.gd"])
        .env("SSC_TEST_API", &api.base)
        .env("SSC_TEST_TOKEN", &tokens[0])
        .output()?;
    ensure!(
        output.status.success()
            && !String::from_utf8_lossy(&output.stderr).contains("ERROR")
            && !String::from_utf8_lossy(&output.stderr).contains("leaked"),
        "Client preference HTTP test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let output = Command::new("timeout")
        .args(["15", "godot4", "--headless", "--xr-mode", "off", "--path"])
        .arg(&project)
        .args(["--script", "res://tests/settings.gd"])
        .output()?;
    ensure!(
        output.status.success()
            && !String::from_utf8_lossy(&output.stderr).contains("ERROR")
            && !String::from_utf8_lossy(&output.stderr).contains("leaked"),
        "Settings test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let opus_file = directory.path().join("clip.opus");
    ensure!(
        Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=330:duration=0.3",
                "-c:a",
                "libopus"
            ])
            .arg(&opus_file)
            .status()?
            .success()
    );
    let output = Command::new("timeout")
        .args(["15", "godot4", "--headless", "--xr-mode", "off", "--path"])
        .arg(&project)
        .args(["--script", "res://tests/packaged_audio.gd"])
        .env("SSC_TEST_OPUS", &opus_file)
        .output()?;
    ensure!(
        output.status.success()
            && !String::from_utf8_lossy(&output.stderr).contains("ERROR")
            && !String::from_utf8_lossy(&output.stderr).contains("leaked"),
        "Packaged audio test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let stress_cycles: u32 = std::env::var("SSC_STRESS_CYCLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|cycles| (1..=100_000).contains(cycles))
        .unwrap_or(100);
    let stress_timeout = 30u64.saturating_mul(u64::from((stress_cycles / 100).max(1)));
    let output = Command::new("timeout")
        .arg(stress_timeout.to_string())
        .args(["godot4", "--headless", "--xr-mode", "off", "--path"])
        .arg(&project)
        .args(["--script", "res://tests/lifecycle_stress.gd"])
        .env("SSC_TEST_OPUS", &opus_file)
        // The documented soak is reproducible: an outer SSC_STRESS_CYCLES
        // raises it above the default 100-cycle regression count, with the
        // deadline scaled so longer runs are not killed early.
        .env("SSC_STRESS_CYCLES", stress_cycles.to_string())
        .output()?;
    ensure!(
        output.status.success()
            && !String::from_utf8_lossy(&output.stderr).contains("ERROR")
            && !String::from_utf8_lossy(&output.stderr).contains("leaked"),
        "Lifecycle stress failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    ensure!(
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
            .arg(&media_file)
            .status()?
            .success()
    );
    let media = std::fs::read(media_file)?;
    let media_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let media_url = format!("http://{}/video.mkv", media_listener.local_addr()?);
    let media_origin = format!("http://{}", media_listener.local_addr()?);
    let media_server = tokio::spawn(async move {
        axum::serve(
            media_listener,
            axum::Router::new()
                .route(
                    "/video.mkv",
                    axum::routing::get(move || {
                        let media = media.clone();
                        async move { ([("content-type", "video/x-matroska")], media) }
                    }),
                )
                .route(
                    "/restricted.mkv",
                    axum::routing::get(|| async { axum::http::StatusCode::FORBIDDEN }),
                )
                .route(
                    "/unsupported.mkv",
                    axum::routing::get(|| async { "This is not a supported media container" }),
                ),
        )
        .await
    });
    // Don't block the async executor that serves the test's media source.
    // Headless test machines may have no user systemd manager; the packaged
    // launcher always sets the scope runner, but the worker's own
    // Landlock/seccomp/rlimit confinement does not depend on it.
    let scoped_media = user_systemd();
    let video_project = project.clone();
    let video_url = media_url.clone();
    let output = tokio::task::spawn_blocking(move || {
        let mut command = Command::new("timeout");
        command
            .args(["25", "godot4"])
            .args(["--headless", "--xr-mode", "off", "--path"])
            .arg(video_project)
            .args(["--script", "res://tests/video.gd"])
            .env("SSC_TEST_VIDEO", &video_url)
            .env("SSC_TEST_MEDIA_ORIGIN", &video_url)
            .env_remove("no_proxy")
            .env_remove("NO_PROXY");
        if !scoped_media {
            command.env_remove("SSC_MEDIA_SCOPE");
        }
        command.output()
    })
    .await??;
    ensure!(
        output.status.success()
            && !String::from_utf8_lossy(&output.stderr).contains("ERROR")
            && !String::from_utf8_lossy(&output.stderr).contains("leaked"),
        "Video surface test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let video_project = project.clone();
    let output = tokio::task::spawn_blocking(move || {
        let mut command = Command::new("timeout");
        command
            .args(["50", "godot4", "--headless", "--xr-mode", "off", "--path"])
            .arg(video_project)
            .args(["--script", "res://tests/video_failures.gd"])
            .env(
                "SSC_TEST_VIDEO_FAILURES",
                serde_json::to_string(&[
                    format!("{media_origin}/removed.mkv"),
                    format!("{media_origin}/restricted.mkv"),
                    format!("{media_origin}/unsupported.mkv"),
                ])
                .unwrap(),
            )
            .env("SSC_TEST_VIDEO", &media_url)
            .env("SSC_TEST_MEDIA_ORIGIN", &media_url)
            .env_remove("no_proxy")
            .env_remove("NO_PROXY");
        if !scoped_media {
            command.env_remove("SSC_MEDIA_SCOPE");
        }
        command.output()
    })
    .await??;
    media_server.abort();
    ensure!(
        output.status.success()
            && !String::from_utf8_lossy(&output.stderr).contains("ERROR")
            && !String::from_utf8_lossy(&output.stderr).contains("leaked"),
        "Video failure/recovery test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    example_world::write_avatar(&mut avatar_package)?;
    let avatar_path = directory.path().join("reference-avatar.zip");
    std::fs::write(&avatar_path, avatar_package.get_ref())?;
    let avatar_manifest: Value = {
        let mut zip = zip::ZipArchive::new(Cursor::new(avatar_package.get_ref()))?;
        serde_json::from_reader(zip.by_name("manifest.json")?)?
    };
    let avatar_config = directory.path().join("avatar-test.json");
    std::fs::write(
        &avatar_config,
        serde_json::to_vec(
            &json!({"package":avatar_path,"hash":hex::encode(Sha256::digest(avatar_package.get_ref())),"manifest":avatar_manifest}),
        )?,
    )?;
    let output = Command::new("timeout")
        .args(["25", "godot4"])
        .args(["--headless", "--xr-mode", "off", "--path"])
        .arg(&project)
        .args(["--script", "res://tests/avatar.gd"])
        .env("SSC_TEST_CONFIG", avatar_config)
        .output()?;
    ensure!(
        output.status.success() && !String::from_utf8_lossy(&output.stderr).contains("ERROR"),
        "VRM test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let asset = api.upload("world", package.get_ref(), &tokens[0]).await?;
    let avatar_asset = api
        .upload("avatar", avatar_package.get_ref(), &tokens[1])
        .await?;
    let grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"world":asset["id"]})),
            &tokens[0],
        )
        .await?;
    let mut a = Peer::start(directory.path(), "a", grant)?;
    let first = a.wait(|e| e["data"]["user"].is_string()).await?;
    let instance = snapshot(&first)["instance"].clone();
    ensure!(first["data"]["send_hz"].as_f64() == Some(60.0));
    let grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"instance":instance,"send_divisor":4,"receive_divisor":4})),
            &tokens[1],
        )
        .await?;
    let mut b = Peer::start(directory.path(), "b", grant)?;
    ensure!(
        b.wait(|e| e["data"]["user"].is_string()).await?["data"]["receive_hz"].as_f64()
            == Some(15.0)
    );
    a.wait(|e| {
        snapshot(e)["players"]
            .as_object()
            .is_some_and(|p| p.len() == 2)
    })
    .await?;
    a.wait(|event| {
        event["kind"].as_f64() == Some(11.0)
            && event["data"]["events"].as_array().is_some_and(|events| {
                events
                    .iter()
                    .any(|event| event["event"] == "join" && event["id"] == users[1])
            })
    })
    .await
    .context("Reliable creator join notification")?;
    a.command(json!({"action":"creator_media","source":"world.on('action',function() world.media({target='screen',action='load',source='https://example.com/video.mp4'}) end)"}))?;
    b.wait(|e| snapshot(e)["media"]["screen"]["playing"] == true)
        .await?;
    b.command(json!({"action":"creator_media","source":"world.on('action',function() world.media({target='screen',action='pause'}) end)"}))?;
    b.wait(|e| {
        e["kind"].as_f64() == Some(255.0)
            && e["data"]["error"]
                .as_str()
                .is_some_and(|s| s.contains("owner"))
    })
    .await?;
    a.command(json!({"action":"command","kind":22,"data":{"target":"screen","action":"pause"}}))?;
    b.wait(|e| snapshot(e)["media"]["screen"]["playing"] == false)
        .await?;
    a.command(
        json!({"action":"creator_media","source":"world.on('action',function() world.media({target='screen',action='seek',value=42}) end)"}),
    )?;
    b.wait(|e| snapshot(e)["media"]["screen"]["position"].as_f64() == Some(42.0))
        .await?;
    a.command(json!({"action":"voice","sequence":1}))?;
    let old_voice = b
        .wait(|e| {
            e["kind"].as_f64() == Some(20.0)
                && e["data"]["user"] == users[0]
                && e["data"]["sequence"].as_f64() == Some(1.0)
        })
        .await?;
    let old_voice_epoch = old_voice["data"]["epoch"]
        .as_str()
        .context("Missing authoritative voice epoch")?
        .parse::<u64>()?;
    api.client
        .put(format!("{}/api/v1/blocks/{}", api.base, users[0]))
        .bearer_auth(&tokens[1])
        .json(&json!({"flags":1}))
        .send()
        .await?
        .error_for_status()?;
    a.command(json!({"action":"voice","sequence":22}))?;
    settle().await;
    b.drain();
    ensure!(
        !b.events
            .iter()
            .any(|e| e["kind"].as_f64() == Some(20.0)
                && e["data"]["sequence"].as_f64() == Some(22.0)),
        "Voice leaked through a mute rule"
    );
    b.wait(|e| {
        e["kind"].as_f64() == Some(3.0)
            && e["data"]["players"].as_array().is_some_and(|players| {
                players
                    .iter()
                    .any(|p| p["id"] == users[0] && p["audio_allowed"] == false)
            })
    })
    .await?;
    a.wait(|e| {
        e["kind"].as_f64() == Some(3.0)
            && e["data"]["players"].as_array().is_some_and(|players| {
                players
                    .iter()
                    .any(|p| p["id"] == users[1] && p["audio_allowed"] == false)
            })
    })
    .await?;
    b.wait(|e| snapshot(e)["players"][users[0]]["audio_allowed"] == false)
        .await?;
    println!(
        "PASS encrypted Opus proximity routing, real Godot encode/decode, and server-side voice blocking"
    );
    a.command(json!({"action":"command","kind":5,"data":{"target":"switch"}}))?;
    b.wait(|e| snapshot(e)["variables"]["lights_enabled"] == true)
        .await?;
    a.command(json!({"action":"command","kind":4,"data":{"text":"hello encrypted world"}}))?;
    b.wait(|e| e["kind"].as_f64() == Some(4.0) && e["data"]["text"] == "hello encrypted world")
        .await?;
    a.command(json!({"action":"malformed"}))?;
    settle().await;
    a.command(json!({"action":"command","kind":4,"data":{"text":"still connected"}}))?;
    b.wait(|e| e["kind"].as_f64() == Some(4.0) && e["data"]["text"] == "still connected")
        .await?;
    b.drain();
    ensure!(
        !b.events
            .iter()
            .any(|e| e["data"]["text"] == "must-not-arrive")
    );
    a.drain();
    b.drain();
    a.events.clear();
    b.events.clear();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    a.drain();
    b.drain();
    let counts = [
        a.events
            .iter()
            .filter(|e| e["kind"].as_f64() == Some(3.0))
            .count(),
        b.events
            .iter()
            .filter(|e| e["kind"].as_f64() == Some(3.0))
            .count(),
    ];
    ensure!(
        counts[0] > 40 && (9..30).contains(&counts[1]),
        "Unexpected full/quarter pose counts: {counts:?}"
    );
    println!(
        "PASS encrypted text/state, malformed packets, independent full/quarter pose rates: {counts:?}"
    );
    // Mid-session renegotiation: a live client may lower its own rates without
    // rejoining; the server announces the effective pair and other sessions
    // keep their negotiated rates.
    b.command(
        json!({"action":"command","kind":29,"data":{"send_divisor":2,"receive_divisor":2}}),
    )?;
    b.wait(|e| {
        e["kind"].as_f64() == Some(26.0)
            && e["data"]["reason"] == "client-request"
            && e["data"]["send_hz"].as_f64() == Some(30.0)
            && e["data"]["receive_hz"].as_f64() == Some(30.0)
    })
    .await?;
    a.command(json!({"action":"command","kind":4,"data":{"text":"rates changed"}}))?;
    a.wait(|e| e["kind"].as_f64() == Some(4.0) && e["data"]["text"] == "rates changed").await?;
    b.wait(|e| e["kind"].as_f64() == Some(4.0) && e["data"]["text"] == "rates changed").await?;
    println!("PASS mid-session rate renegotiation without affecting other sessions");
    let grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"instance":instance})),
            &tokens[2],
        )
        .await?;
    let mut bystander = Peer::start(directory.path(), "bystander", grant)?;
    let late = bystander.wait(|e| e["data"]["user"].is_string()).await?;
    ensure!(
        snapshot(&late)["media"]["screen"]["position"].as_f64() == Some(42.0),
        "Late join lost the paused media timeline"
    );
    println!(
        "PASS authoritative media load/pause/seek, owner permissions and late join at quarter receive rate"
    );
    b.drain();
    let muted_tick = b
        .events
        .iter()
        .filter(|e| e["kind"].as_f64() == Some(3.0))
        .filter_map(|e| e["data"]["tick"].as_f64())
        .fold(0.0f64, f64::max);
    api.client
        .put(format!("{}/api/v1/blocks/{}", api.base, users[0]))
        .bearer_auth(&tokens[1])
        .json(&json!({"flags":0}))
        .send()
        .await?
        .error_for_status()?;
    b.wait(|e| {
        e["kind"].as_f64() == Some(3.0)
            && e["data"]["tick"]
                .as_f64()
                .is_some_and(|tick| tick > muted_tick)
            && e["data"]["players"].as_array().is_some_and(|players| {
                players
                    .iter()
                    .any(|p| p["id"] == users[0] && p["audio_allowed"] == true)
            })
    })
    .await?;
    a.command(json!({"action":"command","kind":21,"data":{"action":"invite","user":users[1]}}))?;
    let invite = b
        .wait(|e| e["kind"].as_f64() == Some(21.0) && e["data"]["event"] == "invite")
        .await
        .context("Private-call invitation delivery")?;
    b.command(json!({"action":"command","kind":21,"data":{"action":"accept","call":invite["data"]["call"]}}))?;
    a.wait(|e| {
        e["kind"].as_f64() == Some(21.0)
            && e["data"]["calls"].as_array().is_some_and(|calls| {
                calls
                    .iter()
                    .any(|c| c["participants"].as_array().is_some_and(|p| p.len() == 2))
            })
    })
    .await
    .context("Private-call acceptance state")?;
    api.client
        .put(format!("{}/api/v1/preferences", api.base))
        .bearer_auth(&tokens[0])
        .json(&json!({"call_mode":"private"}))
        .send()
        .await?
        .error_for_status()?;
    a.command(json!({"action":"voice","sequence":33}))?;
    b.wait(|e| {
        e["kind"].as_f64() == Some(20.0)
            && e["data"]["sequence"].as_f64() == Some(33.0)
            && e["data"]["route"].as_f64() == Some(1.0)
    })
    .await
    .context("Private-call non-positional voice delivery")?;
    settle().await;
    bystander.drain();
    ensure!(
        !bystander
            .events
            .iter()
            .any(|e| e["kind"].as_f64() == Some(20.0)
                && e["data"]["sequence"].as_f64() == Some(33.0)),
        "Private voice leaked to a nearby nonparticipant"
    );
    a.command(json!({"action":"command","kind":21,"data":{"action":"leave"}}))?;
    a.wait(|e| {
        e["kind"].as_f64() == Some(21.0)
            && e["data"]["calls"].as_array().is_some_and(|c| c.is_empty())
    })
    .await
    .context("Private-call leave state")?;
    bystander.stop().await?;
    settle().await;
    println!(
        "PASS private-call invitation/acceptance, non-positional routing, and nearby-bystander exclusion"
    );
    let grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"world":asset["id"],"private":true})),
            &tokens[2],
        )
        .await?;
    let mut remote = Peer::start(directory.path(), "radio-remote", grant)?;
    let joined = remote
        .wait(|e| e["data"]["user"].is_string())
        .await
        .context("Cross-instance radio join")?;
    ensure!(joined["data"]["radio_policy"]["presets"] == json!(["standard", "narrow", "weak"]));
    ensure!(
        joined["data"]["radio_policy"]["ft8_interval"].as_f64() == Some(2.0)
            && joined["data"]["radio_policy"]["sstv_interval"].as_f64() == Some(30.0),
        "Negotiated digital intervals differ: {}",
        joined["data"]["radio_policy"]
    );
    ensure!(
        snapshot(&joined)["instance"] != instance,
        "Radio fixture must use a separate instance"
    );
    let denied_grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"instance":snapshot(&joined)["instance"]})),
            &tokens[0],
        )
        .await?;
    let mut denied = Peer::start(directory.path(), "denied-private-transition", denied_grant)?;
    denied
        .wait(|e| e["kind"].as_f64() == Some(255.0) && e["data"]["error"] == "Invitation required")
        .await
        .context("Uninvited private transition must be rejected")?;
    denied.stop().await?;
    a.command(
        json!({"action":"command","kind":4,"data":{"text":"OLD SESSION SURVIVED REJECTION"}}),
    )?;
    b.wait(|e| {
        e["kind"].as_f64() == Some(4.0) && e["data"]["text"] == "OLD SESSION SURVIVED REJECTION"
    })
    .await
    .context("Rejected transition removed the old authenticated connection")?;
    let reconnect_grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"instance":instance})),
            &tokens[0],
        )
        .await?;
    let mut replacement = Peer::start(directory.path(), "a-live-reconnect", reconnect_grant)?;
    let resumed = replacement
        .wait(|e| e["data"]["user"] == users[0])
        .await
        .context("Live same-instance reconnect")?;
    ensure!(
        snapshot(&resumed)["instance"] == instance
            && snapshot(&resumed)["players"].get(users[1]).is_some(),
        "Reconnect replaced the instance instead of resnapshotting it"
    );
    a.stop().await?;
    a = replacement;
    a.command(json!({"action":"voice","sequence":1}))?;
    let restarted_voice = b
        .wait(|e| {
            e["kind"].as_f64() == Some(20.0)
                && e["data"]["sequence"].as_f64() == Some(1.0)
                && e["data"]["epoch"]
                    .as_str()
                    .and_then(|epoch| epoch.parse::<u64>().ok())
                    .is_some_and(|epoch| epoch > old_voice_epoch)
        })
        .await
        .context(
            "Reconnected voice must carry a new server epoch despite resetting its sequence",
        )?;
    let new_voice_epoch = restarted_voice["data"]["epoch"]
        .as_str()
        .unwrap()
        .to_owned();
    a.command(
        json!({"action":"command","kind":4,"data":{"text":"NEW SESSION SURVIVED OLD DISCONNECT"}}),
    )?;
    b.wait(|e| {
        e["kind"].as_f64() == Some(4.0)
            && e["data"]["text"] == "NEW SESSION SURVIVED OLD DISCONNECT"
    })
    .await
    .context("Stale disconnect removed new membership")?;
    println!(
        "PASS failed private transition preserves the old session; live reconnect resnapshots and survives stale disconnect"
    );
    a.command(json!({"action":"command","kind":25,"data":{"target":"desk_radio","mode":"ft8","data":"CQ CROSS WORLD"}}))?;
    remote
        .wait(|e| {
            e["kind"].as_f64() == Some(25.0)
                && e["data"]["mode"] == "ft8"
                && e["data"]["data"] == "CQ CROSS WORLD"
        })
        .await
        .context("Cross-instance direct FT8 delivery")?;
    a.command(json!({"action":"command","kind":5,"data":{"target":"radio_send"}}))?;
    a.wait(|e| {
        e["kind"].as_f64() == Some(255.0)
            && e["data"]["error"]
                .as_str()
                .is_some_and(|s| s.contains("Radio transmission rate exceeded"))
    })
    .await
    .context("Scripted radio must share the direct-message cooldown")?;
    tokio::time::sleep(Duration::from_millis(2100)).await;
    a.command(json!({"action":"command","kind":5,"data":{"target":"radio_send"}}))?;
    remote
        .wait(|e| {
            e["kind"].as_f64() == Some(25.0)
                && e["data"]["data"] == "CQ CREATOR"
                && e["data"]["sender"] == users[0]
        })
        .await
        .context("Authenticated server-scripted cross-instance FT8 delivery")?;
    let grayscale = "a0".repeat(64 * 48);
    a.command(json!({"action":"command","kind":25,"data":{"target":"desk_radio","mode":"sstv","data":grayscale}}))?;
    remote
        .wait(|e| {
            e["kind"].as_f64() == Some(25.0)
                && e["data"]["mode"] == "sstv"
                && e["data"]["data"]
                    .as_str()
                    .is_some_and(|s| s.len() == 64 * 48 * 2)
        })
        .await
        .context("Cross-instance SSTV delivery")?;
    a.command(json!({"action":"radio_voice","target":"desk_radio"}))?;
    remote
        .wait(|e| {
            e["kind"].as_f64() == Some(24.0)
                && e["data"]["sender"] == users[0]
                && e["data"]["radio"] == "desk_radio"
                && e["data"]["epoch"] == new_voice_epoch
        })
        .await
        .context("Cross-instance Opus radio delivery")?;
    api.client
        .put(format!("{}/api/v1/blocks/{}", api.base, users[0]))
        .bearer_auth(&tokens[2])
        .json(&json!({"flags":3}))
        .send()
        .await?
        .error_for_status()?;
    tokio::time::sleep(Duration::from_millis(2100)).await;
    a.command(json!({"action":"command","kind":25,"data":{"target":"desk_radio","mode":"ft8","data":"CQ BLOCK CHECK"}}))?;
    b.wait(|e| e["kind"].as_f64() == Some(25.0) && e["data"]["data"] == "CQ BLOCK CHECK")
        .await?;
    settle().await;
    remote.drain();
    ensure!(
        !remote
            .events
            .iter()
            .any(|e| e["kind"].as_f64() == Some(25.0) && e["data"]["data"] == "CQ BLOCK CHECK"),
        "Cross-world digital radio bypassed blocking"
    );
    remote.stop().await?;
    let disabled = api
        .upload(
            "world",
            &radio_package(
                package.get_ref(),
                json!([{"id":"desk_radio","position":[0,1.2,-1],"channel":9999}]),
            )?,
            &tokens[2],
        )
        .await?;
    let grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"world":disabled["id"],"private":true})),
            &tokens[2],
        )
        .await?;
    let mut disabled_peer = Peer::start(directory.path(), "radio-disabled", grant)?;
    disabled_peer
        .wait(|e| e["data"]["user"].is_string())
        .await?;
    disabled_peer.command(json!({"action":"command","kind":25,"data":{"target":"desk_radio","mode":"ft8","data":"CQ DISABLED CHANNEL"}}))?;
    disabled_peer
        .wait(|e| {
            e["kind"].as_f64() == Some(255.0)
                && e["data"]["error"]
                    .as_str()
                    .is_some_and(|s| s.contains("disabled"))
        })
        .await?;
    disabled_peer.stop().await?;
    let no_radio = api
        .upload(
            "world",
            &radio_package(package.get_ref(), json!([]))?,
            &tokens[2],
        )
        .await?;
    let grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"world":no_radio["id"],"private":true})),
            &tokens[2],
        )
        .await?;
    let mut silent = Peer::start(directory.path(), "no-radio", grant)?;
    silent.wait(|e| e["data"]["user"].is_string()).await?;
    b.command(json!({"action":"command","kind":25,"data":{"target":"desk_radio","mode":"ft8","data":"CQ OPT OUT CHECK"}}))?;
    a.wait(|e| e["kind"].as_f64() == Some(25.0) && e["data"]["data"] == "CQ OPT OUT CHECK")
        .await?;
    settle().await;
    silent.drain();
    ensure!(
        !silent
            .events
            .iter()
            .any(|e| e["kind"].as_f64().is_some_and(|k| k == 24.0 || k == 25.0)),
        "World without a radio received transmissions"
    );
    silent.stop().await?;
    println!(
        "PASS cross-instance narrowband Opus, protected SSTV image/FT8-style messages and blocking"
    );
    b.command(json!({"action":"command","kind":8,"data":{"avatar":avatar_asset["id"]}}))?;
    a.wait(|e| snapshot(e)["players"][users[1]]["avatar"] == avatar_asset["id"])
        .await?;
    b.command(json!({"action":"command","kind":13,"data":{"name":"happy","weight":1}}))?;
    a.wait(|e| snapshot(e)["players"][users[1]]["avatar_actions"]["happy"].as_f64() == Some(1.0))
        .await?;
    a.command(json!({"action":"raise_hands"}))?;
    b.wait(|e| {
        e["data"]["players"].as_array().is_some_and(|players| {
            players.iter().any(|p| {
                p["id"] == users[0]
                    && p["pose"][8]
                        .as_f64()
                        .is_some_and(|v| (v - 1.7).abs() < 0.01)
            })
        })
    })
    .await?;
    a.command(
        json!({"action":"command","kind":6,"data":{"target":users[1],"action":"touch","hand":0}}),
    )?;
    settle().await;
    b.drain();
    ensure!(
        !b.events.iter().any(|e| e["kind"].as_f64() == Some(6.0)),
        "Touch bypassed opt-in permissions"
    );
    for token in &tokens {
        api.client
            .put(format!("{}/api/v1/preferences", api.base))
            .bearer_auth(token)
            .json(&json!({"can_touch_me":true,"can_touch_others":true}))
            .send()
            .await?
            .error_for_status()?;
    }
    a.command(json!({"action":"command","kind":6,"data":{"target":users[1],"action":"grab","segment":"hair","hand":0}}))?;
    let grabbed = b
        .wait(|e| {
            snapshot(e)["interactions"].as_object().is_some_and(|g| {
                g.values()
                    .any(|v| v["actor"] == users[0] && v["bone"] == "HairRoot")
            })
        })
        .await?;
    let first_grab_revision = snapshot(&grabbed)["revision"].as_f64().unwrap();
    b.command(json!({"action":"command","kind":8,"data":{"avatar":avatar_asset["id"]}}))?;
    // Idempotent selection no longer changes revisions/actions. A subsequent
    // ordered benign state change provides a fresh authoritative observation.
    b.command(json!({"action":"command","kind":7,"data":{"sleeping":true}}))?;
    let same_avatar = a
        .wait(|e| {
            snapshot(e)["revision"]
                .as_f64()
                .is_some_and(|r| r > first_grab_revision)
                && snapshot(e)["players"][users[1]]["avatar_actions"]["happy"].as_f64() == Some(1.0)
                && snapshot(e)["players"][users[1]]["sleeping"] == true
                && snapshot(e)["interactions"]
                    .as_object()
                    .is_some_and(|grabs| !grabs.is_empty())
        })
        .await
        .context("Same immutable avatar selection must retain grabs")?;
    let same_revision = snapshot(&same_avatar)["revision"].as_f64().unwrap();
    b.command(json!({"action":"command","kind":7,"data":{"sleeping":false}}))?;
    b.command(json!({"action":"command","kind":8,"data":{"avatar":""}}))?;
    let removed = a
        .wait(|e| {
            snapshot(e)["revision"]
                .as_f64()
                .is_some_and(|r| r > same_revision)
                && snapshot(e)["players"][users[1]]["avatar"].is_null()
                && snapshot(e)["interactions"]
                    .as_object()
                    .is_some_and(|grabs| grabs.is_empty())
        })
        .await
        .context("Removing an avatar must release old-bone grabs")?;
    let removed_revision = snapshot(&removed)["revision"].as_f64().unwrap();
    // Removal always bypasses admission, but reloading still observes the
    // per-account two-second cooldown established by the initial selection.
    tokio::time::sleep(Duration::from_millis(2050)).await;
    b.command(json!({"action":"command","kind":8,"data":{"avatar":avatar_asset["id"]}}))?;
    let equipped = a
        .wait(|e| {
            snapshot(e)["revision"]
                .as_f64()
                .is_some_and(|r| r > removed_revision)
                && snapshot(e)["players"][users[1]]["avatar"] == avatar_asset["id"]
        })
        .await?;
    let equipped_revision = snapshot(&equipped)["revision"].as_f64().unwrap();
    a.command(json!({"action":"command","kind":6,"data":{"target":users[1],"action":"grab","segment":"hair","hand":0}}))?;
    let grabbed_again = b
        .wait(|e| {
            snapshot(e)["revision"]
                .as_f64()
                .is_some_and(|r| r > equipped_revision)
                && snapshot(e)["interactions"]
                    .as_object()
                    .is_some_and(|grabs| {
                        grabs
                            .values()
                            .any(|g| g["actor"] == users[0] && g["bone"] == "HairRoot")
                    })
        })
        .await?;
    let grabbed_revision = snapshot(&grabbed_again)["revision"].as_f64().unwrap();
    api.client
        .put(format!("{}/api/v1/blocks/{}", api.base, users[0]))
        .bearer_auth(&tokens[1])
        .json(&json!({"flags":5}))
        .send()
        .await?
        .error_for_status()?;
    a.wait(|e| {
        snapshot(e)["revision"]
            .as_f64()
            .is_some_and(|r| r > grabbed_revision)
            && snapshot(e)["interactions"]
                .as_object()
                .is_some_and(|g| g.is_empty())
    })
    .await?;
    println!(
        "PASS owner avatar selection, replicated actions, bilateral touch opt-in, hair-grab ownership and avatar-change/block release"
    );
    // Actual server collision results, not client-declared damage or scores.
    a.command(json!({"action":"command","kind":12,"data":{"action":"lease","target":"tank_a"}}))?;
    b.wait(|e| snapshot(e)["entities"]["tank_a"]["controller"] == users[0])
        .await?;
    b.wait(|e| {
        e["kind"].as_f64() == Some(11.0)
            && e["data"]["events"].as_array().is_some_and(|events| {
                events.iter().any(|event| {
                    event["event"] == "lease"
                        && event["kind"] == "object"
                        && event["target"] == "tank_a"
                        && event["actor"] == users[0]
                        && event.get("hand").is_none()
                })
            })
    })
    .await
    .context("Committed control acquisition must reach client scripts")?;
    b.command(json!({"action":"command","kind":12,"data":{"action":"fire","target":"tank_a"}}))?;
    b.wait(|e| e["data"]["error"] == "No weapon control lease")
        .await?;
    a.command(json!({"action":"command","kind":5,"data":{"target":"spawn_tank"}}))?;
    b.wait(|e| {
        snapshot(e)["entities"].as_object().is_some_and(|entities| {
            entities
                .values()
                .any(|v| v["owner"] == users[0] && v["prefab"] == "tank")
        })
    })
    .await?;
    for health in [75, 50, 25, 0] {
        a.command(
            json!({"action":"command","kind":12,"data":{"action":"fire","target":"tank_a"}}),
        )?;
        if health > 0 {
            b.wait(|e| snapshot(e)["entities"]["tank_b"]["health"].as_f64() == Some(health as f64))
                .await?;
        } else {
            b.wait(|e| {
                snapshot(e)["entities"].is_object()
                    && snapshot(e)["entities"].get("tank_b").is_none()
                    && snapshot(e)["scores"][users[0]].as_f64() == Some(1.0)
            })
            .await?;
        }
        tokio::time::sleep(Duration::from_millis(220)).await;
    }
    b.wait(|e| {
        e["kind"].as_f64() == Some(11.0)
            && e["data"]["events"].as_array().is_some_and(|events| {
                events
                    .iter()
                    .any(|v| v["event"] == "destroy" && v["target"] == "tank_b")
            })
    })
    .await?;
    let before_release = b
        .events
        .iter()
        .filter_map(|e| snapshot(e)["revision"].as_f64())
        .fold(0.0f64, f64::max);
    a.command(json!({"action":"command","kind":12,"data":{"action":"release","target":"tank_a"}}))?;
    b.wait(|e| {
        snapshot(e)["revision"]
            .as_f64()
            .is_some_and(|r| r > before_release)
            && snapshot(e)["entities"]["tank_a"].is_object()
            && snapshot(e)["entities"]["tank_a"]["controller"].is_null()
    })
    .await?;
    b.command(json!({"action":"command","kind":12,"data":{"action":"lease","target":"tank_a"}}))?;
    b.wait(|e| {
        e["kind"].as_f64() == Some(11.0)
            && e["data"]["events"].as_array().is_some_and(|events| {
                events.iter().any(|event| {
                    event["event"] == "release"
                        && event["kind"] == "object"
                        && event["target"] == "tank_a"
                        && event["actor"] == users[0]
                        && event["reason"] == "explicit"
                })
            })
    })
    .await
    .context("Committed control release must reach client scripts")?;
    a.wait(|e| snapshot(e)["entities"]["tank_a"]["controller"] == users[1])
        .await?;
    println!(
        "PASS creator prefab spawning, control permissions/handoff, Rapier projectile damage and scores"
    );
    let public_video = std::env::var("SSC_TEST_PUBLIC_VIDEO")
        .ok()
        .filter(|s| !s.is_empty());
    if let Some(source) = &public_video {
        a.command(json!({"action":"command","kind":22,"data":{"target":"screen","action":"load","source":source}}))?;
        a.decoded_video(true).await?;
        b.decoded_video(true).await?;
        a.command(
            json!({"action":"command","kind":22,"data":{"target":"screen","action":"pause"}}),
        )?;
        a.command(json!({"action":"command","kind":22,"data":{"target":"screen","action":"seek","value":2.0}}))?;
        a.decoded_video(false).await?;
        b.decoded_video(false).await?;
    }
    b.stop().await?;
    settle().await;
    ensure!(api.json("/api/v1/instances", None, &tokens[0]).await?[0]["players"] == 1);
    a.wait(|event| {
        event["kind"].as_f64() == Some(11.0)
            && event["data"]["events"].as_array().is_some_and(|events| {
                events
                    .iter()
                    .any(|event| event["event"] == "leave" && event["id"] == users[1])
            })
    })
    .await
    .context("Reliable creator leave notification")?;
    let grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"instance":instance})),
            &tokens[1],
        )
        .await?;
    let mut c = Peer::start(directory.path(), "rejoin", grant)?;
    ensure!(
        snapshot(&c.wait(|e| e["data"]["user"].is_string()).await?)["variables"]["lights_enabled"]
            == true
    );
    c.wait(|e| {
        snapshot(e)["scores"][users[0]].as_f64() == Some(1.0)
            && snapshot(e)["entities"].get("tank_b").is_none()
    })
    .await?;
    if public_video.is_some() {
        c.decoded_video(false).await?;
        a.command(
            json!({"action":"command","kind":22,"data":{"target":"screen","action":"play"}}),
        )?;
        a.decoded_video(true).await?;
        c.decoded_video(true).await?;
        a.command(
            json!({"action":"command","kind":22,"data":{"target":"screen","action":"stop"}}),
        )?;
        println!(
            "PASS two real clients decode public video/positional PCM, converge to server time, pause/seek, late join and resume"
        );
    }
    c.stop().await?;
    a.stop().await?;
    settle().await;
    ensure!(api.json("/api/v1/instances", None, &tokens[0]).await? == json!([]));
    ensure!(
        api.json("/api/v1/assets", None, &tokens[0])
            .await?
            .as_array()
            .is_some_and(|assets| assets.iter().any(|entry| entry["id"] == asset["id"]))
    );
    ensure!(
        api.response(
            "/api/v1/game-grants",
            Some(json!({"instance":instance})),
            &tokens[0]
        )
        .await?
        .status()
            == 400
    );
    let grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"world":asset["id"],"private":true})),
            &tokens[0],
        )
        .await?;
    // Start a private instance through the same grant API used by the browser.
    let mut fresh = Peer::start(directory.path(), "fresh", grant)?;
    let state = fresh.wait(|e| e["data"]["user"].is_string()).await?;
    ensure!(
        snapshot(&state)["media"]
            .as_object()
            .is_some_and(|m| m.is_empty()),
        "Fresh instance retained a previous media session"
    );
    // Revision resnapshots omit the immutable manifest for peers that already
    // hold the same world version; state-only bumps stay cheap on reliable ch0.
    ensure!(
        snapshot(&state).get("manifest").is_some(),
        "Join snapshot lacked the world manifest"
    );
    fresh.command(json!({"action":"command","kind":7,"data":{"sleeping":true}}))?;
    let bump = fresh
        .wait(|e| {
            e["kind"].as_f64() == Some(2.0)
                && e["data"].get("user").is_none()
                && e["data"]["snapshot"].is_object()
        })
        .await?;
    ensure!(
        snapshot(&bump).get("manifest").is_none(),
        "Revision resnapshot re-shipped the manifest"
    );
    ensure!(
        snapshot(&bump)["visual"].as_object().is_some(),
        "Snapshot lacked the replicated visual map"
    );
    println!("PASS manifest-optional revision resnapshots with replicated visual state");
    let private_id = snapshot(&state)["instance"].as_str().unwrap();
    let owner_list = api.json("/api/v1/instances", None, &tokens[0]).await?;
    ensure!(owner_list.as_array().unwrap().iter().any(|i| i["id"] == private_id && i["can_invite"] == true));
    let stranger_list = api.json("/api/v1/instances", None, &tokens[1]).await?;
    ensure!(!stranger_list.as_array().unwrap().iter().any(|i| i["id"] == private_id));
    let invite_path = format!("/api/v1/instances/{private_id}/invite");
    ensure!(api.response(&invite_path, Some(json!({"user":users[2]})), &tokens[1]).await?.status() == 403);
    let invitation = api.json(&invite_path, Some(json!({"user":users[1]})), &tokens[0]).await?;
    ensure!(invitation["expires_in"] == 300);
    let invited_grant = api.json("/api/v1/game-grants", Some(json!({
        "instance":private_id, "invitation":invitation["invitation"],
        "send_divisor":2,"receive_divisor":4
    })), &tokens[1]).await?;
    let mut invited = Peer::start(directory.path(), "invited", invited_grant)?;
    let invited_state = invited.wait(|e| e["data"]["user"].is_string()).await?;
    ensure!(snapshot(&invited_state)["instance"] == private_id);
    let member_list = api.json("/api/v1/instances", None, &tokens[1]).await?;
    ensure!(member_list.as_array().unwrap().iter().any(|i| i["id"] == private_id && i["can_invite"] == false));
    invited.stop().await?;
    println!("PASS HTTP private-instance creation, owner-only invitations, account-bound redemption and member discovery");
    let pending_gc_grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"world":asset["id"]})),
            &tokens[1],
        )
        .await?;
    let version = asset["version"].as_str().unwrap();
    let retired_path = directory
        .path()
        .join("assets")
        .join(format!("{version}.zip"));
    std::fs::File::open(&retired_path)?.set_times(
        std::fs::FileTimes::new()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(172800)),
    )?;
    ensure!(
        api.response("/api/v1/admin/assets/gc", Some(json!({})), &tokens[0])
            .await?
            .status()
            == 403
    );
    api.json("/api/v1/admin/assets/gc", Some(json!({})), &api.admin)
        .await?;
    ensure!(retired_path.is_file(), "GC removed a catalog version");
    api.client
        .delete(format!(
            "{}/api/v1/assets/{}",
            api.base,
            asset["id"].as_str().unwrap()
        ))
        .bearer_auth(&tokens[0])
        .send()
        .await?
        .error_for_status()?;
    let stats = api
        .json("/api/v1/admin/assets/gc", Some(json!({})), &api.admin)
        .await?;
    ensure!(stats["scanned"].as_u64().is_some_and(|count| count <= 64));
    ensure!(
        retired_path.is_file(),
        "GC removed a live or granted immutable version"
    );
    let bytes = api
        .response(
            &format!(
                "/api/v1/assets/{}/download?version={version}",
                asset["id"].as_str().unwrap()
            ),
            None,
            &tokens[0],
        )
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    ensure!(
        hex::encode(Sha256::digest(&bytes)) == version,
        "Live deleted-catalog download changed bytes"
    );
    fresh.stop().await?;
    settle().await;
    api.json("/api/v1/admin/assets/gc", Some(json!({})), &api.admin)
        .await?;
    ensure!(
        retired_path.is_file(),
        "GC removed an unexpired pending grant's package"
    );
    let mut granted = Peer::start(directory.path(), "gc-grant-handoff", pending_gc_grant)?;
    granted
        .wait(|event| event["data"]["user"].is_string())
        .await?;
    api.json("/api/v1/admin/assets/gc", Some(json!({})), &api.admin)
        .await?;
    ensure!(
        retired_path.is_file(),
        "Grant consumption lost the live-instance pin"
    );
    granted.stop().await?;
    settle().await;
    api.json("/api/v1/admin/assets/gc", Some(json!({})), &api.admin)
        .await?;
    ensure!(
        !retired_path.exists(),
        "Released obsolete version was not reclaimed"
    );
    let db = rusqlite::Connection::open(directory.path().join("platform.sqlite"))?;
    ensure!(
        db.query_row(
            "SELECT COUNT(*) FROM asset_versions WHERE hash=?",
            [version],
            |row| row.get::<_, i64>(0)
        )? == 0
    );
    ensure!(
        api.response(
            &format!(
                "/api/v1/assets/{}/download?version={version}",
                asset["id"].as_str().unwrap()
            ),
            None,
            &api.admin
        )
        .await?
        .status()
            == 404
    );
    println!(
        "PASS administrator-only bounded GC, live deleted-catalog downloads, pending-grant handoff and final version reclamation"
    );
    println!(
        "PASS late join, reconnect, empty-instance destruction including media, persistent catalog"
    );
    Ok(())
}

/// Minimal mirror of the realtime cipher used only by this test binary to
/// exercise admission at the raw ENet level. The construction must stay in
/// lockstep with server/src/realtime/crypto.rs and client/scripts/cipher.gd.
fn test_key(secret: &[u8], direction: u8, purpose: u8) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update([direction, purpose]);
    hash.update((secret.len() as u32).to_le_bytes());
    hash.update(secret);
    hash.update(b"ssc-cipher-v2");
    hash.finalize().into()
}
fn test_seal(secret: &[u8], channel: u8, direction: u8, kind: u8, payload: &[u8]) -> Vec<u8> {
    use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
    use hmac::{Hmac, Mac};
    let iv = rand::random::<[u8; 16]>();
    let mut plain = vec![kind];
    plain.extend_from_slice(payload);
    let sealed = cbc::Encryptor::<aes::Aes256>::new(
        &test_key(secret, direction, 0).into(),
        &iv.into(),
    )
    .encrypt_padded_vec_mut::<Pkcs7>(&plain);
    let mut frame = vec![1u8, channel, direction];
    frame.extend_from_slice(&1u64.to_le_bytes());
    frame.extend_from_slice(&iv);
    frame.extend_from_slice(&sealed);
    let mut mac = Hmac::<Sha256>::new_from_slice(&test_key(secret, direction, 1)).unwrap();
    mac.update(&frame);
    frame.extend_from_slice(&mac.finalize().into_bytes());
    frame
}
fn test_open(secret: &[u8], channel: u8, frame: &[u8]) -> Option<(u8, Vec<u8>)> {
    use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
    use hmac::{Hmac, Mac};
    if frame.len() < 75 || frame[..3] != [1, channel, 1] {
        return None;
    }
    let body = &frame[..frame.len() - 32];
    let mut mac = Hmac::<Sha256>::new_from_slice(&test_key(secret, 1, 1)).unwrap();
    mac.update(body);
    mac.verify_slice(&frame[frame.len() - 32..]).ok()?;
    let plain = cbc::Decryptor::<aes::Aes256>::new(
        &test_key(secret, 1, 0).into(),
        frame[11..27].into(),
    )
    .decrypt_padded_vec_mut::<Pkcs7>(&frame[27..frame.len() - 32])
    .ok()?;
    Some((plain[0], plain[1..].to_vec()))
}

#[tokio::test]
async fn challenge_handshake_blocks_and_admin_endpoints() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("ssc-wire-")
        .tempdir_in(std::env::temp_dir())?;
    let http_socket = std::net::TcpListener::bind("127.0.0.1:0")?;
    let http_address = http_socket.local_addr()?;
    let udp_socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let udp_address = udp_socket.local_addr()?;
    let api = Api {
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?,
        base: format!("http://{http_address}"),
        admin: token(),
    };
    drop(http_socket);
    drop(udp_socket);
    let _server = Process(
        Command::new(env!("CARGO_BIN_EXE_super-simple-chat-server"))
            .env("SSC_ADMIN_TOKEN", &api.admin)
            .env("SSC_DATA", directory.path())
            .env("SSC_LISTEN", http_address.to_string())
            .env("SSC_ENET", udp_address.to_string())
            .env("SSC_PUBLIC_URL", &api.base)
            .env("SSC_TPS", "60")
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if api.json("/health", None, &api.admin).await.is_ok() {
            break;
        }
        ensure!(Instant::now() < deadline, "Backend did not start");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let users = ["123456789", "987654321"];
    let tokens = [token(), token()];
    for user in users {
        api.json("/api/v1/admin/users", Some(json!({"id":user})), &api.admin)
            .await?;
    }
    {
        let db = rusqlite::Connection::open(directory.path().join("platform.sqlite"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64
            + 3600;
        for (user, token) in users.iter().zip(&tokens) {
            db.execute(
                "INSERT INTO sessions VALUES(?,?,?)",
                rusqlite::params![hex::encode(Sha256::digest(token.as_bytes())), user, expires],
            )?;
        }
    }
    // Block listing, unknown-target rejection, and flag round-trip.
    ensure!(
        api.json("/api/v1/blocks", None, &tokens[0]).await?["blocks"]
            .as_array()
            .is_some_and(|blocks| blocks.is_empty())
    );
    ensure!(
        api.client
            .put(format!("{}/api/v1/blocks/000000000", api.base))
            .bearer_auth(&tokens[0])
            .json(&json!({"flags":1}))
            .send()
            .await?
            .status()
            == 404
    );
    api.client
        .put(format!("{}/api/v1/blocks/{}", api.base, users[1]))
        .bearer_auth(&tokens[0])
        .json(&json!({"flags":3}))
        .send()
        .await?
        .error_for_status()?;
    let listed = api.json("/api/v1/blocks", None, &tokens[0]).await?;
    ensure!(
        listed["blocks"][0]["voice"] == true
            && listed["blocks"][0]["text"] == true
            && listed["blocks"][0]["touch"] == false
            && listed["blocks"][0]["avatar"] == false
            && listed["blocks"][0]["complete"] == false
            && listed["blocks"][0]["target"] == users[1]
    );
    api.client
        .put(format!("{}/api/v1/blocks/{}", api.base, users[1]))
        .bearer_auth(&tokens[0])
        .json(&json!({"flags":0}))
        .send()
        .await?
        .error_for_status()?;
    ensure!(
        api.json("/api/v1/blocks", None, &tokens[0]).await?["blocks"][0]["flags"] == 0
    );
    // Admin quota updates on unknown users fail with 404, not silence.
    ensure!(
        api.client
            .put(format!("{}/api/v1/admin/users/000000000", api.base))
            .bearer_auth(&api.admin)
            .json(&json!({"world_slots":1,"avatar_slots":1,"world_bytes":1000,"avatar_bytes":1000}))
            .send()
            .await?
            .status()
            == 404
    );
    // Instance limits and default quotas are visible to the administrator.
    let health = api.json("/api/v1/admin/health", None, &api.admin).await?;
    ensure!(health["limits"]["instances"] == 64 && health["limits"]["instance_players"] == 64);
    ensure!(
        health["defaults"]["world_slots"] == 4
            && health["defaults"]["avatar_slots"] == 8
            && health["defaults"]["world_bytes"] == 1_000_000_000u64
            && health["defaults"]["avatar_bytes"] == 100_000_000u64
    );
    // A disabled asset disappears from user catalogs and grants, and can be
    // re-enabled without losing the immutable version.
    let mut package = Cursor::new(Vec::new());
    example_world::write(&mut package)?;
    let asset = api
        .upload("world", &package.into_inner(), &tokens[0])
        .await?;
    api.json(
        &format!("/api/v1/admin/assets/{}/enabled", asset["id"].as_str().unwrap()),
        Some(json!({"enabled":false})),
        &api.admin,
    )
    .await?;
    ensure!(
        api.response(
            &format!("/api/v1/admin/assets/{}/enabled", asset["id"].as_str().unwrap()),
            Some(json!({"enabled":false})),
            &api.admin
        )
        .await?
        .status()
            == 200
    );
    ensure!(
        !api
            .json("/api/v1/assets?kind=world", None, &tokens[1])
            .await?
            .as_array()
            .is_some_and(|assets| assets.iter().any(|entry| entry["id"] == asset["id"])),
        "Disabled world stayed visible to a normal user"
    );
    ensure!(
        api.response(
            "/api/v1/game-grants",
            Some(json!({"world":asset["id"]})),
            &tokens[0]
        )
        .await?
        .status()
            == 400
    );
    api.json(
        &format!("/api/v1/admin/assets/{}/enabled", asset["id"].as_str().unwrap()),
        Some(json!({"enabled":true})),
        &api.admin,
    )
    .await?;
    let grant = api
        .json(
            "/api/v1/game-grants",
            Some(json!({"world":asset["id"]})),
            &tokens[0],
        )
        .await?;
    ensure!(grant["protocol"] == 2, "Grant did not advertise protocol 2");
    // Bounded administrator log ring: authenticated-only, with recorded admin
    // actions inspectable without a log shipper.
    ensure!(
        api.response("/api/v1/admin/logs", None, &tokens[0])
            .await?
            .status()
            == 403
    );
    let logs = api.json("/api/v1/admin/logs", None, &api.admin).await?;
    let entries = logs.as_array().unwrap();
    ensure!(!entries.is_empty() && entries.len() <= 256);
    ensure!(
        entries
            .iter()
            .any(|entry| entry["line"].as_str().is_some_and(|line| line.contains("allowlisted")))
    );
    ensure!(
        entries.iter().any(|entry| entry["line"]
            .as_str()
            .is_some_and(|line| line.contains("asset") && line.contains("disabled")))
    );
    // Raw ENet admission: the server speaks first with a per-connection
    // challenge; a captured handshake from another connection must fail.
    // Upload ownership/quota/slot/replacement boundaries: one matrix.
    {
        let mut package = Cursor::new(Vec::new());
        example_world::write(&mut package)?;
        let world = package.into_inner();
        let put = |kind: &str, replace: Option<&str>, token: &str, bytes: &[u8]| {
            let mut body = b"--ssc-upload\r\nContent-Disposition: form-data; name=\"package\"; filename=\"package.zip\"\r\nContent-Type: application/zip\r\n\r\n".to_vec();
            body.extend(bytes);
            body.extend(b"\r\n--ssc-upload--\r\n");
            let query = match replace {
                Some(id) => format!("?kind={kind}&replace={id}"),
                None => format!("?kind={kind}"),
            };
            api.client
                .post(format!("{}/api/v1/assets{query}", api.base))
                .bearer_auth(token)
                .header("Content-Type", "multipart/form-data; boundary=ssc-upload")
                .body(body)
                .send()
        };
        // Unknown replacement target is a 404, not an internal error.
        ensure!(
            put("world", Some("nonexistent000"), &tokens[0], &world).await?.status() == 404,
            "Unknown replacement asset did not return 404"
        );
        // Replacing another user's asset or the wrong kind is forbidden.
        let stranger = api.upload("world", &world, &tokens[1]).await?;
        ensure!(
            put("world", Some(stranger["id"].as_str().unwrap()), &tokens[0], &world)
                .await?
                .status()
                == 403,
            "Replacing another user's asset was not forbidden"
        );
        let avatar_package;
        {
            let mut avatar = Cursor::new(Vec::new());
            example_world::write_avatar(&mut avatar)?;
            avatar_package = avatar.into_inner();
        }
        let owned_avatar = api.upload("avatar", &avatar_package, &tokens[0]).await?;
        ensure!(
            put("world", Some(owned_avatar["id"].as_str().unwrap()), &tokens[0], &world)
                .await?
                .status()
                == 403,
            "Replacing with the wrong kind was not forbidden"
        );
        // Slot and byte ceilings: one world slot, 1000 world bytes.
        let quota = |slots: i64, bytes: i64| {
            api.client
                .put(format!("{}/api/v1/admin/users/{}", api.base, users[0]))
                .bearer_auth(&api.admin)
                .json(&json!({"world_slots":slots,"avatar_slots":8,"world_bytes":bytes,"avatar_bytes":100000000}))
                .send()
        };
        quota(1, 1000).await?.error_for_status()?;
        ensure!(world.len() > 1000, "Fixture world must exceed the test byte quota");
        ensure!(
            put("world", None, &tokens[0], &world).await?.status() == 413,
            "An over-quota world upload was not rejected with 413"
        );
        // Slot exhaustion at the cap; replacing an owned asset stays possible.
        quota(1, 1_000_000_000).await?.error_for_status()?;
        let second = put("world", None, &tokens[0], &world).await?;
        ensure!(second.status() == 400, "Slot exhaustion returned {}", second.status());
        let replaced = put("world", Some(asset["id"].as_str().unwrap()), &tokens[0], &world)
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        ensure!(replaced["id"] == asset["id"], "Replacement changed the asset ID");
        // Deleting another user's asset fails without leaking its existence.
        ensure!(
            api.client
                .delete(format!("{}/api/v1/assets/{}", api.base, stranger["id"].as_str().unwrap()))
                .bearer_auth(&tokens[0])
                .send()
                .await?
                .status()
                == 400
        );
        // A byte quota cut below the package size rejects replacement too.
        quota(1, 10).await?.error_for_status()?;
        ensure!(
            put("world", Some(asset["id"].as_str().unwrap()), &tokens[0], &world)
                .await?
                .status()
                == 413,
            "A below-size byte quota did not reject the replacement"
        );
        quota(4, 1_000_000_000).await?.error_for_status()?;
        api.client
            .delete(format!("{}/api/v1/assets/{}", api.base, owned_avatar["id"].as_str().unwrap()))
            .bearer_auth(&tokens[0])
            .send()
            .await?
            .error_for_status()?;
        println!("PASS upload ownership, slot, byte-quota, replacement and deletion boundaries");
    }
    let secret = hex::decode(grant["secret"].as_str().unwrap())?;
    let ticket = grant["ticket"].as_str().unwrap().as_bytes();
    let drain = |host: &mut rusty_enet::Host<std::net::UdpSocket>| -> Result<Vec<Value>> {
        let mut seen = Vec::new();
        for _ in 0..200 {
            while let Some(event) = host.service().map_err(|e| anyhow::anyhow!("{e}"))? {
                match event {
                    rusty_enet::Event::Connect { .. } => {
                        seen.push(json!({"event":"connect"}));
                    }
                    rusty_enet::Event::Receive { channel_id, packet, .. } => {
                        seen.push(json!({"event":"receive","channel":channel_id,
                            "data":hex::encode(packet.data())}));
                    }
                    rusty_enet::Event::Disconnect { .. } => {
                        seen.push(json!({"event":"disconnect"}));
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(seen)
    };
    let connect = || -> Result<(rusty_enet::Host<std::net::UdpSocket>, rusty_enet::PeerID)> {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
        let mut host = rusty_enet::Host::new(
            socket,
            rusty_enet::HostSettings {
                peer_limit: 1,
                channel_limit: 3,
                ..Default::default()
            },
        )?;
        let peer_id = {
            let peer = host.connect(udp_address, 3, 0)?;
            peer.id()
        };
        Ok((host, peer_id))
    };
    let challenge_of = |events: &[Value]| -> Option<Vec<u8>> {
        events
            .iter()
            .filter(|e| e["event"] == "receive" && e["channel"] == 0)
            .find_map(|e| {
                let data = hex::decode(e["data"].as_str().unwrap()).ok()?;
                (data.len() == 17 && data[0] == 0).then(|| data[1..].to_vec())
            })
    };
    // First connection: receive a challenge and abandon it.
    let (mut host, peer) = connect()?;
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while challenge_of(&events).is_none() {
        events.extend(drain(&mut host)?);
        ensure!(Instant::now() < deadline, "No challenge greeting arrived");
    }
    let _ = peer;
    drop(host);
    // Second connection: answer the fresh challenge and get admitted.
    let (mut host, peer) = connect()?;
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut challenge = Vec::new();
    while challenge.is_empty() {
        events.extend(drain(&mut host)?);
        if let Some(found) = challenge_of(&events) {
            challenge = found;
            break;
        }
        ensure!(Instant::now() < deadline, "No challenge for the second connection");
    }
    let mut reply = ticket.to_vec();
    reply.extend_from_slice(&challenge);
    reply.extend_from_slice(&test_seal(&secret, 0, 0, 1, ticket));
    host.peer_mut(peer)
        .send(0, &rusty_enet::Packet::new(reply.clone(), rusty_enet::PacketKind::Reliable))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut admitted = Value::Null;
    while admitted.is_null() {
        for event in drain(&mut host)? {
            if event["event"] == "receive" && event["channel"] == 0 {
                let data = hex::decode(event["data"].as_str().unwrap())?;
                if let Some((2, payload)) = test_open(&secret, 0, &data) {
                    let value: Value = serde_json::from_slice(&payload)?;
                    if value["user"].is_string() {
                        admitted = value;
                    }
                }
            }
        }
        ensure!(Instant::now() < deadline, "Correct handshake was not admitted");
    }
    ensure!(
        admitted["snapshot"]["manifest"].is_object(),
        "Admission snapshot lacked the manifest"
    );
    drop(host);
    // Third connection: replay the second connection's captured reply bytes.
    // The stale challenge must reject it before the one-use ticket is touched.
    let (mut host, peer) = connect()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    // Wait for the server's challenge greeting so the replay lands after it.
    while challenge_of(&events).is_none() {
        events.extend(drain(&mut host)?);
        ensure!(
            Instant::now() < deadline,
            "No challenge greeting for the replay connection"
        );
    }
    host.peer_mut(peer)
        .send(0, &rusty_enet::Packet::new(reply, rusty_enet::PacketKind::Reliable))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut outcome = Value::Null;
    while outcome.is_null() {
        for event in drain(&mut host)? {
            match event["event"].as_str() {
                Some("disconnect") => outcome = event,
                Some("receive") => {
                    let data = hex::decode(event["data"].as_str().unwrap())?;
                    // The unauthenticated challenge greeting is expected; any
                    // other packet would mean the replay was admitted.
                    if data.len() == 17 && data[0] == 0 {
                        continue;
                    }
                    outcome = event;
                }
                _ => {}
            }
        }
        ensure!(
            Instant::now() < deadline,
            "Replayed handshake was neither rejected nor disconnected"
        );
    }
    ensure!(
        outcome["event"] == "disconnect",
        "Replayed handshake was accepted or answered: {outcome}"
    );
    println!("PASS challenge admission, ticket binding, and stale-handshake rejection");
    // Operator policies are persisted separately from live instances and
    // explicitly take effect on restart, not halfway through a session.
    let settings_path = "/api/v1/admin/settings";
    ensure!(api.response(settings_path, None, &tokens[0]).await?.status() == 403);
    let settings = api.json(settings_path, None, &api.admin).await?;
    ensure!(settings["restart_required"] == false);
    let mut saved = settings["saved"].clone();
    saved["tps"] = json!(30);
    saved["instance_limit"] = json!(3);
    saved["instance_players"] = json!(8);
    saved["default_world_slots"] = json!(7);
    saved["voice_bitrate"] = json!(16000);
    saved["radio_presets"] = json!(["narrow"]);
    saved["script_policy"] = json!({"memory_bytes":1048576,"callback_millis":1,"scheduler_millis":2});
    let put = |value: Value, token: String| {
        api.client.put(format!("{}{settings_path}", api.base)).bearer_auth(token).json(&value).send()
    };
    let mut legacy = settings["saved"].clone();
    legacy.as_object_mut().unwrap().remove("script_policy");
    let legacy_response: Value = put(legacy, api.admin.clone()).await?.error_for_status()?.json().await?;
    ensure!(legacy_response["restart_required"] == false);
    ensure!(put(saved.clone(), tokens[0].clone()).await?.status() == 403);
    let mut invalid = saved.clone();
    invalid["network_rates"]["minimum_hz"] = json!(60);
    ensure!(put(invalid, api.admin.clone()).await?.status() == 400);
    let mut invalid = saved.clone();
    invalid["radio_presets"] = json!(["unknown"]);
    ensure!(put(invalid, api.admin.clone()).await?.status() == 400);
    let mut invalid = saved.clone();
    invalid["radio_presets"] = json!(["standard,narrow"]);
    ensure!(put(invalid, api.admin.clone()).await?.status() == 400);
    let mut invalid = saved.clone();
    invalid["script_policy"]["callback_millis"] = json!(1000);
    ensure!(put(invalid, api.admin.clone()).await?.status() == 400);
    let mut invalid = saved.clone();
    invalid["admin_token"] = json!("must-not-be-configurable");
    ensure!(put(invalid, api.admin.clone()).await?.status() == 422);
    ensure!(api.json(settings_path, None, &api.admin).await?["saved"] == settings["saved"]);
    let response: Value = put(saved.clone(), api.admin.clone()).await?.error_for_status()?.json().await?;
    ensure!(response["restart_required"] == true);
    let pending = api.json(settings_path, None, &api.admin).await?;
    ensure!(pending["saved"] == saved && pending["active"]["tps"] == 60);
    drop(host);
    drop(_server);
    let _restarted = Process(
        Command::new(env!("CARGO_BIN_EXE_super-simple-chat-server"))
            .env("SSC_ADMIN_TOKEN", &api.admin)
            .env("SSC_DATA", directory.path())
            .env("SSC_LISTEN", http_address.to_string())
            .env("SSC_ENET", udp_address.to_string())
            .env("SSC_PUBLIC_URL", &api.base)
            .env("SSC_TPS", "60")
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(current) = api.json(settings_path, None, &api.admin).await {
            ensure!(current["restart_required"] == false && current["active"] == saved);
            break;
        }
        ensure!(Instant::now() < deadline, "Restarted server did not load saved policies");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let health = api.json("/api/v1/admin/health", None, &api.admin).await?;
    ensure!(health["tps"] == 30 && health["limits"]["instances"] == 3 && health["limits"]["instance_players"] == 8);
    ensure!(health["instances"] == 0 && health["defaults"]["world_slots"] == 7);
    ensure!(health["script_policy"] == saved["script_policy"]);
    ensure!(health["creator_vms"]["per_vm_bytes"] == 1048576);
    api.json("/api/v1/admin/users", Some(json!({"id":"444444444"})), &api.admin).await?;
    let accounts = api.json("/api/v1/admin/users", None, &api.admin).await?;
    ensure!(accounts.as_array().unwrap().iter().any(|user| user["id"] == "444444444" && user["world_slots"] == 7));
    println!("PASS validated administrator policy edits, rejected secret fields, unchanged live policies and SQLite persistence across restart");
    Ok(())
}
