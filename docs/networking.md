# Experimental game protocol and reconnect behavior

Use matching client/server builds. The current protocol is an implemented
project-owned subset, not a compatibility promise for older development builds.

## Transport

Low-level ENet uses three channels: reliable control/state/text on 0, sequenced
unreliable poses/inputs on 1, and unsequenced unreliable voice on 2. Full/half/
quarter send and receive preferences are independent. Reliable state changes are
not downsampled by pose preferences. Received transforms are interpolated between
samples or held; clients do not extrapolate authoritative physics.

HTTPS must protect production game-grant delivery. A grant supplies a one-use
ticket and independent per-connection random master secret. ENet packets use the
tested version-1 AES-256-CBC/PKCS#7 encrypt-then-HMAC-SHA256 envelope, not assumed
Godot/Rust DTLS interoperability. Direction/purpose keys are derived separately.
The authenticated header includes version, channel, direction and a little-endian
u64 counter, followed by a random 16-byte IV, ciphertext and 32-byte MAC. MACs are
verified before decryption. Each channel rejects counters at or below its latest
authenticated value; older reordered envelopes are discarded too. Large reliable
JSON snapshots use bounded application fragments. This is not a completed
cryptographic/security audit or full load acceptance.

## Challenge handshake

Admission is challenge-based. On ENet `Event::Connect` the server speaks first
with a cleartext 17-byte reliable channel-0 packet: a 0 tag byte followed by a
fresh random 16-byte challenge. The client replies with the 64-byte one-use
ticket, the 16-byte challenge echoed, and a sealed kind-1 envelope whose
payload is the ticket bytes, so the cleartext ticket is MAC-covered end to end.
The server rejects the reply unless the echoed challenge matches this
connection's pending value; challenges are per-connection, so a captured
handshake cannot be replayed onto a fresh connection to burn someone else's
ticket, and pending handshakes expire after five seconds. The realtime protocol
version advertised with game grants is now 2.

## Per-peer rates (new implementation, unverified)

`SSC_NETWORK_MIN_HZ`/`SSC_NETWORK_MAX_HZ` (defaults 1/120) bound both directions
independently. Integer tick divisors are chosen within those bounds, so a 90 TPS
server can announce fractional rates such as 22.5 Hz. Impossible configurations
(for example exactly 40 Hz at 60 TPS) fail startup instead of silently violating
the limits. Requested full/half/quarter factors are clamped to this policy.

`SSC_NETWORK_ADAPT=true` samples ENet RTT/reliable-packet loss every five seconds.
Three consecutive poor samples (RTT >=350 ms or loss >=8%) double each peer's
divisors within the minimum rate. Six consecutive healthy samples (RTT <=180 ms
and loss <=2%) recover toward that peer's original requested/clamped rates.
Intermediate samples reset both streaks. No peer can change simulation TPS or
another peer's schedule. Reliable authenticated kind 26 announces the effective
`send_hz`/`receive_hz`; the client adjusts its tracking cadence, not rendering.
Set adaptation to `false` to disable this policy.

`SSC_NETWORK_AGGREGATION=latest` sends complete latest samples at each peer's
receive cadence. Optional `suppress-unchanged` additionally hashes current
presentation state and skips identical disposable packets. It resends a complete
sample at least once per second (or the next negotiated send opportunity), never
uses lossy delta baselines, averages poses or predicts missing input. Playing
media still changes position; stopped-media sampling timestamps alone do not
force updates. Reliable graph/state/events remain independent of this setting.
The NixOS module exposes matching `simulationTps`, `networkMinimumHz`,
`networkMaximumHz`, `networkAdaptation` and `networkAggregation` options.

This batch has not been compiled/tested; deterministic policy and client decode
regressions are written, and packet-loss/load acceptance remains outstanding.

The optional aggregation path now repeats recently changed player/body samples
for one second and emits a complete keyframe at least once per second. Old static
samples need not occupy every intermediate packet. Playing-media timing is
refreshed in keyframes and reliable controls, rather than forcing the entire idle
graph to transmit at pose rate. This is not a delta baseline or prediction:
changed samples repeat across packet loss and keyframes repair an omitted final
sample. Partial player updates cannot retire idle proximity voice streams;
complete reliable rosters still remove departed/blocked players.

Authenticated upstream data is capped at 256 KiB per peer per second, before
envelope authentication work. Exceeding byte/message limits retires membership
immediately rather than leaving a disconnected peer counted for the timeout.
Independent pose/body-tracker/control pacing permits a two-sample jitter burst
without inadvertently halving well-paced input. New pacing/partial-roster cases
are unexecuted.

Large disposable pose graphs now use bounded kind-27 multipart samples instead
of silently exceeding the 60-KiB packet ceiling. Reassembly retains only the
newest sample, caps it at 256 parts / 6 MiB, expires incomplete data after two
seconds, and applies nothing until complete. A later sample supersedes missing
parts rather than retransmitting obsolete transforms. Reliable kind-10 and
disposable kind-27 envelopes are explicitly excluded from recursive fragmentation;
this also fixes a large-reliable-envelope recursion case found during review.
ENet itself is limited to 64-KiB packets and 256-KiB waiting data per server peer,
before application authentication. New fragmentation cases are unexecuted.

## Stale avatar input

