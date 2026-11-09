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
        let mut child = Command::new("godot4")
            .args(["--headless", "--path"])
            .arg(project)
            .args(["--script", "res://tests/peer.gd", "--xr-mode", "off"])
            .env("SSC_TEST_CONFIG", config)
            .env_remove("no_proxy")
            .env_remove("NO_PROXY")
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
    let output = Command::new("timeout")
        .args(["30", "godot4", "--headless", "--xr-mode", "off", "--path"])
        .arg(&project)
        .args(["--script", "res://tests/lifecycle_stress.gd"])
        .env("SSC_TEST_OPUS", &opus_file)
        .env("SSC_STRESS_CYCLES", "100")
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
    let video_project = project.clone();
    let video_url = media_url.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new("timeout")
            .args(["25", "godot4"])
            .args(["--headless", "--xr-mode", "off", "--path"])
            .arg(video_project)
            .args(["--script", "res://tests/video.gd"])
            .env("SSC_TEST_VIDEO", &video_url)
            .env("SSC_TEST_MEDIA_ORIGIN", &video_url)
            .env_remove("no_proxy")
            .env_remove("NO_PROXY")
            .output()
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
        Command::new("timeout")
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
            .env_remove("NO_PROXY")
            .output()
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
    let same_avatar = a
        .wait(|e| {
            snapshot(e)["revision"]
                .as_f64()
                .is_some_and(|r| r > first_grab_revision)
                && snapshot(e)["players"][users[1]]["avatar_actions"]
                    .as_object()
                    .is_some_and(|actions| actions.is_empty())
                && snapshot(e)["interactions"]
                    .as_object()
                    .is_some_and(|grabs| !grabs.is_empty())
        })
        .await
        .context("Same immutable avatar selection must retain grabs")?;
    let same_revision = snapshot(&same_avatar)["revision"].as_f64().unwrap();
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
            Some(json!({"world":asset["id"]})),
            &tokens[0],
        )
        .await?;
    let mut fresh = Peer::start(directory.path(), "fresh", grant)?;
    let state = fresh.wait(|e| e["data"]["user"].is_string()).await?;
    ensure!(
        snapshot(&state)["media"]
            .as_object()
            .is_some_and(|m| m.is_empty()),
        "Fresh instance retained a previous media session"
    );
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
