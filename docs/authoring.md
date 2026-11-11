# Creator API 1 — experimental implemented subset

All uploaded content is a ZIP containing `manifest.json` and embedded-resource
GLB/VRM files. The server does not execute scripts during validation. The full
repository specification remains authoritative. This document describes only
the implemented subset; see `docs/status.md` for remaining acceptance gaps.

Publication verifies actual decompressed ZIP lengths and CRCs, a 256 KiB
manifest limit, and UTF-8/64 KiB limits on all Luau files, including unused ones.
GLB files must contain aligned JSON followed by at most one embedded BIN chunk,
with exact file/chunk lengths and in-bounds embedded buffer views. Each checked
structure table is capped at 16,384 entries; node hierarchies must have unique
parents, valid scene roots, no cycles and at most 256 nodes per ancestry chain.
Accessor ranges (including sparse storage, interleaving and matrix padding) are
checked with overflow-safe arithmetic. Per GLB, declared accessor arrays have a
256 MiB aggregate budget estimated at four bytes per component, including arrays
without backing views. This is not a bound on all importer/native allocations,
index contents, image decoding or GPU memory. Complete glTF/image validation and
decoded-allocation hardening remain in progress.

PNG, JPEG and WebP publication checks use the maintained Rust `image` decoders,
not custom codecs. Compressed images are limited to 16 MiB; dimensions to 8192
per axis; and estimated decoded storage to 64 MiB/image and 256 MiB/package,
conservatively charging eight bytes/pixel. Embedded views are streamed and
decoded one at a time without retaining a whole GLB binary chunk. Exact duplicate
views are decoded once but charged for each declaration; partially overlapping
image views are unsupported.
Signatures must match embedded MIME declarations. Decoder allocation limits are
also configured, but are not a general process, CPU or aggregate upload budget.
Runtime texture copies, mipmaps and importer allocations need separate limits.

Generate reference packages with the locked development environment:

```sh
cargo run --example make-example -- /tmp/opencode/quiet-room.zip
cargo run --example make-example -- /tmp/opencode/tank-yard.zip --tanks
```

Upload them through the website. The tank yard contains a switch/door, a prop,
two tanks, a spawn switch, and GLB tank/shell prefabs. In the desktop phone,
claim the nearby vehicle, use arrow keys to drive/turn, and Space to fire.
Right-drag outside the phone to look around. WASD moves the user independently
of the vehicle: driving never moves the XR camera. Release the lease to hand
the vehicle to someone else. A tank needs four hits before destruction; its
attacker receives a point. These are virtual game objects only.

## Manifest

Schema 1 uses metres, seconds, Godot's right-handed coordinates (+Y up,
forward -Z), positions `[x,y,z]`, and quaternions `[x,y,z,w]` on the wire.
Objects have stable `id`, `component`, `position`, optional `size`, optional
`physics`, and optional `prefab` referring to a packaged definition:

```json
{
  "schema": 1,
  "kind": "world",
  "name": "Example",
  "scene": "world.glb",
  "variables": {
    "lights_enabled": {"type":"boolean", "default":false}
  },
  "objects": [
    {"id":"tank_a", "component":"vehicle", "position":[0,0.6,-1.5], "prefab":"tank"}
  ],
  "prefabs": {
    "tank": {
      "scene":"prefabs/tank.glb", "component":"vehicle",
      "physics":{"size":[1.5,0.8,2], "mass":100, "health":100, "projectile":"shell"}
    },
    "shell": {
      "scene":"prefabs/shell.glb", "component":"projectile",
      "physics":{"shape":"sphere", "size":[0.15,0.15,0.15], "gravity_scale":0,
                 "damage":25, "speed":25, "lifetime":3}
    }
  },
  "scripts": [
    {"path":"scripts/main.luau", "side":"server", "attachment":"world",
     "capabilities":["state","objects"]}
  ]
}
```

Physics defaults: dynamic boxes, 0.5 m dimensions, mass 1, gravity multiplier 1,
health/damage/lifetime 0, drive speed 5 m/s, turn speed 1.5 rad/s, and weapon
interval 0.5 s. Zero lifetime means no scheduled expiry. Explicit-size static
objects and doors get box colliders; opening a door disables its collider.
An object's explicit physics overrides its prefab definition. A sphere uses
`size[0]/2` as radius. No polygon limits are imposed; component, memory, file,
and physics allocation budgets are separate resource limits.

## Authoritative Luau

Each server script has an independent 8 MiB sandbox and a 5 ms thread-CPU
execution watchdog (3 ms for client scripts), with a separate 250 ms wall-time
backstop. Scheduler stalls do not consume its CPU allowance. If the thread
clock is unavailable, the watchdog fails back to the stricter wall-time limit.
The server caps creator VMs at 64 across all instances (512 MiB of configured
Lua heap ceilings, not a whole-process memory budget). Capacity failure rejects
new-instance admission without leaving the player's previous instance; partial
construction returns its slots. Disabled server VMs and their pending timers
are dropped promptly. Administrator health reports active VMs and these limits.
The native client caps active creator VMs at 64 (at most 512 MiB of configured
Lua heaps, excluding other engine/native allocations). Exceeding a limit reports
a script error; it does not remove server interactions or promise a minimum
rendering frame rate. Client dispatch output is limited to 64 commands and
64 KiB of serialized command values.
It receives no OS/filesystem/network/engine handles. A script that errors during
an event is disabled, not retried every frame. Shared server command queues are
bounded to 64 commands and 64 KiB of owned UTF-8 payload plus a fixed per-command
bookkeeping charge. Command IDs are bounded to 64 bytes. These are not allocator
or native-engine heap quotas. Script-issued changes are applied by the server.

