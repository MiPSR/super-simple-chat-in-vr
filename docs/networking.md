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
cryptographic/security audit or a complete aggregation/load policy.

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
