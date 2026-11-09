# Resource admission (new batch, unverified)

These controls are implemented but have **not been compiled or tested**. They
bound specific owned resources, not total process RSS, importer peaks, GPU-driver
allocations or every native decoder allocation. Hardware/load/security acceptance
remains required.

## Client rendering

Native RAII reservations apply across imported worlds, avatars, resident prefabs
and instantiated prefab visuals in one client process:

| Resource | Aggregate ceiling |
|---|---:|
| Imported/prefab nodes | 32,768 |
| Conservative surface passes | 16,384 |
| Geometry elements | 4,000,000 |
| Estimated owned RGBA texture bytes including mips | 512 MiB |
| Skeleton bones | 4,096 |
| Imported lights | 64 |

Each surface is charged three passes (base, optional outline, optional matcap)
regardless of the current material tier. Geometry elements are the greater of
vertex/index counts per ArrayMesh surface, or generated primitive vertices.
Textures are deduplicated within an import and charged at `width*height*16/3`.
Shared textures are not charged again for prefab instances; those instances keep
the source PackedScene alive so its asset/texture reservation cannot disappear
early. Instance geometry/pass/node work is still charged. Costs are conservative
admission estimates, not measured VRAM or draw-call counts.

At most four registered lights compute shadows at once; Tier 0 suppresses all
computed shadows. Waiting eligible lights are promoted when an earlier light is
hidden, disabled or retired. Authored light energy is clamped to 8, positional
range to 100 metres and directional shadow distance to 100 metres. Script requests
remain stored across tier changes but cannot bypass those limits. Fallback avatar
spheres use deliberately low tessellation instead of Godot's high default.

An imported scene additionally has a 4,096-node ceiling. Failed asset admission
does not emit the new loaded scene. An over-budget prefab instance uses the
ordinary authoritative-size box fallback, reports the error and avoids repeated
instantiate/reject loops for that same prefab resource. Gameplay still uses the
server entity and collision proxy; a rendering tier never disables scripts.
Reservations retire with the owning scene/PackedScene, including queued deletion.

Scene construction currently precedes admission, so this does not bound transient
importer allocations. A stronger worker-process/importer memory boundary remains
unfinished. Host-generated fallback/UI/IK objects are not fully included in this
imported-scene accounting and require separate aggregate CPU/GPU acceptance.

## Packaged audio

Native compressed-clip reservations allow at most **128 open clips and 128 MiB**
of owned compressed audio across all world/avatar packages, including idle clips.
Reservation precedes the native byte copy and libopusfile open. Parse failures,
decode failures, explicit close and object teardown release the reservation.
The existing per-clip 32-MiB limit remains. PCM feed buffers are independently
bounded; real-time mixer/decoder CPU still requires aggregate scheduling work.

## Other established boundaries

The existing client/server creator-VM ceilings and per-VM heaps/watchdogs remain;
the client now additionally has a recurring-work scheduler. Live callbacks are
queued in order (128 jobs / 2 MiB per runtime, 256 KiB per event) and advanced
round-robin across active runtimes, at most 256 work units and a 4-ms wall-time
target per render frame. Context sampling, one VM invocation and individual
presentation commands are separate units. Pending ticks coalesce and accumulate
at most one second of elapsed time; timers do not burst-catch-up after suspension.
Frame-budget exhaustion yields rather than disabling an innocent script.

This is a cooperative target, not a hard frame deadline: a single native VM
invocation still has its existing CPU watchdog and a grouped rendering command
can overshoot before the scheduler yields. Top-level package initialization and
scene construction remain synchronous. Explicitly paused/detached host fixtures
retain synchronous dispatch for deterministic tools. New scheduler tests are
unexecuted.

The live server now enables a matching fair, queued creator scheduler across
instances: at most 256 units and a cooperative target of half a simulation tick
(capped at 8 ms). Physics still advances every authoritative tick independently
of client network rates. Pending tick callbacks coalesce, while ordered event
jobs retain progress across scripts instead of silently skipping the remainder
when one callback consumes a frame. Queues are bounded to 128 jobs / 2 MiB per
instance and 256 KiB per job. Individual Lua calls retain the 5-ms CPU watchdog.
Deferred interaction callbacks recheck membership, target existence and reach;
their authenticated initiator exists only for that callback, not later timers.
Direct registry fixtures retain synchronous execution unless they explicitly
enable scheduling. Constructor/physics/native-command costs are not preemptible.

Server entities additionally require process-wide RAII admission, capped at
32,768 retained entities across all instances (in addition to per-instance entity
and dynamic-body limits). Publication is unaffected. Initial-world construction
reserves before physics insertion; failed admission preserves old membership.
Destroy/instance teardown release ownership, and retained entity clones keep the
reservation until the last clone drops. Administrator health includes usage.

Creator diagnostics are capped at 2,048 UTF-8 bytes and strip terminal control
characters other than newline/tab. Formatting stops at the bound rather than
first constructing an arbitrarily large error string. Repeated command/backlog
diagnostics are summarized at most once per second per instance, and the NixOS
service also sets a journal rate limit. New regression sources are unexecuted.

Avatar selection is idempotent and preserves existing discrete actions when the
same immutable avatar is reselected. Loading a different avatar has a two-second
per-account cooldown that survives world changes/reconnects; removing an avatar
is always permitted. Media surfaces have a one-second load cooldown and at most
16 non-noop control changes per second. Stop remains available even at capacity,
and no-op controls do not increment the authoritative revision.

Video retains its four
decoder ceiling, fixed output dimensions, finite demux/PCM buffers and bounded
worker threads, but has no general decoder/resolver process memory sandbox.
See [media-security.md](media-security.md), [asset-storage.md](asset-storage.md)
and [asset-cache.md](asset-cache.md) for their distinct boundaries.

Optional acoustic obstruction retains at most 512 weak source records and makes
at most four ray queries per physics tick, with a 200-ms per-source minimum
interval. Listener-zone noise uses one lazy 100-ms generator, bounded to 4,800
stereo frames produced per render frame. Noise and bus effects respect the
administrator's effect ceiling. New lifecycle/ray fixtures are unexecuted.

### Outstanding transport queue boundary

Incoming ENet packet/assembly quotas and authenticated input pacing do **not**
bound outgoing reliable storage. The pinned rusty_enet public API exposes traffic
totals, not retained outgoing queue bytes or acknowledgement retirement. A traffic
token bucket or application-level acknowledgement would not by itself prove a
hard queue bound against a peer that selectively acknowledges ENet traffic.
Explicit retained-byte admission in the transport, with deterministic slow-peer
disconnect/cleanup coverage, remains required. Do not treat this audit as an
implemented outgoing queue limit.