Common APIs:

- `world.on_interact(object_id, function(player) ... end)`; `player.id` identifies
  the authenticated initiator. The server checks membership and reach first.
- `world.on(event, function(data) ... end)` currently dispatches `join`, `leave`,
  `tick` (`dt`, `clock`), `collision` (`a`, `b`, `clock`), and authorized avatar
  `touch`/`grab`/explicit `release` requests. Explicit release includes the held
  segment/bone and `reason="explicit"`.
- Authoritative `damage` (`target`, `amount`, `health`, `actor`, `clock`) and
  `destroy` (`target`, `clock`) notifications are queued rather than recursively
  calling scripts while applying commands. Damage precedes destruction for a
  lethal hit. By callback time the destroyed object is already absent. Handler-
  generated notifications wait until a later tick. At most 128 notifications are
  queued, eight are attempted per simulation tick, with a 5 ms batch wall-time
  yield between dispatches in addition to each VM's CPU watchdog. Overload drops
  newest notifications with a bounded diagnostic, not authoritative state changes.
  These callbacks do not grant an authenticated radio-transmission identity.
- Automatic avatar-contact release on disconnect or permission/block cleanup
  uses the same bounded deferred queue, with `reason="disconnect"` or `"policy"`
  and the original actor/target/segment/bone/hand. Constraints are removed before
  notification. Changing/removing the target's immutable equipped avatar also
  releases old-bone grabs with `reason="avatar_change"`; reselecting the same
  version keeps them. Client avatar/world scripts also receive releases when an
  authenticated viewer-filtered snapshot removes or replaces an existing grab,
  with `reason="snapshot"` and `snapshot_tick` (not an invented server clock or
  policy reason). Explicit release packets are deduplicated against subsequent
  snapshots. Callbacks cannot revive constraints or bypass permissions. Unloaded/
  blocked avatar VMs are torn down rather than kept alive for notifications.
  Live connection takeover also releases old contact constraints with reason
  `reconnect` while preserving the current player/instance state.
- Server prop/vehicle control emits deferred `grab`, `lease` and `release`
  callbacks through the same bounded queue. Object events have `kind="object"`,
  `target`, `actor`, `clock`, `mode="grab"|"lease"`, and a hand index or `nil`.
  Avatar contact events have `kind="avatar"`. Acquisition reasons are `explicit`
  or `script`; release reasons are `explicit`, `handoff`, `lease_expired`,
  `disconnect`, `reconnect` or `destroy`. Renewing the same holder/mode does not duplicate an
  acquisition notification. Scripted handoff first releases the old hand/physics
  constraint, then acquires the new lease. Static entities cannot be leased.
  Events describe committed outcomes; by callback time the current holder may
   already differ. Client-world scripts receive the same committed object-control
   events over reliable state transport, plus `damage` and `destroy` events.
   Client dispatch is deferred with at most 128 pending events, at most eight
   drained per frame and a five-millisecond between-dispatch budget; overflow
   drops newest notifications and reports an error. Each VM retains its own CPU
   watchdog. Queries reflect the latest received graph, not necessarily the
   historical object state when the event was generated. VM/package teardown
   clears pending notifications.
- Client-world scripts also receive `join`/`leave` with `id` and `clock`, and
  `collision` with entity IDs `a`, `b` and `clock`. Membership notifications are
  filtered by the viewer's bilateral avatar-visibility blocks. A live same-instance
  connection takeover is continuous membership, not another join/leave pair.
  Collision starts use the bounded deferred server queue too, rather than up to
  64 independent immediate Lua dispatches; at most 64 new contacts are admitted
  per physics tick, with overflow reported. These callbacks describe committed
  outcomes, not client-declared contacts.
  Client notifications received while a world package loads wait in its bounded
  queue until scripts are ready. Startup commands are drained separately before
  those callbacks and the first tick. Failed/empty packages and instance teardown
  clear pending notifications. This is not a replay of membership history before
  admission or a complete player-enumeration API.
- Both hosts expose `world.after(seconds, function() ... end)` and
  `world.cancel_timer(id)`; client avatar scripts can use the `avatar` alias.
  Timers are VM-local one-shots, at most 64 pending, with finite delays in
  `0..600` seconds. Zero-delay rescheduling yields until the next processed tick.
  They advance on processed simulation/presentation ticks, not an OS wall-clock
  thread; overload can delay them and focus/sleep gaps do not cause unbounded
  catch-up bursts. Callbacks share the dispatch's existing CPU/memory/capability
  limits. Cancellation and VM teardown release registry references. Scheduling
  a timer from an interaction does not preserve an authenticated radio identity.
- `world.object(id)` returns a detached read-only-in-effect snapshot: editing the
  returned table cannot change the world.