A player sample keeps retransmitting for one second of ticks after its last
change; an avatar whose input stops is then no longer transmitted, and
receivers hold the last known pose instead of predicting it. A session that
authenticates no packets at all for 15 seconds is killed and its membership
removed, so a stalled client cannot hold a seat indefinitely.

## Focus loss and resume (new implementation, unverified)

Reliable authenticated kind 28 reports the connection owner's `active` boolean.
Suspending input immediately releases that player's leases/held props and avatar
constraints with reason `focus_lost`, clears body-tracker input, and holds the
last authoritative pose. Suspended sessions cannot send gameplay/tracking,
proximity/radio voice or contact/action requests. World scripts also cannot lease
an object to an input-suspended player. Deferred interaction callbacks recheck
this state. Resuming permits new input but never recreates old constraints.

The client watches desktop application focus or OpenXR session focus/presence,
not desktop-window focus while XR is active. It cancels pending radial gestures,
stops movement/control transmission, mutes capture, clears radio transmission
selection and hides the 3D phone. Local XR camera tracking remains independent;
rendered remote poses are not extrapolated. Resume restores permitted input and
phone visibility, but the microphone stays muted until the user explicitly
enables it again. OpenXR signals/current state avoid assuming the initial session
is already focused. New headless transition fixtures are unexecuted; real
headset/dashboard/focus/resume acceptance remains outstanding.

## Voice stream epochs

Client-to-server proximity/call audio still supplies a little-endian u32 serial,
u64 timestamp and one 20 ms Opus packet. The server validates duration, encoded
byte/rate limits, membership, blocking and recipient routing. Clients do not
choose sender identity or stream epoch.

Server-to-client decrypted kind-20 voice frames contain:

| Field | Encoding |
|---|---|
| Kind | u8, 20 |
| Flags | u8, `0x80` marks epoch format; bit 0 is non-positional call routing |
| Sender length | u8, 1–64 UTF-8 bytes |
| Sender | UTF-8 bytes |
| Stream epoch | little-endian positive u64, bounded to signed-64 range |
| Audio serial | little-endian u32 |
| Timestamp | little-endian u64 |
| Audio | Opus packet |

Epochs increase on each successful game connection within a server process.
Receivers retire the old generator/decoder when an epoch increases and reject
older epochs before applying route/gain metadata. Within an epoch, duplicate or
old serials are rejected with standard half-range u32 wrap handling. A new
connection can restart serials without being muted behind the old high-water
mark. Radio JSON carries the same server-assigned epoch as a decimal string to
avoid Godot JSON floating-point rounding. Legacy unmarked kind-20 payloads are
not interpreted as epoch frames. No creator command can select an epoch or enable
microphone capture.

## Admission and reconnect

New-instance limits/construction and existing-instance permissions/capacity are
checked before leaving an existing instance. Failure preserves the previous
membership, shared state and authenticated connection. Private invitations are
user-bound, expiring and consumed only after successful admission; outsiders do
not see private listings. Private authorization precedes capacity diagnostics.

One account has one live game connection. A successful live takeover retires its
old connection only after admission; later disconnect events from that old peer
cannot remove the replacement. Same-instance live reconnect resnapshots the
existing player/instance rather than recreating it, even in a full private
instance where that member's invitation was already consumed. Scores, selected
avatar/actions and authoritative pose survive. Old control/hand/contact
constraints are released with reason `reconnect`, and new tracking/input must
reacquire control. Continuous membership does not emit another join/leave pair.

This does not resurrect an instance destroyed after its last participant left.
Synthetic and real-Godot tests cover rejection preservation, private invitation
boundaries, live takeover, stale disconnects and voice restart. Packet-loss/load,
long-session and real XR acceptance remain required.

New unverified client work now stages a second transport while retaining the old
authenticated one. Rejection leaves the old connection, cipher and resources
untouched. Success transfers the admitted transport and renegotiates audio without
briefly treating a successful takeover's old-peer retirement as a user leave.
Real disconnect/timeout emits explicit teardown instead of leaving old scripts,
media and constraints running indefinitely. Main instance identity, not just the
asset hash, controls runtime reset, including switches between instances using
the same immutable world version.

Asset loaders now cancel by generation: stale HTTP callbacks and completed GLTF
results cannot publish an abandoned scene. A running importer retires without a
UI-thread join, releases its pin, and then processes the newest queued load.
Only final node teardown joins a still-running worker. New staged-transition and
cancelled-generation fixtures are unexecuted.

## Revision snapshots

Revision resnapshots (kind 2 without the join-negotiation fields) omit
`manifest` when the receiving peer already holds that exact world version; the
immutable world definition is not re-sent just because instance state changed.
A client must keep its current world definition when a snapshot carries no
manifest. Admission snapshots and world switches still ship the full manifest.

## Realtime SQLite reads

Per-tick SQLite reads on the authoritative realtime thread — session validity,
call-mode preferences, blocks and touch consent — go through a bounded TTL
cache with a 5-second window, pruned every tick and reset wholesale when a map
exceeds 4,096 entries. Block and preference changes therefore surface within
one TTL window; session revocation is rechecked each tick through the same
cache and surfaces within 15 seconds worst case. Access semantics are
unchanged.

## Shared physics budget

The authoritative tick carries one shared 8-millisecond physics budget across
all instances. An instance that exhausts the shared allowance freezes its
bodies for that tick — its scene step and contact resolution are skipped —
rather than stalling every other player's tick. The budget resets each tick;
creator/script work keeps its existing separate queues and watchdogs.
