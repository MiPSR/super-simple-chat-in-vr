# Media isolation and remaining limits

Public media is fetched by each client; the game server stores only its source
and authoritative timeline. Packaged standalone audio has a separate Opus-only
policy; video audio formats do not expand that policy.

## Implemented layers

- Shared conservative IP classification rejects private, loopback, reserved,
  documentation and transition ranges, including mapped IPv4 bypasses.
- A bounded local HTTP/CONNECT relay checks every resolved address before
  connecting to a pinned IP. TLS remains end-to-end. DNS uses asynchronous
  Hickory resolution with system nameservers and a five-second lookup timeout,
  rather than uncancellable libc `getaddrinfo` work. Relay shutdown cancels
  outstanding client tasks and drops its runtime. If system DNS configuration
  is unavailable (as in offline Nix checks), literal IPs are still checked;
  domain lookups fail closed instead of using a fallback public nameserver.
- A disposable decoder thread installs Linux seccomp socket filtering inherited
  by its children, including yt-dlp. Private socket connections, bind/listen,
  explicit datagrams, sendmsg and io_uring paths are rejected. Godot's main,
  XR and realtime-network threads are not filtered.
- mpv user configuration and arbitrary scripts are disabled. Its maintained
  yt-dlp hook is retained with user configuration, cache and plugins disabled;
  a packaged timeout bounds resolver wall-clock time.
- Load hooks accept HTTP(S) or bounded, known network-only EDL wrappers.
  Unsafe playlist loading and automatic adjacent-file loading are disabled;
  FFmpeg protocols are allowlisted. mpv's network-origin policy also prevents
  remote playlists from opening filesystem sources.
- Four native decoder slots, bounded frame/PCM queues and output resolution
  limits constrain presentation resources. Worker handles are retained and
  joined at extension shutdown rather than detached across library unload.

`SSC_TEST_MEDIA_ORIGIN` permits one literal loopback HTTP fixture socket for
integration tests only. The production launcher clears it along with
`no_proxy`/`NO_PROXY`. Bare development launches must clear proxy bypass values
themselves or protected playback refuses to start.

## Evidence and limits

Offline tests exercise inherited thread/exec filtering, unaffected unrelated
networking, private redirects/HLS references without opening the private socket,
and M3U/EDL references without opening or reading a watched local canary.
Single-client public MP4, HLS and YouTube decoding, positional PCM,
pause/seek/resume and cleanup have passed against a debug extension. Package
checks and acceptance are recorded separately in [status.md](status.md).
An opt-in external-network ENet run also passed two separate Godot clients
decoding public MP4 and YouTube, converging to authoritative server time,
pausing/seeking, reconnecting a late joiner and resuming. These are short
headless checks, not long-session or real-headset acceptance.

This is **not** a general subprocess/filesystem sandbox. Native decoder or
resolver vulnerabilities, hostile processes sharing client memory/file
descriptors, and comprehensive CPU/memory/process isolation are not covered.
In particular, seccomp user-notification socket checks are not a substitute
for a hardened process boundary against malicious native code. The resolver
timeout is not a cgroup memory limit. Output dimensions do not bound all
internal codec allocations. Creator Luau isolation is a separate mechanism.

Multi-client decoded-output synchronization, long sessions, restricted/removed
external sources and complete resource budgets remain release requirements.