- Server-world and client-world `world.find_tag(tag, after_id?)` return two values: up to 256
  matching stable object IDs in lexical order, and a continuation ID or `nil`.
  Pass the continuation into the next call to enumerate larger matches. Queries
  within one dispatch see the same snapshot; queued commands become visible on
  a subsequent dispatch. Queries consume the existing CPU watchdog budget;
  large enumerations can page across ticks, but those pages are not an atomic
  multi-tick snapshot. Returned IDs/tables cannot change shared state.
  Object and prefab declarations accept optional `tags` arrays, at most 16 unique
  case-sensitive names each, using 1–32 ASCII letters/digits or `_-.`. Authored
  objects inherit their prefab's tags; spawned entities inherit packaged prefab
  tags. Merged entity tags are sorted/deduplicated. Publication caps authored/
  inherited declarations and prefab declarations at 64 KiB each; each instance
  also enforces a 64 KiB live serialized-tag budget, including dynamic spawns.
  Destruction releases that budget. This does not bound complete snapshots or
   replace broader entity/importer/network allocation limits.
   Client-world VMs share a JSON-only scene context, available during script
   startup even when world loading finishes after network admission. Queries
   return replicated entity properties, not Godot nodes or mutation handles;
   absent objects and null references read as Lua `nil`. Newer pose samples
   survive older reliable graphs; deleted entities cannot be revived by stale
   graphs or pose packets. Limits are 2 MiB per JSON update, 4096 objects and
   64 KiB of resolved tags. Invalid updates clear the client context and report
   an error. Avatar VMs have separate contexts and require an explicit `queries`
   capability for their own-model view, described below; they never inherit the
   world's object context.
- With `state`: `world.state.get(key)` / `world.state.set(key,value)` read/write
  explicitly declared, typed instance variables. Undeclared keys, incompatible
  types and out-of-range values error without changing the variable. Clients
   receive snapshots but cannot invoke these authoritative setters directly.
   Client-world scripts expose read-only `world.state.get(key)` from the shared
   received snapshot, including during startup. Absent keys return `nil`, false
   remains false, and vectors are detached tables. There is no client setter.
   State snapshots have at most 128 valid keys, supported scalar/vector values,
   4096-byte strings and a 32-KiB JSON bridge bound (formatting headroom above
   the authoritative 16-KiB budget). Older state ticks are ignored; replacement
   and teardown clear removed keys. Avatar VMs do not inherit world state.
- With `objects`: `world.spawn(prefab_id,{position={x,y,z}, velocity={x,y,z},
  owner=player.id})` returns a reserved stable ID. Only packaged prefabs can be
  spawned; invalid requests are rejected when applied.
- `world.destroy(id)`, `world.move(id,{x,y,z})`, `world.velocity(id,{x,y,z})`,
  `world.set_visible(id,bool)`, `world.set_active(id,bool)`.
- `world.damage(id,amount,actor_id)`, `world.add_score(player_id,integer)`,
  `world.lease(object_id,player_id)`. These are authoritative creator rules,
  not endpoints clients can invoke directly.

Client control requests are limited to claim/grab/release, drive inputs, and
fire. They cannot specify projectile damage, spawn arbitrary objects, or set
scores. Leases are exclusive, expire without renewal, and are released on
disconnect. Vehicle inputs go stale after 250 ms and stop driving. Held props
follow received hand samples, not invented tracking. Rapier advances already
authorized objects independently of per-client network rates. Secondary avatar
physics is not part of this server simulation.

Snapshots include current objects, transforms, leases, health, variables, and
scores. Reliable spawn/destruction/damage events accompany state changes;
unreliable transform streams are independently rate-controlled. All of this is
discarded when an instance empties.

### Client material and light controls

Client scripts with `visual` can enqueue fixed presentation-only setters:

```lua
world.set_material('Panel', 'albedo', {0.2, 0.4, 0.6})
world.set_material('Panel', 'roughness', 0.25)
world.set_light('Lamp', 'energy', 3)
world.set_light('Lamp', 'enabled', false)
world.set_ambient({0.5, 0.6, 1}, 0.25)
```

Material properties are `albedo`/`emission` (three RGB numbers in `[0,1]`),
`roughness`/`metallic` (`[0,1]`), and `emission_energy` (`[0,8]`). Light
properties are `color` (RGB), `energy` (`[0,8]`), `enabled`/`shadow` (booleans),
and `range` (`[0.01,100]` metres). Values must have their actual types; numeric
strings are not numbers. No shader, texture, resource path or generic property
access is exposed. Lua values are decoded only as these fixed flat shapes.

Targets are exact unique scoped node names or authoritative entity IDs; avatars
can also use their own captured `node:N` IDs when queries are enabled. World
scripts can target `@world` for the loaded world root. Object-attached scripts
can target only their attachment ID. A target group applies to all descendant
meshes/lights. Ambiguous names and paths are rejected. A range update involving
a directional light is rejected before updating any light in the group.

Material overrides are isolated per mesh; script state survives tier changes,
and entity-level overrides are reapplied when a fallback becomes a packaged
prefab. Tier 0 suppresses computed shadows; lower tiers approximate scripted
emission without the authored emission texture. These calls affect only the
local presentation, not authoritative state or other clients. Use replicated
state and client callbacks for shared behavior.

