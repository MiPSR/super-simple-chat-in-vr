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
Both active materials and retained normalized source materials are counted:
disabled emission/normal textures cannot escape admission merely because Godot
omits unused unlit properties when duplicating a material.
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
worker threads. Production decoding now uses separate Landlock-confined helper
processes with a 2-GiB per-process address-space ceiling and bounded IPC deadlines.
This is not an aggregate resolver-descendant cgroup memory/CPU/process budget.
See [media-security.md](media-security.md), [asset-storage.md](asset-storage.md)
and [asset-cache.md](asset-cache.md) for their distinct boundaries.

Optional acoustic obstruction retains at most 512 weak source records and makes
at most four ray queries per physics tick, with a 200-ms per-source minimum
interval. Listener-zone noise uses one lazy 100-ms generator, bounded to 4,800
stereo frames produced per render frame. Noise and bus effects respect the
administrator's effect ceiling. Lifecycle/ray fixtures pass headlessly; audible
acceptance still requires hardware.

### Outgoing transport ownership

Server outgoing packet buffers now hold RAII reservations through rusty_enet's
public `Box<T: AsRef<[u8]>>` packet-data ownership API. The reservation retires
only when ENet releases its last packet reference (acknowledgement, discard,
disconnect or host teardown), not on an application acknowledgement or a traffic
counter reset. Each peer has a 4-MiB ceiling and the process a 64-MiB ceiling.
Charges include buffer allocation capacity plus conservative per-packet and
MTU-dependent fragment-command overhead; these are admission estimates, not a
complete allocator/RSS accounting guarantee.

Authenticated administrator health exposes `realtime_output.reserved_bytes`,
`peer_limit_bytes` and `process_limit_bytes`. Reserved bytes are a live atomic
sample of packet-owned charges, not wire throughput or total ENet memory.

Reliable admission/send failure disconnects the slow peer and marks membership
for cleanup on the next authoritative tick. Unreliable admission failure drops
that sample. Non-control channels use `AlwaysUnreliable` so a large pose/voice
packet cannot silently become reliable while fragmenting. Unit packet-clone and
real unacknowledged ENet queue teardown regressions are included; verification
results are recorded separately in status documentation. Incoming ENet control
bookkeeping and client-side Godot transport allocation remain distinct boundaries.

### Import concurrency

OFD cache pins conservatively survive duplicated or fork-inherited descriptors.
`CLOEXEC` releases inherited copies at exec, not at fork; eviction can therefore
briefly report busy after the originating reader closes. It skips such entries
until a later bounded pass rather than deleting a potentially live package.

Completion/failure callbacks cannot start a replacement import until the old
import's cache pin and FIFO slot retire. Replacement requests are queued during
completion dispatch, and failed-cache removal uses the completed path rather
than mutable next-load state. This prevents reentrant listeners from releasing
the new import's slot or deleting the new package.

World and avatar package loaders now share one FIFO GLTF import/construction
slot per Godot process. Waiting loaders retain their completed-ZIP pins, do not
start decoder threads, and release their queue entry and pin when cancelled or
destroyed. The slot spans parsing, prefab/main scene construction and synchronous
load listeners; cancelled running imports retire before the next begins. Weak
ownership prevents a deleted loader from wedging the queue. This avoids multiplying
peak allocations across simultaneous avatar/world imports, but does **not** impose
a per-import allocator/RSS ceiling or isolate a Godot importer crash. Those remain
separate release limitations. FIFO/dead-owner/cancellation fixtures are included.

Leaving an instance now retires an active avatar loader asynchronously: it cancels
publication immediately, retains its thread/cache ownership until work finishes,
then frees the loader. It no longer destroys that loader (and joins an active
importer) on an ordinary world-leave frame. Final application/tree shutdown still
joins remaining threads before unloading engine/native code. A semaphore-blocked
fixture checks that retirement returns before a worker is permitted to finish.
