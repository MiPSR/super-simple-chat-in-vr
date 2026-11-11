//! libmpv's existing software renderer feeds Vulkan-compatible Godot textures.
//! Decode/render/IO run on a worker, never the XR frame thread. No custom codec.
use crate::{media_proxy::MediaProxy, media_security::NetworkGuard};
use godot::{classes::RefCounted, prelude::*};
use libmpv2_sys as mpv;
use std::{
    collections::VecDeque,
    ffi::{CStr, CString, c_void},
    fs::File,
    io::Read,
    os::fd::{AsRawFd, FromRawFd},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::Duration,
};

static PLAYERS: AtomicUsize = AtomicUsize::new(0);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
type SharedJoin = Arc<Mutex<Option<thread::JoinHandle<()>>>>;
struct WorkerRecord {
    stop: Arc<AtomicBool>,
    join: SharedJoin,
}
static WORKERS: Mutex<Vec<WorkerRecord>> = Mutex::new(Vec::new());

pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::Release);
    let records = std::mem::take(&mut *WORKERS.lock().unwrap());
    for record in &records {
        record.stop.store(true, Ordering::Release);
    }
    for record in records {
        if let Some(join) = record.join.lock().unwrap().take() {
            let _ = join.join();
        }
    }
}
#[derive(Default)]
struct Output {
    frame: Vec<u8>,
    audio: VecDeque<Vector2>,
    position: f64,
    error: String,
}
#[derive(serde::Serialize, serde::Deserialize)]
enum Command {
    Load(String),
    Pause(bool),
    Seek(f64),
    Speed(f64),
}
struct Worker {
    stop: Arc<AtomicBool>,
    sender: SyncSender<Command>,
    output: Arc<Mutex<Output>>,
    join: SharedJoin,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Handles remain in the bounded registry until reaped or extension
        // shutdown. Never detach native code past the library's unload point.
        if let Ok(mut join) = self.join.try_lock()
            && join.as_ref().is_some_and(|j| j.is_finished())
        {
            let _ = join.take().unwrap().join();
        }
    }
}
struct Player {
    diagnostic: String,
    local_fixture: bool,
    loaded: bool,
    pending_seek: Option<f64>,
    handle: *mut mpv::mpv_handle,
    render: *mut mpv::mpv_render_context,
    _writer: File,
    width: i32,
    height: i32,
    pixels: Vec<u8>,
    audio: Arc<Mutex<VecDeque<Vector2>>>,
    audio_stop: Arc<AtomicBool>,
    audio_thread: Option<thread::JoinHandle<()>>,
    _guard: NetworkGuard,
    proxy: MediaProxy,
}
impl Drop for Player {
    fn drop(&mut self) {
        // The independent reader keeps draining until mpv has terminated: PCM
        // output must never block its core/render API on a full pipe.
        unsafe {
            if !self.render.is_null() {
                mpv::mpv_render_context_free(self.render);
            }
            if !self.handle.is_null() {
                mpv::mpv_terminate_destroy(self.handle);
            }
        }
        self.audio_stop.store(true, Ordering::Release);
        if let Some(reader) = self.audio_thread.take() {
            let _ = reader.join();
        }
    }
}
fn check(code: i32) -> Result<(), String> {
    if code >= 0 {
        return Ok(());
    }
    let message = unsafe { CStr::from_ptr(mpv::mpv_error_string(code)) };
    Err(message.to_string_lossy().into_owned())
}
fn param(kind: u32, data: *mut c_void) -> mpv::mpv_render_param {
    mpv::mpv_render_param { type_: kind, data }
}
impl Player {
    fn new(width: i32, height: i32, local_fixture: bool) -> Result<Self, String> {
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err("Video service is shutting down".into());
        }
        if !local_fixture
            && ["no_proxy", "NO_PROXY"]
                .iter()
                .any(|name| std::env::var(name).is_ok_and(|v| !v.is_empty()))
        {
            return Err(
                "Protected media requires no_proxy/NO_PROXY unset; use the packaged launcher"
                    .into(),
            );
        }
        let proxy = MediaProxy::new().map_err(|e| format!("Media relay: {e}"))?;
        Self::with_proxy(width, height, local_fixture, proxy)
    }
    fn with_proxy(
        width: i32,
        height: i32,
        local_fixture: bool,
        proxy: MediaProxy,
    ) -> Result<Self, String> {
        let proxy_url = proxy.url();
        let guard = NetworkGuard::install(proxy.address, proxy.fixture).map_err(|e| {
            format!("Media socket guard requires Linux seccomp user notifications: {e}")
        })?;
        let mut fds = [-1; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: pipe2 transferred two distinct owned descriptors on success.
        let (reader, writer) = unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
        if unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let handle = unsafe { mpv::mpv_create() };
        if handle.is_null() {
            return Err("libmpv initialization failed".into());
        }
        let audio = Arc::new(Mutex::new(VecDeque::new()));
        let shared_audio = audio.clone();
        let audio_stop = Arc::new(AtomicBool::new(false));
        let stop = audio_stop.clone();
        let audio_thread = thread::spawn(move || read_audio(reader, shared_audio, stop));
        let mut player = Self {
            diagnostic: String::new(),
            local_fixture,
            loaded: false,
            pending_seek: None,
            handle,
            render: ptr::null_mut(),
            _writer: writer,
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
            audio,
            audio_stop,
            audio_thread: Some(audio_thread),
            _guard: guard,
            proxy,
        };
        for (name, value) in [
            ("config", "no"),
            ("terminal", "no"),
            ("input-terminal", "no"),
            ("idle", "yes"),
            ("vo", "libmpv"),
            ("ao", "pcm"),
            ("audio-channels", "stereo"),
            ("audio-samplerate", "48000"),
            ("audio-format", "float"),
            ("ao-pcm-waveheader", "no"),
            ("hwdec", "no"),
            ("vd-lavc-threads", "2"),
            ("ad-lavc-threads", "2"),
            ("vd-lavc-o", "max_pixels=8294400"),
            ("cache-on-disk", "no"),
            ("demuxer-max-bytes", "32MiB"),
            ("demuxer-max-back-bytes", "4MiB"),
            ("network-timeout", "5"),
            ("video-timing-offset", "0"),
            ("ytdl", "yes"),
            ("load-scripts", "no"),
            ("autoload-files", "no"),
            ("load-unsafe-playlists", "no"),
            ("tls-verify", "yes"),
            (
                "ytdl-format",
                "bestvideo[height<=720]+bestaudio/best[height<=720]",
            ),
        ] {
            player.option(name, value)?;
        }
        player.option("http-proxy", &proxy_url)?;
        player.option(
            "stream-lavf-o",
            &format!("http_proxy=%{}%{proxy_url}", proxy_url.len()),
        )?;
        player.option("ytdl-raw-options", &format!("ignore-config=,no-cache-dir=,no-plugin-dirs=,socket-timeout=5,retries=1,extractor-retries=1,playlist-end=1,sub-langs=none,proxy=%{}%{proxy_url}",proxy_url.len()))?;
        // Only mpv's bundled hook runs; user scripts/config remain disabled.
        // The Nix launcher supplies an immutable resolver path, not a shell command.
        if let Ok(path) = std::env::var("SSC_YTDLP") {
            player.option("script-opts", &format!("ytdl_hook-ytdl_path={path}"))?;
        }
        player.option(
            "ao-pcm-file",
            &format!("/proc/self/fd/{}", player._writer.as_raw_fd()),
        )?;
        // No file/device/pipe protocols in public media playlists or redirects.
        let protocols = if local_fixture {
            "file,http,https,tcp,tls,crypto,data,httpproxy"
        } else {
            "http,https,tcp,tls,crypto,data,httpproxy"
        };
        player.option(
            "demuxer-lavf-o",
            &format!(
                "protocol_whitelist=%{}%{protocols},http_proxy=%{}%{proxy_url}",
                protocols.len(),
                proxy_url.len()
            ),
        )?;
        unsafe {
            check(mpv::mpv_initialize(player.handle))?;
            check(mpv::mpv_request_log_messages(
                player.handle,
                c"warn".as_ptr(),
            ))?;
            check(mpv::mpv_hook_add(player.handle, 0, c"on_load".as_ptr(), 50))?;
        }
        let mut params = [
            param(
                mpv::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                c"sw".as_ptr().cast_mut().cast(),
            ),
            param(0, ptr::null_mut()),
        ];
        unsafe {
            check(mpv::mpv_render_context_create(
                &mut player.render,
                player.handle,
                params.as_mut_ptr(),
            ))?;
        }
        Ok(player)
    }
    fn option(&self, name: &str, value: &str) -> Result<(), String> {
        let name = CString::new(name).map_err(|e| e.to_string())?;
        let value = CString::new(value).map_err(|e| e.to_string())?;
        unsafe {
            check(mpv::mpv_set_option_string(
                self.handle,
                name.as_ptr(),
                value.as_ptr(),
            ))
        }
        .map_err(|e| format!("{}: {e}", name.to_string_lossy()))
    }
    fn command(&self, args: &[&str]) -> Result<(), String> {
        let strings = args
            .iter()
            .map(|a| CString::new(*a).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut pointers: Vec<_> = strings.iter().map(|s| s.as_ptr()).collect();
        pointers.push(ptr::null());
        // mpv copies all command arguments before the asynchronous call returns.
        unsafe {
            check(mpv::mpv_command_async(
                self.handle,
                0,
                pointers.as_mut_ptr(),
            ))
        }
    }
    fn property(&self, name: &CStr, format: u32, data: *mut c_void) -> Result<(), String> {
        unsafe {
            check(mpv::mpv_set_property_async(
                self.handle,
                0,
                name.as_ptr(),
                format,
                data,
            ))
        }
    }
    fn apply(&mut self, command: Command) -> Result<(), String> {
        match command {
            Command::Load(url) => {
                self.diagnostic.clear();
                if !self.local_fixture
                    && (url.len() > 4096
                        || url.contains(['\0', '\n', '\r'])
                        || !(url.starts_with("https://") || url.starts_with("http://")))
                {
                    return Err("Only validated HTTP(S) video URLs are accepted".into());
                }
                self.loaded = false;
                self.pending_seek = None;
                self.command(&["loadfile", &url, "replace"])
            }
            Command::Pause(paused) => {
                let mut value = i32::from(paused);
                self.property(
                    c"pause",
                    mpv::mpv_format_MPV_FORMAT_FLAG,
                    (&mut value as *mut i32).cast(),
                )
            }
            Command::Seek(position) => {
                self.audio.lock().unwrap().clear();
                if self.loaded {
                    self.command(&["seek", &position.to_string(), "absolute+exact"])
                } else {
                    self.pending_seek = Some(position);
                    Ok(())
                }
            }
            Command::Speed(mut speed) => self.property(
                c"speed",
                mpv::mpv_format_MPV_FORMAT_DOUBLE,
                (&mut speed as *mut f64).cast(),
            ),
        }
    }
    fn poll(&mut self, output: &mut Output) {
        for _ in 0..32 {
            unsafe {
                let event = mpv::mpv_wait_event(self.handle, 0.0);
                if event.is_null() || (*event).event_id == mpv::mpv_event_id_MPV_EVENT_NONE {
                    break;
                }
                if (*event).event_id == mpv::mpv_event_id_MPV_EVENT_LOG_MESSAGE
                    && !(*event).data.is_null()
                {
                    let log = &*(*event).data.cast::<mpv::mpv_event_log_message>();
                    if !log.text.is_null() {
                        self.diagnostic = crate::error_message::bounded(format_args!(
                            "{}{}",
                            self.diagnostic,
                            CStr::from_ptr(log.text).to_string_lossy()
                        ));
                    }
                }
                if (*event).event_id == mpv::mpv_event_id_MPV_EVENT_HOOK && !(*event).data.is_null()
                {
                    let hook = &*(*event).data.cast::<mpv::mpv_event_hook>();
                    let name =
                        mpv::mpv_get_property_string(self.handle, c"stream-open-filename".as_ptr());
                    let permitted = if name.is_null() {
                        false
                    } else {
                        let source = CStr::from_ptr(name).to_string_lossy();
                        let valid =
                            self.local_fixture || crate::media_source::network_source(&source);
                        mpv::mpv_free(name.cast());
                        valid
                    };
                    if !permitted {
                        output.error =
                            "Media playlist/resolver contained a forbidden source or unsupported directive".into();
                        // Replace the source before releasing the load hook;
                        // never let a network playlist open a local file/EDL.
                        if mpv::mpv_set_property_string(
                            self.handle,
                            c"stream-open-filename".as_ptr(),
                            c"memory://".as_ptr(),
                        ) < 0
                        {
                            continue;
                        }
                    }
                    let _ = mpv::mpv_hook_continue(self.handle, hook.id);
                }
                if (*event).event_id == mpv::mpv_event_id_MPV_EVENT_FILE_LOADED {
                    self.loaded = true;
                    if let Some(position) = self.pending_seek.take()
                        && let Err(error) =
                            self.command(&["seek", &position.to_string(), "absolute+exact"])
                    {
                        output.error = error;
                    }
                }
                if (*event).event_id == mpv::mpv_event_id_MPV_EVENT_END_FILE
                    && !(*event).data.is_null()
                {
                    let end = &*(*event).data.cast::<mpv::mpv_event_end_file>();
                    if let Err(error) = check(end.error) {
                        output.error = crate::error_message::bounded(format_args!(
                            "{error}: {}",
                            self.diagnostic
                        ));
                    }
                }
            }
        }
        let mut position = 0.0f64;
        unsafe {
            if mpv::mpv_get_property(
                self.handle,
                c"time-pos".as_ptr(),
                mpv::mpv_format_MPV_FORMAT_DOUBLE,
                (&mut position as *mut f64).cast(),
            ) >= 0
            {
                output.position = position;
            }
        }
        if unsafe { mpv::mpv_render_context_update(self.render) } & 1 != 0 {
            let mut size = [self.width, self.height];
            let mut stride = self.width as usize * 4;
            let mut block = 0i32;
            let mut params = [
                param(
                    mpv::mpv_render_param_type_MPV_RENDER_PARAM_SW_SIZE,
                    size.as_mut_ptr().cast(),
                ),
                param(
                    mpv::mpv_render_param_type_MPV_RENDER_PARAM_SW_FORMAT,
                    c"rgb0".as_ptr().cast_mut().cast(),
                ),
                param(
                    mpv::mpv_render_param_type_MPV_RENDER_PARAM_SW_STRIDE,
                    (&mut stride as *mut usize).cast(),
                ),
                param(
                    mpv::mpv_render_param_type_MPV_RENDER_PARAM_SW_POINTER,
                    self.pixels.as_mut_ptr().cast(),
                ),
                param(
                    mpv::mpv_render_param_type_MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                    (&mut block as *mut i32).cast(),
                ),
                param(0, ptr::null_mut()),
            ];
            match unsafe {
                check(mpv::mpv_render_context_render(
                    self.render,
                    params.as_mut_ptr(),
                ))
            } {
                Ok(()) => {
                    for pixel in self.pixels.as_chunks_mut::<4>().0 {
                        pixel[3] = 255;
                    }
                    output.frame.clone_from(&self.pixels);
                }
                Err(error) => output.error = error,
            }
        }
        output.audio.extend(self.audio.lock().unwrap().drain(..));
        let denied = self.proxy.take_error();
        if !denied.is_empty() {
            output.error = denied;
        }
    }
}
fn read_audio(mut reader: File, output: Arc<Mutex<VecDeque<Vector2>>>, stop: Arc<AtomicBool>) {
    let mut pending = Vec::with_capacity(8192);
    let mut buffer = [0u8; 4096];
    while !stop.load(Ordering::Acquire) {
        for _ in 0..16 {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => pending.extend_from_slice(&buffer[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => return,
            }
        }
        let count = pending.len() / 8;
        {
            let mut output = output.lock().unwrap();
            for bytes in pending[..count * 8].as_chunks::<8>().0 {
                let left = f32::from_le_bytes(bytes[..4].try_into().unwrap());
                let right = f32::from_le_bytes(bytes[4..].try_into().unwrap());
                output.push_back(Vector2::new(
                    if left.is_finite() {
                        left.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    },
                    if right.is_finite() {
                        right.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    },
                ));
            }
            while output.len() > 12000 {
                output.pop_front();
            }
        }
        pending.drain(..count * 8);
        thread::sleep(Duration::from_millis(10));
    }
}
impl Worker {
    fn new(width: i32, height: i32, local_fixture: bool) -> Result<Self, String> {
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return Err("Video service is shutting down".into());
        }
        if !(64..=1280).contains(&width) || !(64..=720).contains(&height) {
            return Err("Video surface exceeds pixel budget".into());
        }
        if PLAYERS.fetch_add(1, Ordering::AcqRel) >= 4 {
            PLAYERS.fetch_sub(1, Ordering::AcqRel);
            return Err("At most four active decoders per client".into());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let running = stop.clone();
        let (sender, receiver) = mpsc::sync_channel(32);
        let output = Arc::new(Mutex::new(Output::default()));
        let shared = output.clone();
        let join = thread::Builder::new()
            .name("media-decoder".into())
            .spawn(move || {
                if !local_fixture {
                    if let Err(error) = isolated::run(width, height, &receiver, &running, &shared) {
                        let mut output = shared.lock().unwrap();
                        if output.error.is_empty() {
                            output.error = crate::error_message::bounded(error);
                        }
                    }
                    PLAYERS.fetch_sub(1, Ordering::AcqRel);
                    return;
                }
                match Player::new(width, height, local_fixture) {
                    Ok(mut player) => {
                        while !running.load(Ordering::Acquire) {
                            for command in receiver.try_iter().take(32) {
                                if matches!(command, Command::Seek(_) | Command::Load(_)) {
                                    shared.lock().unwrap().audio.clear();
                                }
                                if let Err(error) = player.apply(command) {
                                    shared.lock().unwrap().error = error;
                                }
                            }
                            let mut decoded = Output::default();
                            player.poll(&mut decoded);
                            {
                                let mut output = shared.lock().unwrap();
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
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                    }
                    Err(error) => shared.lock().unwrap().error = error,
                }
                PLAYERS.fetch_sub(1, Ordering::AcqRel);
            })
            .map_err(|error| {
                PLAYERS.fetch_sub(1, Ordering::AcqRel);
                error.to_string()
            })?;
        let join = Arc::new(Mutex::new(Some(join)));
        let mut records = WORKERS.lock().unwrap();
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            stop.store(true, Ordering::Release);
            drop(records);
            if let Some(join) = join.lock().unwrap().take() {
                let _ = join.join();
            }
            return Err("Video service is shutting down".into());
        }
        records.retain(|record| {
            let mut join = record.join.lock().unwrap();
            if join.as_ref().is_some_and(|j| j.is_finished()) {
                let _ = join.take().unwrap().join();
            }
            join.is_some()
        });
        records.push(WorkerRecord {
            stop: stop.clone(),
            join: join.clone(),
        });
        Ok(Self {
            stop,
            sender,
            output,
            join,
        })
    }
}

#[path = "video_process.rs"]
mod isolated;
pub fn media_worker_main() -> Result<(), String> {
    isolated::entry().map_err(|error| error.to_string())
}

#[derive(GodotClass)]
#[class(base=RefCounted,init)]
struct VideoDecoder {
    worker: Option<Worker>,
    width: i32,
    height: i32,
    base: Base<RefCounted>,
}
#[godot_api]
impl VideoDecoder {
    #[func]
    fn open_url(&mut self, url: GString, width: i32, height: i32) -> GString {
        let url = url.to_string();
        if url.len() > 4096
            || url.contains(['\0', '\n', '\r'])
            || !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return "Only validated HTTP(S) video URLs are accepted".into();
        }
        if self.width == width
            && self.height == height
            && let Some(worker) = &self.worker
        {
            return match worker.sender.try_send(Command::Load(url)) {
                Ok(()) => GString::new(),
                Err(_) => "Video command queue is full".into(),
            };
        }
        self.worker = None;
        match Worker::new(width, height, false) {
            Ok(worker) => {
                let _ = worker.sender.try_send(Command::Load(url));
                self.worker = Some(worker);
                self.width = width;
                self.height = height;
                GString::new()
            }
            Err(e) => e.as_str().into(),
        }
    }
    #[func]
    fn pause(&self, value: bool) {
        if let Some(w) = &self.worker {
            let _ = w.sender.try_send(Command::Pause(value));
        }
    }
    #[func]
    fn seek(&self, seconds: f64) {
        if seconds.is_finite()
            && seconds >= 0.0
            && let Some(w) = &self.worker
        {
            let _ = w.sender.try_send(Command::Seek(seconds));
        }
    }
    #[func]
    fn speed(&self, value: f64) {
        if value.is_finite()
            && (0.25..=2.0).contains(&value)
            && let Some(w) = &self.worker
        {
            let _ = w.sender.try_send(Command::Speed(value));
        }
    }
    #[func]
    fn position(&self) -> f64 {
        self.worker
            .as_ref()
            .map_or(0.0, |w| w.output.lock().unwrap().position)
    }
    #[func]
    fn frame(&self) -> PackedByteArray {
        self.worker.as_ref().map_or_else(PackedByteArray::new, |w| {
            let frame = std::mem::take(&mut w.output.lock().unwrap().frame);
            PackedByteArray::from(frame.as_slice())
        })
    }
    #[func]
    fn audio(&self) -> PackedVector2Array {
        self.worker
            .as_ref()
            .map_or_else(PackedVector2Array::new, |w| {
                let mut data = w.output.lock().unwrap();
                let count = data.audio.len().min(4800);
                PackedVector2Array::from(data.audio.drain(..count).collect::<Vec<_>>().as_slice())
            })
    }
    #[func]
    fn error(&self) -> GString {
        self.worker.as_ref().map_or_else(GString::new, |w| {
            std::mem::take(&mut w.output.lock().unwrap().error)
                .as_str()
                .into()
        })
    }
    #[func]
    fn close(&mut self) {
        self.worker = None;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn extension_shutdown_joins_workers() {
        let output = std::process::Command::new("timeout")
            .arg("15")
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "video::tests::shutdown_child",
                "--ignored",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    #[ignore = "isolated process: shutdown is terminal for this library instance"]
    fn shutdown_child() {
        let workers: Vec<_> = (0..4)
            .map(|_| super::Worker::new(64, 64, true).unwrap())
            .collect();
        super::shutdown();
        assert_eq!(super::PLAYERS.load(std::sync::atomic::Ordering::Acquire), 0);
        assert!(super::WORKERS.lock().unwrap().is_empty());
        assert!(super::Worker::new(64, 64, true).is_err());
        drop(workers);
        super::shutdown(); // Idempotent even after the Godot objects drop.
    }
    use super::*;
    use std::{io::Write, net::TcpListener};
    fn guarded_http_reply(response: Vec<u8>) -> Output {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(&response);
        });
        let output = thread::spawn(move || {
            let proxy = MediaProxy::with_fixture(Some(origin)).unwrap();
            let mut player = Player::with_proxy(64, 64, false, proxy).unwrap();
            player
                .apply(Command::Load(format!("http://{origin}/source.m3u")))
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut output = Output::default();
            while output.error.is_empty() {
                player.poll(&mut output);
                assert!(
                    std::time::Instant::now() < deadline,
                    "Rejected media did not report failure"
                );
                thread::sleep(Duration::from_millis(10));
            }
            output
        })
        .join()
        .unwrap();
        server.join().unwrap();
        output
    }
    #[test]
    fn private_redirect_does_not_connect() {
        let secret = TcpListener::bind("127.0.0.1:0").unwrap();
        secret.set_nonblocking(true).unwrap();
        let address = secret.local_addr().unwrap();
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{address}/secret.mp4\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let hls = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2,\nhttp://{address}/secret.ts\n#EXT-X-ENDLIST\n"
        );
        let playlist = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{hls}",
            hls.len()
        );
        for response in [redirect, playlist] {
            let output = guarded_http_reply(response.into_bytes());
            assert!(!output.error.is_empty());
            assert_eq!(
                secret.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
    #[test]
    fn network_playlist_cannot_open_local_file() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("private.mp4");
        std::fs::write(&file, b"PRIVATE LOCAL CANARY").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(raw >= 0);
        let mut watch = unsafe { File::from_raw_fd(raw) };
        assert!(
            unsafe {
                libc::inotify_add_watch(
                    watch.as_raw_fd(),
                    path.as_ptr(),
                    libc::IN_OPEN | libc::IN_ACCESS,
                )
            } >= 0
        );
        for body in [
            format!("#EXTM3U\n#EXTINF:1,Private\nfile://{}\n", file.display()),
            format!("# mpv EDL v0\nfile=file://{}\n", file.display()),
        ] {
            let mime = if body.starts_with("# mpv EDL") {
                "application/octet-stream"
            } else {
                "audio/x-mpegurl"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let output = guarded_http_reply(response.into_bytes());
            assert!(!output.error.is_empty());
            let mut events = [0u8; 1024];
            assert_eq!(
                watch.read(&mut events).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock,
                "A remote playlist read the local canary"
            );
        }
    }
    #[test]
    fn software_decoder_and_pcm_route() {
        let temp = tempfile::Builder::new()
            .prefix("ssc-video-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("test.mkv");
        let status = std::process::Command::new("ffmpeg")
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
                "1",
                "-c:v",
                "mpeg4",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&file)
            .status()
            .expect("Run in the locked native shell with ffmpeg");
        assert!(status.success());
        assert!(Worker::new(1281, 720, true).is_err());
        for _ in 0..4 {
            let worker = Worker::new(64, 64, true).unwrap();
            worker
                .sender
                .send(Command::Load(file.to_str().unwrap().into()))
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            loop {
                {
                    let output = worker.output.lock().unwrap();
                    assert!(output.error.is_empty(), "mpv: {}", output.error);
                    if output.frame.len() == 64 * 64 * 4
                        && output.audio.iter().any(|v| v.x.abs() > 0.001)
                    {
                        break;
                    }
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "No decoded video/PCM arrived from libmpv"
                );
                thread::sleep(Duration::from_millis(20));
            }
            worker.stop.store(true, Ordering::Release);
            worker.join.lock().unwrap().take().unwrap().join().unwrap();
        }
        let failed = Worker::new(64, 64, true).unwrap();
        failed
            .sender
            .send(Command::Load(
                temp.path()
                    .join("missing.mkv")
                    .to_string_lossy()
                    .into_owned(),
            ))
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while failed.output.lock().unwrap().error.is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "Invalid media did not produce an error"
            );
            thread::sleep(Duration::from_millis(20));
        }
        failed.stop.store(true, Ordering::Release);
        failed.join.lock().unwrap().take().unwrap().join().unwrap();
        assert_eq!(
            PLAYERS.load(Ordering::Acquire),
            0,
            "Decoder slot leaked after shutdown"
        );
    }
}