`world.set_ambient(rgb, brightness)` is restricted to world-attached client
scripts. RGB is in `[0,1]` and brightness in `[0,2]`, including complete darkness
at zero in Tier 0. It tints world, entity and avatar base colors, not the phone
or other host UI. Emission remains independently controlled. New visuals inherit
the current tint. Existing visuals are recolored through an incremental walk
(256 node/child steps per render frame); large scenes may settle over multiple
frames. Repeated requests replace the desired tint without restarting the walk,
and do not reimport assets or rebuild normalized materials. Instance teardown
clears pending work and resets ambient color. This is a cheap presentation
control, not an authoritative light simulation or an access-control mechanism.

Local native/headless regression cases pass. GPU/visual acceptance remains separate.

#### Standardized advanced material schema

The same fixed material properties are usable from scripts and a package's
optional `materials` map, including avatar packages. Prefab definitions can carry
their own `materials` map. Targets are unique imported node names or `@root` for
the supplied scene root; names are applied in sorted order (`@root` first):

```json
"materials": {
  "@root": { "roughness": 0.8 },
  "Panel": {
    "albedo": [0.3, 0.4, 0.5],
    "specular": 0.6,
    "rim": 0.4,
    "rim_tint": 0.7,
    "outline_width": 0.005,
    "outline_color": [0, 0, 0],
    "matcap": "pearl",
    "matcap_strength": 0.2
  }
}
```

`specular`, `rim`, `rim_tint` and `matcap_strength` are `[0,1]`;
`outline_width` is `[0,0.02]` metres; `outline_color` is RGB `[0,1]`.
`matcap` is exactly `none`, `pearl` or `metal`. Matcaps are project-generated
cached 64-pixel textures with a project-owned shader, never uploaded shader code
or runtime URLs. Tier 2 supports rim shading, normal maps, toon specular, emission
and up to two optional extra passes (grown-backface outline and additive matcap).
Tier 1 retains modest specular and core colors; lower tiers omit extra passes
and rim shading but preserve their stored settings for switching back. Uploaded
next-pass chains are not retained. Transparent/cutout outline appearance remains
an explicit visual acceptance item rather than a promise of a Poiyomi clone.

At most 256 style targets and 13 fixed properties per target are accepted.
Missing/ambiguous mesh targets fail import. Own-node name and typed-descendant
lookups use weak cached indexes rather than repeatedly scanning the entire
scene for each command. Entity visual replacement invalidates those descendant
indexes. Imported names are stable labels for that immutable loaded scene, not
an API for adding arbitrary host nodes.

Renderer/audio admission limits are documented in [resource-policy.md](resource-policy.md).

Tier 0 converts blended transparency to alpha-hash dithering while retaining
authored alpha; cutout materials retain their threshold. Returning to higher tiers
restores the original blend mode. Normalization preserves authored UV transforms
and roughness, and interprets the pinned MToon importer's fixed color, alpha,
normal and emission uniforms without retaining its uploaded material/pass graph.
Local headless regressions cover alpha/cutoff/UV/emission and shader color variants;
actual dithered transparency and outline quality still require GPU/VR acceptance.

### Supplied client animations

Client `visual` scripts can call `play_animation(target, clip, speed?)`,
`stop_animation(target)`, and `seek_animation(target, seconds)`. World/avatar
aliases expose the same functions. Targets use the same scoped resolver as
materials/lights, and must be an AnimationPlayer or contain exactly one supplied
player. `@avatar` selects the own imported avatar model; `@world` selects the
loaded world. Speed defaults to 1 and is bounded to `[0.1,4]`. Seek requires an
assigned clip and cannot exceed its length (absolute maximum one day).

Package import disables autoplay and copies/sanitizes animation libraries.
Only supplied spatial transform/bone and blend-shape tracks remain; arbitrary
property, method, audio and foreign-node tracks are stripped. This does not
grant access to the phone, other avatars or host nodes. Playback is cosmetic:
shared/discrete behavior must be driven from authoritative replicated state or
the avatar's declared server-replicated actions, not client-authored outcomes.
The pre-existing avatar `animation(name)` helper remains available.

Local headless control/sanitization regressions pass.

### Client acoustic-zone controls

World-attached client scripts with `audio` can call
`world.set_zone(id, property, value)` for a declared acoustic zone. Properties
are `reverb`, `echo`, `distortion`, `noise` (numbers `[0,1]`) and `compression` (boolean).
Effective amounts remain clamped to the administrator's audio-effect ceiling;
zero ceiling also suppresses compression. Bounds/position and microphone/call
routing are not writable. Avatar/object-attached scripts cannot use this host.

Writes to the same zone coalesce, identical writes do nothing, and at most two
zone effect chains rebuild per render frame with a rotating cursor. Private-call
dry routing and the final limiter remain unchanged. The host configures declared
zones before script initialization so startup setters are meaningful. Shared
changes should be driven from replicated state. Local headless regressions pass.

### Client callback scheduling

