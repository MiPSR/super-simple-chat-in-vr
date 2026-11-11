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

## Process boundary

Production `VideoDecoder` workers now launch the packaged `ssc-media-worker`
executable, not libmpv in Godot's address space. Only a bounded command array,
exact-size RGBA frames, capped stereo PCM, finite timing and 2,048-byte diagnostics
cross the IPC socket. Lengths are validated before allocation. Each full IPC
exchange has a three-second deadline, including partial reads/writes; stalled or
crashed workers report a surface failure. Teardown kills/reaps the worker process
group (including resolver descendants); parent death also kills the worker.
The pre-exec parent guard checks the expected parent after installing the death
signal, rejecting startup if the client disappeared before signal registration.

The helper requires Linux 6.12+ with Landlock ABI 6 enabled, and fails closed if
isolation cannot be installed. Landlock permits read/execute of the public Nix
store, narrowly selected DNS/certificate/time/CPU files and null/random/owned
pipe paths. Other files, including client credentials and assets, cannot be read
or written. Signal/abstract-Unix-socket scopes prevent interaction outside the
sandbox; seccomp additionally denies ptrace, foreign memory writes/descriptor access,
namespace/mount escape, BPF, perf, userfaultfd and io_uring APIs. The existing
socket guard and checked HTTP relay still enforce ordinary public-media routing.
The notification broker retains `process_vm_readv` for checking socket addresses
in its own descendants; Landlock's ptrace-domain rule denies reads of the parent
client, with an explicit isolated regression probe.

Resource limits cap each process/descendant's address space at 2 GiB, open file
descriptors at 128, regular-file output at zero bytes, and disable core dumps.
Only standard descriptors survive exec; the helper receives a minimal environment
and no account credentials. The production launcher supplies an immutable helper
path. In the edit shell, `cargo build -p super-simple-chat-native` builds both the
extension and helper; `SSC_MEDIA_WORKER` points to the debug executable.

Tests cover filesystem canaries, forbidden signals, memory limits, malformed
IPC, expired exchange deadlines, nonfinite PCM, the pre-exec parent-death race,
actual helper initialization/HTTP-fixture decode, descendant-held descriptor
closure on group teardown, and `setsid`/`setpgid` denial. These pass locally. Results must not be
inferred from the historical in-process codec tests below.

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

This is still not a complete hostile-native-code resource guarantee. Address-space
limits are per process, not an aggregate cgroup memory/process/CPU limit across
all resolver descendants. The helper's relay is in the same address space as its
decoder; a native exploit could potentially alter relay policy inside that
process. The IPC/process/filesystem boundary protects Godot from ordinary decoder
crashes and forbidden file access, but does not establish protection from kernel
exploits or comprehensive containment of arbitrary native malware. Importer peaks
and creator Luau isolation remain separate mechanisms.

Normal teardown and detected helper failure kill the private process group.
Abrupt death of the entire client delivers the parent-death signal to the helper,
not automatically to every resolver descendant: Linux clears that setting on
fork. The ordinary resolver wrapper has its own timeout, but immediate aggregate
descendant cleanup after client death still needs a supervisor/cgroup boundary.

Multi-client decoded-output synchronization, long sessions, restricted/removed
external sources and complete resource budgets remain release requirements.
