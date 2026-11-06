# Experimental compatibility contracts

Application, HTTP, package, and realtime versions currently start at 1 and are
not declared stable. Unsupported envelope versions are rejected. Breaking
changes will increment the relevant contract; creator examples must be updated
together with their manifest/API definitions.

## Realtime

The transport is native low-level ENet with three channels, not Godot's
MultiplayerAPI. Channel 0 is reliable ordered state/events/commands; channel 1
is unreliable sequenced tracking, object transforms and continuous controls;
channel 2 carries time-sensitive Opus audio. ENet compression and Godot-specific
DTLS are not enabled.

HTTPS issues a random one-use 60-second ticket and a random 32-byte master key.
Only explicit loopback development may use HTTP. The first ENet packet is the
64-byte ASCII ticket followed by an authenticated client envelope of kind 1.
A malformed authenticated handshake does not consume another user's ticket.

Envelope bytes: version u8 (=1), channel u8, direction u8 (0 client, 1 server),
monotonic per-direction/per-channel u64 LE sequence, random 16-byte IV,
AES-256-CBC/PKCS#7 ciphertext, and 32-byte HMAC-SHA256 over every preceding
envelope byte. Ciphertext begins with a u8 message kind. Independent keys are
SHA256(master || direction-u8 || purpose-u8), purpose 0 encryption / 1 MAC.
The MAC is checked before decryption, and replay/outdated sequences are rejected.
This provides application-payload confidentiality/authentication, not encrypted
ENet headers, forward secrecy, traffic-analysis resistance, or DoS protection.

Kinds: 1 authenticate; 2 authoritative snapshot; 3 pose/transform sample;
4 text; 5 object interaction; 6 avatar contact; 7 posture; 8 avatar selection;
9 leave; 10 reliable fragment; 11 authoritative object events; 12 control action;
13 declared avatar action; 14 body-tracker inputs; 20 Opus; 21 private calls;
22 media command; 23 monotonic clock probe; 24 radio voice; 25 radio data;
255 error. Except client pose and
voice samples, payloads are bounded UTF-8 JSON.
This is an explicit binary envelope with JSON state, not a compact finalized
binary state schema; bandwidth optimization remains work before release.

Client kind-3 samples are exactly 84 bytes: head, left hand, right hand, each
seven f32 LE values (position xyz and quaternion xyzw). Non-finite/excessive
values are rejected. Server messages identify tick and state revision; clients
interpolate only known samples and hold the last pose when samples run out.
Local headset and controller tracking never waits for these samples.

Kind-12 actions: `lease`, `release`, `grab` (hand 0 left / 1 right), `drive`
(throttle and turn in [-1,1]), and `fire`, each naming a `target` object ID.
Drive uses channel 1; discrete transitions use channel 0. Clients do not submit
damage, scores, projectile transforms, or spawn definitions.

An envelope is at most 65,536 bytes. Large reliable JSON messages are split into
kind-10 fragments, containing `id`, zero-based `index`, `count` (at most 256),
original `kind`, and up to 24,000 bytes of hex-encoded `data`. Fragments are
reassembled in reliable order with a bounded allocation. Snapshots and subsequent
reliable changes are serialized by one authoritative thread, so snapshots form
a consistent baseline before later changes on that channel.

Kind 22 uses `{target,action,source?,value?}`. Actions are `load`, `play`, `pause`,
`seek`, `rate`, `stop`; only the instance owner or players explicitly permitted
by a declared surface's `public_controls` can issue them. Server scripts with
the media capability use the same validation. Snapshot `media` maps surface IDs
to `{source,position,playing,rate,generation,sampled_at}`. Position is sampled
against a monotonic server clock, not the simulation TPS. Transient media state
is never persisted. Periodic kind-3 state also carries current media targets.

Kind 23 echoes `{sent}` as `{sent,server}`. Clients use the lowest-RTT probe to
estimate the server monotonic-clock offset and age media targets. This estimates
network delay, not perfect clock synchronization. Clients perform coarse seeks
for drift over 750 ms and small local speed adjustments otherwise.

Kind 20 client payload: u32 LE sequence, u64 LE capture timestamp, one Opus
packet (20 ms at 48 kHz). Server payload prepends route u8 (0 positional,
1 non-positional), sender-ID byte length u8, and sender-ID UTF-8 bytes. A frame
is routed once to each authorized recipient, never through both paths.

Kind 24 is unsequenced encrypted channel-2 radio voice. Kind 25 is reliable
channel-0 protected radio data. Both use bounded JSON `{target,mode,data}`:
`voice` contains hex Opus (20 ms at 48 kHz under the radio bitrate policy),
`ft8` contains at most 40 printable ASCII bytes, and `sstv` contains exactly
3,072 grayscale bytes as hex (64×48). Voice is rejected on the digital path and
vice versa. Server deliveries identify receiver `radio`, `channel`, authenticated
`sender`, `mode`, `data`, and a server-owned `sequence`. Clients cannot choose
arbitrary recipients or impersonate a sender. This protected representation is
not an external modem protocol. Initial negotiation includes radio bitrate,
allowed channels and filter/noise policy.