Live callbacks are queued and processed in order by a fair client-wide render
frame scheduler. A busy script does not make every other VM run a full watchdog
budget in the same frame. Ticks coalesce instead of forming an unbounded backlog;
`dt` includes deferred elapsed time, capped at one second. Author callbacks must
not assume presentation commands have already reached the renderer synchronously
when the input packet arrives. See [resource-policy.md](resource-policy.md) for
ceilings, initialization exceptions and remaining cooperative-budget limits.

### Creator call requests

Server-world scripts may request the `voice` capability and call
`world.call_invite(user_id)` or `world.call_leave()` **only inside an authenticated
`world.on_interact` callback**. The caller is always the actual interaction
initiator, never a script-supplied identity. Deferred interactions recheck current
membership/reach; timers cannot inherit this authority. Client/avatar/object
declarations cannot request this capability, and creators cannot accept a call
for someone else or enable microphone capture.

Invitations pass through the same router as the phone: the target must be online,
the initiator must explicitly enable `allow_creator_calls` in their account
settings (off by default),
voice blocks override requests, and the recipient must explicitly accept through
their UI before receiving private audio. World bypass policies and user call modes
still govern actual routing. Invite limits are four per actor per ten seconds
and one per recipient per five seconds, with cooldowns surviving disconnect.
The instance's creator call queue is bounded to eight requests. Local authorization,
timer, cooldown and recipient-acceptance policy regressions pass.

### Authoritative entity attachments and rotation

Server scripts with `objects` can enqueue:

```lua
world.attach('accessory', 'vehicle', {0, 1, 0})
world.rotate('vehicle', {0, 0.7071068, 0, 0.7071068}) -- x,y,z,w
world.detach('accessory')
```

Attachments are Rapier fixed joints between existing entities in the same
instance, not Godot node parenting. The child must have dynamic physics; both
entities require physics bodies. Offsets are parent-local, bounded to 100 metres
per component, with relative identity rotation. Direct parent/child collision is
disabled. The child initially snaps to the parent-local anchor; subsequent motion
is server-simulated. Client rendering still interpolates received world poses,
without another local parenting transform or speculative physics.

Cycles and paths longer than 16 entities are rejected before changing leases or
physics state. Attaching releases the child's existing control lease; it cannot
be leased or directly teleported/rotated until detached. Reattachment publishes
detach/attach notifications. Destroying a parent detaches surviving children and
removes owned joints. `world.on('attach', ...)` / `world.on('detach', ...)` receive
bounded deferred notifications on both creator hosts. Detached object queries
and authoritative snapshots expose `attached_to` and `attachment_offset`.

Rotation uses a normalized finite quaternion; flat numeric vectors/quaternions
are decoded without recursively walking arbitrary Lua tables. Local command and
Rapier/cycle/lease/snapshot regression cases pass.

Replicated `world.state.set` writes likewise decode only the variable's declared
boolean/number/integer/string/vector shape. Numeric strings and truthy non-boolean
values are not coerced; vectors use three raw numeric components without table
metamethods. Recursive/deep tables are rejected before generic JSON traversal.

### Client avatar bone and spring controls

Avatar scripts may request `queries` in addition to `visual`/`audio`. With this
read-only capability, `avatar.object(id)` and `avatar.find_tag(tag, after?)` see
only that avatar's supplied spatial nodes and skeleton, as detached JSON tables:

```lua
local head = avatar.object('humanoid:Head')
local nodes, continuation = avatar.find_tag('avatar.node')
-- Inspect node.name, node.visible, node.position and node.rotation.
```

`node:0` is the imported model root; `node:N` identifies a supplied `Node3D` in
the captured normalized import traversal. Host IK targets/modifiers are excluded.
`bone:N` identifies the normalized skeleton's bone index, and `humanoid:Head`
(similarly other available humanoid names) aliases the mapped bone. Tags are
`avatar.node`, `avatar.bone`, and `avatar.humanoid`, respectively. Display names
are truncated to 64 Unicode characters (at most 256 UTF-8 bytes); use IDs, not
these labels, as identifiers. IDs remain stable for that loaded immutable model,
and removed nodes disappear without their IDs being reused.

Stored local positions/quaternions and node visibility are sampled before each
dispatch that has an active query-enabled script. Singular node transforms use
identity rotation. These are local presentation snapshots, not authoritative
world poses or a promise to expose final post-modifier rendered skin matrices.
The normalized tracked/estimated input-source data remains in tick `tracking`.
Limits are 4096 combined node/bone/alias objects, 2 MiB per JSON update, and
64 KiB of tags. Invalid/oversized contexts clear and report an error rather than
exposing partial Godot objects. Avatar scripts without `queries` get an empty
context even when another script on the same avatar requested it. The opt-in
avoids sampling this view for ordinary visual/audio-only scripts; aggregate
sampling/renderer performance still requires extended acceptance.

With `visual`, an `avatar`-attached script can use `avatar.set_visible('node:N',
bool)` on these own-node IDs. Bone rotation/reset also accepts `bone:N` and
`humanoid:Name`, in addition to ordinary bone names. No node path or foreign
avatar can be reached. World state and world objects are not part of this view.

Client avatar scripts with `visual` can call:

```lua
avatar.bone_rotation('Head', 0, 0.3826834, 0, 0.9238795) -- x,y,z,w
avatar.reset_bone('Head')
avatar.spring_gravity(0.5)
```

Bone IDs use the normalized humanoid mapping when available, with an exact
own-skeleton bone-name fallback. Quaternions must have finite components in
`[-1,1]` and nonzero length; the host normalizes them. Overrides are absolute
local rotations, with at most 64 distinct overridden bones per avatar. They run
after IK/springs and before authorized cosmetic grab constraints. Reset removes
the override and returns that bone to its authored rest rotation; ordinary
animation/tracking can drive it again. A script attached to `avatar` can address
its own bones; a bone-specific attachment can address only its exact target.
No paths, Godot handles, other avatars or authoritative collision proxies are
editable through this API.

`spring_gravity` sets the own avatar's imported VRM secondary-system gravity
multiplier in `[-2,2]` (default `1`). It changes runtime node state, not shared
spring resources or collider permission policy. It requires the `avatar`
attachment. Both effects are local presentation: replicate declared discrete
actions when all viewers should make the same choice, rather than streaming
bone transforms. Avatar replacement tears down overrides and spring settings.
Per-chain spring controls and richer bone/material APIs remain unfinished.

### Replicated variable declarations

The `variables` manifest map declares defaults before any server script runs:

```json
{
  "lights_enabled":{"type":"boolean","default":false},
  "round":{"type":"integer","default":1,"min":1,"max":100},
  "progress":{"type":"number","default":0,"min":0,"max":1},
  "message":{"type":"string","default":"","max_bytes":256},
  "waypoint":{"type":"vector3","default":[0,0,0],"min":-1000,"max":1000}
}
```

At most 128 keys (1–64 ASCII letters/digits/underscore/hyphen) are allowed.
Numeric bounds must be finite and within ±1 billion; defaults must satisfy
their own type/range. Strings declare a 1–4096 UTF-8 byte limit. JSON state
values are additionally limited to 8 KiB each and 16 KiB in aggregate including
keys, so escaped strings can reach the serialized budget before their byte
limit. Defaults and writes are checked against the aggregate budget. Integer
values from Luau must be exact whole numbers and are normalized on the wire.
All declared state is visible to joined participants; it is not secret storage.

Earlier experimental scripts that introduced keys dynamically must add these
declarations. Missing declarations are not inferred from script source. A fresh
instance restores manifest defaults, never state from a destroyed instance.

## Video surfaces and timelines

The manifest may declare up to four `video_surfaces`:

```json
{"id":"screen","position":[0,2,-3],"size":[3.2,1.8],"public_controls":false}
```

The client creates a world-space quad facing +Z. Size is in metres. Controls
default to the instance owner only; `public_controls: true` permits joined
players to control that surface. Phone controls issue authenticated reliable
commands, independently of negotiated pose rates.

A server or client-world script with the `media` capability can issue bounded commands:

```lua
world.media({target="screen", action="load", source="https://example.com/video.mp4"})
world.media({target="screen", action="pause"})
world.media({target="screen", action="seek", value=42})
world.media({target="screen", action="rate", value=1.25})
world.media({target="screen", action="play"})
world.media({target="screen", action="stop"})
```

Server scripts act as authoritative world rules. Client scripts instead send
ordinary authenticated player requests through the same reliable path as the
phone: they cannot supply actor/owner identity or bypass owner/public-control
permissions. The client script attachment must be `world` or the exact target
surface ID. Avatar runtimes have no media-control network binding. The runtime
permits at most eight media requests per second across all its scripts, reports
overflow once per window and resets the counter on teardown; this is separate
from the server's connection limits. Neither host gives scripts a decoder,
arbitrary HTTP, DNS or filesystem API.

Both hosts share request validation: target IDs are 1–64 UTF-8 bytes, source
strings at most 4096 bytes, load sources public HTTP(S) without credentials,
seeks `0..604800` seconds, and rates `0.25..2`. Unknown fields/actions and
non-finite values are rejected. Decoder-side DNS/redirect/socket protections
still apply; syntax checks do not establish that a hostname resolves publicly.

The server stores the source and a monotonic play/pause/seek/rate timeline only;
clients fetch media directly through libmpv. Reliable snapshots and periodic
target-time updates support late joins and client drift correction. Native
decoders have bounded output queues, at most four simultaneous workers and
1280×720 output dimensions. The current surface uses 640×360 output. Decoded
PCM enters a positional Godot audio stream, not the system audio device.
YouTube uses the Nix-pinned yt-dlp resolver through mpv's bundled hook.

The server rejects literal private/reserved addresses, credentials and
non-HTTP(S) sources. Clients also use a local HTTP/CONNECT relay that resolves,
checks and pins public destination IPs. A Linux decoder-thread socket guard
is inherited by decoder threads and resolver subprocesses, without restricting
Godot's unrelated networking. Network-only EDL wrappers from the maintained
mpv hook are validated with bounded entries, nesting and directive allowlists.
User resolver configuration/plugins/cache and unsafe playlist loading are
disabled. The packaged resolver has a 30-second wall-clock timeout.

These are layered network protections, **not a general process/filesystem
sandbox or a complete decoder/resolver memory budget**. Tests cover private
redirect/HLS destinations and local-file M3U/EDL canaries, but do not establish
security against arbitrary native-code compromise. Single-client public MP4,
HLS and live YouTube playback have been exercised; multi-client real-output
synchronization and long-session acceptance remain. See [media-security.md](media-security.md).

The same headless surface test can exercise an external source without writing
a downloaded video to disk (network availability is not part of the offline
test suite):

```sh
nix develop --command bash -c 'env -u no_proxy -u NO_PROXY SSC_TEST_VIDEO="https://media.w3.org/2010/05/sintel/trailer.mp4" godot4 --path "$SSC_TEST_CLIENT" --headless --xr-mode off --script res://tests/video.gd'
```

An opt-in external-network ENet run also creates actual video surfaces in two
separate Godot peers, checks decoded pixels/PCM and convergence to server time,
then pauses, seeks, reconnects a late joiner and resumes. It does not substitute
a local URL for the public source:

```sh
nix develop --command bash -c 'SSC_TEST_PUBLIC_VIDEO="https://media.w3.org/2010/05/sintel/trailer.mp4" cargo test --locked -p super-simple-chat-server --test enet encrypted_godot_enet -- --nocapture'
```

This test is not part of offline Nix checks. External availability, source
duration and seek support affect its result; long-session and hardware testing
remain separate acceptance work.

## Named acoustic zones

World manifests can declare up to 16 source zones. The first matching zone wins;
unmatched sources and non-positional private calls use the dry Master path.

```json
{"id":"wet_closet","position":[0,1,0],"size":[2,2,2],
 "effects":{"reverb":0.9,"echo":0.3,"distortion":0.1,"compression":true}}
```

Effect amounts are in `[0,1]` and clamped to the administrator's
`SSC_AUDIO_EFFECT_MAX` (default `0.9`). Geometry only defines membership: a large
room may be dry, and a small room may be very reverberant. Voice and video audio
use Godot's native bus effects and a final limiter. Buses are released when the
world changes. The noise/occlusion implementation below passes local regressions;
scripts cannot arbitrarily reroute sources or private calls.

### Optional occlusion and zone noise

World manifests can set `"realistic_acoustics":true` (default false). This enables
a bounded single-ray obstruction approximation, not a room-acoustics simulation:
up to four physics-space queries per physics tick, rotating among at most 512
sources, with a minimum 200-ms interval per source. A blocked path applies 0.35
gain and a 1,200-Hz attenuation filter; unobstructed paths restore the original
filter. Sources' ancestor collision bodies are excluded. Queries use existing
physics collision geometry, not visual triangles; reflections, diffraction and
material transmission are not simulated. Under load, updates are less frequent.

Proximity voice, packaged clips, video and radio sources participate. Private-call
audio bypasses both zone routing and obstruction, restoring any previous filter.
Radio retains its narrowband chain and listener-zone routing; its obstruction is
evaluated at the receiving device. Account voice gain also applies to radio voice.

Zone `effects.noise` is independent of the realistic-acoustics toggle. The first
zone containing the listener selects bounded mono white noise, mixed through that
zone's effects. Gain is `0.1 * min(noise, audio_effect_ceiling)`; a zero ceiling
disables it. One lazy generator fills at most 4,800 stereo frames per render
frame, stops outside noisy zones, reuses its player on re-entry, and is released
on world teardown. `world.set_zone(id,"noise",value)` uses the same `[0,1]` bounds
as other scalar effects. Ray/exclusion/dry-route and generator lifecycle fixtures
pass locally; audible and hardware acceptance remain.

### Packaged standalone audio

World and avatar manifests may declare up to eight `sounds`:

```json
{"id":"bell","path":"audio/bell.opus","position":[0,1,-2],
 "volume":0.7,"autoplay":false,"loop":false}
```

Only Ogg Opus is admitted: a single logical stream, mono/stereo, at most ten
minutes, 32 MiB per file and 64 MiB of audio per package. Publication and playback
use the same maintained libopusfile integration to check the container, stream
layout and duration. Malformed payloads can still fail during decode; this is
not a full malformed-resource security audit. Decoding is memory-only and does
not expose filesystem/network callbacks. PCM is read in bounded chunks into
Godot's native 3D generators, routed through the source's acoustic zone.
There is a process-wide ceiling of 32 decoders and 64 MiB resident compressed
audio, separate from broader package/cache/GPU budgets still under development.

With the `audio` capability, both hosts expose `world.play_sound(id)`,
`world.stop_sound(id)` and `world.seek_sound(id, seconds)`; client avatar scripts
can use the equivalent `avatar` alias. IDs address only declared sounds in the
script's own package. Server commands deliver reliable transient events to
current instance participants; these are not synchronized video timelines or
a replay log for late joiners. Use authored looping/autoplay for local ambience.
Client scripts cannot send an authoritative sound command to other clients.
World/avatar replacement releases its decoders, generators and queued PCM.
The server sends viewer-specific `audio_allowed` state for avatars, based on
bilateral voice/full blocks. The official client applies it to all avatar
sound players, including script-triggered sounds; creator commands cannot
change that mute. This is not a claim that every block/script interaction in
the specification has completed acceptance testing.

## Virtual cross-world radio

Opt in by declaring up to eight standardized components:

```json
"radios": [{"id":"desk_radio","position":[0,1.2,-1],"channel":1,"preset":"narrow"}]
```

Transmitters must be joined and within 3 m of the selected radio. Receivers need
a same-channel component; worlds with no radios receive no transmissions.
Voice is spatialized at each receiver's speaker and only routed within 30 m.
Voice and digital messages honor bilateral voice/text/full blocks respectively.
Radio voice also honors each world's proximity-bypass permission.

The phone selects the microphone route without unmuting it: radio transmission
or normal proximity/private-call routing, never both. Radio takes precedence
for 250 ms server-side, with a 300 ms client mute of overlapping normal playback.
The radio band-pass/distortion chain feeds the listener's named acoustic-zone
bus. This is an explicit mixing policy, not physically accurate propagation.

Built-in SSTV uses protected 64×48 grayscale image data, with local visual
degradation. FT8-style messages use up to 40 printable ASCII characters. The
illustrative tones/hiss are generated locally; these are **not** interoperable
SSTV/FT8 modem waveforms. No external program, audio virtual device, or RF link is
required or implemented. Received images/text appear on the in-world radio.

Administrator environment settings:

- `SSC_RADIO_CHANNELS`: comma-separated allowed channels (default `1,2,3,4`;
  empty disables transmission), at most 32 channels in `1..9999`.
- `SSC_RADIO_BITRATE`: 8,000–16,000 bit/s, default 8,000.
- `SSC_RADIO_PRESETS`: nonempty, unique comma-separated subset of
  `standard,narrow,weak` (all enabled by default). The official client disables
  unlisted choices and falls back to the first allowed preset.
- `SSC_RADIO_FILTER_HZ`: band-pass centre 300–3,000 Hz, default 1,500.
- `SSC_RADIO_NOISE`: hiss/image degradation 0–0.8, default 0.15.
- `SSC_RADIO_INTERFERENCE`: additional degradation during overlapping senders,
  0–0.8, default 0.35. Activity windows are 250 ms for voice frames, 1 s for
  FT8-style messages and 2 s for SSTV; this is a cheap virtual-channel model.
- `SSC_RADIO_FT8_INTERVAL`: seconds between digital messages, 1–120, default 2.
- `SSC_RADIO_SSTV_INTERVAL`: seconds between images, 5–600, default 30.

World scripts with the `radio` capability can request:

```lua
world.radio_message("desk_radio", "CQ VR")
world.radio_image("desk_radio", grayscale_hex) -- exactly 64*48 bytes as hex
```

Client requests use the same authenticated request path as the phone. Declared component,
reach, channel, payload, cooldown and block checks remain server-owned. Scripts
cannot enable microphone capture or send raw voice frames. Remote avatar scripts
cannot impersonate the listener: avatar manifests allow only client-side
`visual`/`audio` and read-only own-model `queries` capabilities, never `radio`.

Server scripts may call these functions **only inside `world.on_interact`
callbacks**. The host binds the actual authenticated initiator; editing the
callback's detached player table cannot change the sender. Source loading,
ticks, timers and other background events have no transmission identity and
are rejected. Identity is cleared even when a callback errors. A bounded queue
is drained outside the instance lock, then the existing radio router rechecks
membership, radio reach, channel policy, blocking and the user's shared digital
cooldowns. Scripted transmissions cannot bypass cooldowns by reconnecting or by
switching between phone and creator APIs. The reference `radio_send` button
demonstrates an authenticated server-scripted FT8-style message.

The official client also paces digital requests using negotiated intervals,
sharing counters between creator and phone controls and retaining them across
world changes for the same authenticated user. Server checks remain decisive;
this local pacing prevents a creator command burst from flooding reliable
transport. Server-script echoes update the local counter too.

The phone offers listener-wide `standard`, `narrow` and `weak` presets. Narrow
and weak lower the band-pass centre within the 300 Hz minimum, add degradation
above the administrator's noise baseline and reduce outgoing Opus bitrate where
possible (never below 8,000 bit/s or above the negotiated ceiling). They cannot
remove administrator degradation or increase the configured quality ceiling.
The administrator preset allowlist is sent with the negotiated radio policy.
As with any local DSP, a modified client can alter its own presentation; channel,
packet/bitrate and cooldown enforcement remains server-side.

Each radio declaration can also select a speaker-side `preset` (`standard` by
default, or `narrow`/`weak`). Unknown names are rejected on upload. A preset
disabled by a later administrator policy falls back to the first allowed name.
Each speaker has an independent filter bus feeding the listener-wide radio bus
and existing environment chain. Object degradation and listener degradation
compose; neither can remove administrator filtering/noise. Identical noise
floors use the maximum, not a duplicated additive penalty. Voice and digital
tones use the object's bus; protected images use its effective noise floor.
This does not introduce per-object Opus encoder rates or bypass the server's
negotiated bitrate ceiling. Real-headset/audio acceptance remains outstanding.

Current channel activity and rate counters are transient and never stored in
SQLite. Cooldowns survive reconnects, expire within ten minutes when offline,
and have a bounded 1,024-user counter budget. More advanced collision modelling,
complete creator policy controls and real XR/audio usability validation remain
unfinished.
