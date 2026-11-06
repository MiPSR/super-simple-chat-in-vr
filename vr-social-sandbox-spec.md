# VR Social Sandbox — Project Specification

**Status:** Agreed functional specification  
**Specification date:** 2026-10-30  
**Target:** Complete initial release, private deployment  
**Primary platform:** NixOS, Linux x86-64, PCVR through OpenXR

## 1. Project objective

Build a private, VRChat-like social VR sandbox focused on spending time together in user-created worlds, displaying personal avatars, talking, watching videos, scripting interactions, and physically interacting with other avatars. It must support comfortable extended sessions, including sleeping in VR.

This is **not** an attempt to reproduce VRChat's entire ecosystem. The project intentionally standardizes materials, assets, scripting, networking, and interactions to reduce complexity and resource usage without restricting creators to noninteractive static scenes.

All functionality described as required in this document belongs to the **first complete release**. Development may be incremental, but features must not be silently reclassified as optional or postponed to a hypothetical second release. Only features explicitly marked *future* are outside the initial scope.

### 1.1 Core principles

- Prefer native functionality provided by Godot, OpenXR, Rust, or established libraries. Do not reimplement an engine, media player, codec, or transport without a concrete reason.
- Keep the backend in Rust and the internal game client in GDScript. Use sandboxed Luau only for creator scripts.
- Standardize uploaded assets and creator APIs. Do not load arbitrary uploaded Godot projects, native code, shaders, or unrestricted GDScript.
- Keep all live world instances ephemeral and their shared state server-authoritative.
- Provide rich avatar/world behavior without imposing polygon-count limits or per-avatar rendering quality grades.
- Let each client reduce rendering quality and network update rate independently.
- Prioritize VR comfort, predictable behavior, stable long sessions, and graceful recovery over speculative motion prediction or maximum visual fidelity.
- Use a simple monolithic deployment until a real requirement justifies splitting services.

## 2. Supported platforms and technology

| Area | Choice |
| --- | --- |
| Engine | Godot **4.7.2 stable**, or a newer stable release when implementation begins |
| Internal client scripting | GDScript |
| Creator scripting | Sandboxed Luau |
| XR | OpenXR, using Godot's native XR support |
| Client operating system | NixOS/Linux x86-64 initially |
| Server operating system | NixOS initially |
| Later platforms | Windows, potentially other desktop Linux distributions |
| Explicitly excluded initial targets | Standalone Quest, mobile, consoles, browser-based game client |
| Backend | Rust, latest compatible stable toolchain pinned in the Nix flake |
| HTTP API | Axum |
| Database | One SQLite database, using `rusqlite` |
| Website | Vanilla HTML, CSS, JavaScript; no mandatory web framework |
| Real-time transport | Godot's native low-level ENet API, with a Rust ENet implementation |
| Voice and uploaded audio codec | Opus |
| Avatars | VRM 1.0, with platform-specific metadata |
| World geometry | glTF 2.0 / GLB |
| Video | mpv/libmpv with yt-dlp support for YouTube and direct public media |
| Packaging/deployment | One Nix flake exporting client/server packages, runnable apps, and a NixOS service module |

Choose existing, maintained integrations for VRM, Luau, Opus, and mpv after checking compatibility with the pinned Godot version. Native extensions are justified only for functionality Godot lacks; keep them thin and isolated. Do not introduce Rust into ordinary Godot gameplay scripts simply for language consistency.

Pin all versions and hashes using `flake.lock` and the appropriate Rust dependency lockfile. Prefer stable releases over beta versions.

## 3. Logical architecture

One Rust backend application initially contains these modules:

1. HTTP API and static website delivery.
2. Discord OAuth authentication and account administration.
3. Asset upload validation, processing, storage, and download.
4. Instance lifecycle and discovery.
5. ENet real-time connection/session management.
6. Per-instance authoritative world state and script execution.
7. Voice routing, private-call channels, and global radio channels.
8. Server-wide settings and permissions.

One SQLite database persists **platform data**, not live instances. Uploaded assets reside on disk, referenced by database entries. Active worlds, scripts, player positions, transient media states, interactions, and instance objects live in memory.

A reverse proxy provides HTTPS for the website and API. ENet uses its own network listener. Do not use SQLite to exchange per-tick positions or audio frames.

The Godot client contains the XR rig, avatar/VRM loader, scene loader, renderer, audio mixer, video surfaces, networking, local physics/IK, Luau client sandbox, and in-world UI.

## 4. Nix packaging and repository

Use **one flake**, making both the client and server independently runnable:

```sh
nix run .#client
nix run .#server
nix develop
```

Expose suitable `packages`, `apps`, `devShells`, and `nixosModules.server`. Provide build commands and a documented local development configuration. A single root repository with `/client`, `/server`, `/web`, `/shared` (schemas/protocol definitions), `/tools`, `/docs`, and `/tests` is a suggested layout, not a mandatory filename contract.

NixOS services manage the backend; do not require Docker, Podman, or Kubernetes. Website static assets may be embedded in or served alongside the Rust application. The server must support normal configuration and data paths through its NixOS module.

Windows builds are secondary and must not complicate the NixOS-first workflow unnecessarily.

## 5. Identity, access, and administration

### 5.1 Accounts

- Normal users authenticate through **Discord OAuth2 only**.
- The Discord user ID must also be present in an administrator-managed allowlist. OAuth success alone never grants access.
- Admin accounts have a separate, protected authentication path and do not depend on Discord authentication.
- The website and game client both require an authenticated authorized account. The client may complete OAuth through an external system browser and redeem a short-lived one-use login grant.
- Unapproved accounts cannot browse private content, download assets, join instances, or operate scripts remotely.
- Sessions, invitation tokens, and game connection credentials have explicit lifetimes and can be revoked.

### 5.2 Admin website

The admin can:

- Add/remove Discord IDs from the allowlist and manage user access.
- Suspend/ban users and revoke their active sessions.
- Review, disable, or delete uploaded worlds and avatars as needed.
- Set per-user world/avatar slot counts and per-asset upload limits; set global default values.
- Configure instance-capacity ceilings, networking/TPS policies, script budgets, audio/voice/radio quality limits, and server-wide feature policies.
- Configure audio profiles and quality constraints, including voice and radio bitrate.
- Inspect server health, current instances, connected players, and actionable error logs.

Keep administration simple; do not add a large role-management or moderation platform without a real need.

## 6. Persistent data and file storage

Use **one SQLite database** through `rusqlite`, with migrations and suitable WAL/busy-timeout policy. A single backend process should coordinate most database access.

Persistent data includes:

- Authorized Discord identities and admin accounts.
- Account preferences and privacy/blocking settings.
- World and avatar asset metadata, owners, versions, and storage references.
- User slots and quotas.
- Server and administrator configuration.
- World catalog/search metadata and ownership.

Persist **no gameplay state**, instance snapshots, world object positions, active calls, media positions, or script variables once an instance is destroyed. Any in-memory instance registry is nonpersistent.

Store uploaded model packages outside SQLite. Validate, stage, then atomically publish files. Store content hashes and package/schema versions. Replacing a world creates a new immutable asset version; an already running instance continues using the version with which it was created. Referenced versions remain available until no longer needed.

## 7. Uploads, ownership, and quotas

Initial administrator-configurable defaults:

| Property | Default |
| --- | ---: |
| World slots per user | 4 |
| Avatar slots per user | 8 |
| Maximum uploaded world package | 1 GB |
| Maximum uploaded avatar package | 100 MB |

These are **per-asset package size limits**, not polygon ceilings. Enforce limits during upload and also prevent compressed archives from expanding without bound. All quotas are configurable by administrators.

### 7.1 Worlds

- Any allowlisted user can upload, replace, delete, name, describe, and manage their worlds through the website.
- Every published world is visible and searchable to all authorized users. **There are no private world listings.**
- Instances of a published world can be public or private/invite-only.
- World creators specify a preferred maximum player count and instance options within administrative server limits.
- The creator may update a world while older instances continue with their existing loaded asset version.

### 7.2 Avatars

- Avatars belong to exactly one uploading user.
- There is no public avatar library, avatar sharing, gifting, or avatar cloning feature.
- Avatar upload and replacement are performed **on the website**, not from inside the game client.
- The game can select among the owner's uploaded avatar slots.
- Other clients necessarily receive the mesh and textures required to display the owner's equipped avatar. This is **not DRM**: private ownership means no permission to equip another user's avatar, not a promise that visible asset data cannot be extracted.
- If the user has no avatar, render a lightweight fallback humanoid made from simple spheres/capsules or similarly minimal shapes.

### 7.3 Validation

- Accept only documented package/file types and versions.
- Reject path traversal, invalid archive entries, oversized unpacked content, unsupported executable payloads, and malformed resources.
- Validate texture and asset references, duplicate identifiers, scene nodes, script attachments, and size budgets.
- Do not execute a world or avatar script during file validation.
- Normalize material definitions and metadata to the project's documented schema.
- Never load untrusted `.pck` files or arbitrary uploaded Godot resources containing executable behavior.
- Provide understandable validation diagnostics to the uploader.

## 8. Authoring and asset package formats

### 8.1 World package

Use a documented, versioned archive containing a manifest, a glTF/GLB scene, packaged prefab assets, Opus audio, and optional Luau scripts. A conceptual example:

```text
example-world.zip
  manifest.json
  world.glb
  prefabs/
    tank.glb
    shell.glb
  audio/
    engine.opus
  scripts/
    main.luau
    tank.luau
```

A manifest identifies world metadata, supported material properties, spawn points, collision/physics proxies, scene object IDs, interactable components, prefab IDs, optional scripts, video surfaces, and optional radio objects. All runtime-spawnable objects **must be supplied within the uploaded world package**.

A creator can prepare the GLB using Blender or another glTF authoring tool, then bundle the manifest and scripts. Provide reference examples and authoring/export utilities where this reduces work. Do not require a full Godot project from creators.

### 8.2 Avatar package

Use VRM 1.0 as the primary humanoid format, with an optional manifest and Luau scripts for custom actions/behavior. Support VRM humanoid bone mappings, expressions/blend shapes, spring bones, colliders, and supported material properties.

BOOTH avatars distributed only as Unity projects require conversion/export to VRM by the uploader; importing Unity asset packages into the game is not required. Avatar owners remain responsible for complying with their assets' licenses.

Avatars may include arbitrary authored visual features and animations, including clothing/accessory toggles, expressions, unusual movements, and explicit or comedic animations. There is no project-wide content restriction on which authored mesh parts may be toggled. Scripts remain constrained by API permissions and sandbox rules.

### 8.3 Geometry policy

- **No hard triangle-count or polygon-count upload limit** for worlds or avatars.
- Source geometry is not automatically replaced with lower-poly geometry when selecting lower client quality.
- Client graphical settings can reduce textures, shader work, draw distance, effects, and resolution without changing the source mesh.
- Authored deformation, blend shapes, bones, visibility toggles, and spawning packaged prefab meshes are allowed.
- Custom user-provided native shaders or arbitrary runtime geometry-generating code are not part of the upload interface.
- High-complexity meshes may load slowly or exceed hardware capabilities. Bound allocations and show a clear fallback/error instead of crashing; do not invent a fake performance guarantee.

## 9. Rendering and quality

Every world and avatar uses **one standardized material model**. Users cannot upload arbitrary shaders, and there are **no avatar quality classes**.

Each client selects one of three graphical quality tiers affecting **both worlds and avatars**:

| Tier | Behavior |
| --- | --- |
| **0 — Ultra / Potato** | Unlit/albedo textures, optional dithering for transparency, no computed dynamic shadows or baked-lighting requirement; basic global color/brightness control remains available. |
| **1 — Standard** | Low-cost diffuse/toon lighting, modest specular response, limited shadows and basic effects. |
| **2 — Advanced** | The complete project-defined toon/material feature set, inspired by Poiyomi-style materials where practical: selected matcaps, outline/emission effects, richer specular/shading, etc. |

Implement as **one constrained material schema with multiple supported rendering paths**, not three incompatible upload formats. A client at Tier 0 must still see the same objects and gameplay-relevant visual state as a Tier 2 client, even when detail and lighting differ. Material quality must never determine whether world scripts execute.

The minimum tier can be fully unlit while still allowing world scripts and creators to control an inexpensive global ambient tint/brightness, including deliberate darkness or day/night effects. No realistic lighting computation is required at this tier.

All advanced graphical controls are independent settings where supported by the selected Godot rendering backend, including:

- Render resolution and render scale.
- Antialiasing (including MSAA and any renderer-supported alternatives).
- Upscaling/FSR when the chosen backend supports it.
- Texture resolution reduction, filtering, and memory-budget policy.
- Shadow quality, post-processing, transparency strategy, and distance culling.
- Optional renderer selection where supported, with restart if needed.

Use Godot's **Mobile renderer as the initial OpenXR-compatible baseline**, accepting its feature limitations. Forward+ may be supported later if it gives a real benefit. FSR2 and some advanced effects are Forward+-specific; don't present unsupported settings as functional on Mobile.

Hardware ray tracing is **not an initial requirement** because it is not a standard supported Godot 4.7 feature. Add it only if a stable future engine/backend provides an appropriate interface. Do not invent a ray-tracing implementation.

Textures/materials are normalized and interpreted by the project; a user's chosen tier only removes or approximates supported effects.

## 10. OpenXR, movement, and body tracking

- Use Godot's native OpenXR support for headset/controllers and applicable body trackers.
- The player's **own headset and hand pose must update locally at XR frame rate** independently of networking, to avoid VR discomfort.
- Implement full-body tracking through available OpenXR tracker/body inputs, with graceful fallback when a runtime does not expose particular joints.
- Implement full-body simulation/IK for head-and-controller-only users, including estimated hips, torso, legs, knees, feet, and ground contact.
- Support standing, crouching, sitting, and lying/sleeping postures. Non-FBT users must be able to select and remain in a comfortable sleeping pose while their head/hands continue to work.
- An avatar's joint state can derive from tracked joints, IK, or an authored animation; expose this distinction to permitted avatar scripts.
- Locomotion and avatar animation must not forcibly move or rotate the real user's XR camera in response to network corrections.
- Support authored avatars' ordinary skeletons, expressions, and spring-bone behaviors through the normalized avatar/IK interface.

The server synchronizes meaningful tracking inputs and pose states; clients reconstruct and render complete skeletons locally. Do not stream every animated bone each tick when the same result can be reconstructed from tracking/animation parameters.

## 11. Physical avatar interaction and permissions

Support head pats, hair grabbing, and other permitted avatar interactions.

- Contact detection uses appropriately defined avatar colliders and tracked/simulated hands.
- An interaction must pass both participants' permissions, including **`can_touch_me`** for the target and **`can_touch_others`** for the initiator, plus user blocking rules.
- Avatar authors may customize interaction responses through sandboxed scripts: expressions, sounds, toggles, animation playback, and spring-bone responses.
- Avatar owners may disable contact, and blocks always override world scripts.
- Shared interaction events are validated and assigned by the server.
- Hair, clothes, ears, and similar **secondary physics are simulated locally** by each client. Synchronize grab/release events, target bone/segment ID, and relevant hand/constraint inputs, rather than broadcasting every hair bone.
- Clients are allowed minor differences in cosmetic physics; permissions and event ownership must agree.

## 12. Worlds, instances, and lifecycle

Published worlds are searchable and accessible to all allowlisted users. Their runtime **instances** can be:

- **Public** — discoverable/joinable by authorized users, subject to capacity.
- **Private/invite-only** — joinable by explicitly invited authorized users.

World creators control the preferred instance capacity, subject to administrator maximums. Multiple instances can use the same published world/version.

**Instances are strictly ephemeral.**

1. The server creates an instance when needed for a joining player/party.
2. The instance loads the published asset version and initializes its world scripts and shared state.
3. During the instance's life, the server owns authoritative object/player state.
4. An arriving or reconnecting user receives an authoritative snapshot plus any updates that occurred while it was being delivered.
5. **Immediately when the last player leaves or disconnects, the instance is destroyed.** Its scripts, entities, invitations, media playback state, and all in-memory game state are discarded.
6. The instance is **not persisted to SQLite or restored after destruction/server restart**. A subsequent join creates a fresh instance using the world definition.

If one user disconnects and **at least one other player remains**, the instance continues; a reconnect receives current live state. If the disconnect leaves zero players, the instance dies and cannot be rejoined in its previous state. An in-progress connection is not counted as a joined player until admitted; teardown follows the actual server-side player count.

Persistent world catalog entries, world uploads, account settings, and admin configuration remain available independently of instances.

## 13. Network transport and authentication

Use **Godot's native low-level `ENetConnection` / `ENetPacketPeer`** and a compatible Rust ENet implementation, with `rusty_enet` as the first candidate. Do not use Godot's internal high-level MultiplayerAPI wire protocol for a non-Godot Rust server.

Define a small, explicit, versioned binary application protocol containing message IDs, instance IDs, entity IDs, state sequence/tick information, and the minimum required data. Avoid building a large generic RPC framework.

Use ENet's existing delivery mechanisms:

- **Reliable ordered:** join/leave, permissions, text, selections, world state transitions, creation/destruction, grab/release, script-authoritative events, control requests, media commands, and snapshot setup.
- **Unreliable sequenced:** player/head/hand/FBT poses and fast-changing transforms where newer samples supersede older ones.
- **Unreliable time-sensitive:** Opus frames and other disposable real-time samples, with sequencing/timestamps at the application layer as needed.

Avoid filling reliable channels with continuously changing tracking data. Use separate channels or appropriate packet flags to prevent unrelated traffic from blocking critical updates.

Authentication is required before joining an instance or accepting privileged messages. HTTP login issues short-lived authenticated game connection grants; ENet cannot create an identity or bypass the Discord allowlist.

### 13.1 ENet compatibility and encryption verification

Godot ships a modified ENet integration, and its custom DTLS extension is **not automatically interoperable** with plain Rust ENet. Standard ENet has **no built-in encryption**.

**Verified from documentation:** Godot exposes native low-level ENet APIs; `rusty_enet` is an ENet implementation in Rust. **Not yet verified by a live test:** successful packet exchange and a compatible encrypted channel with this exact Godot/Rust combination.

Before declaring networking complete, build automated integration tests for connection, reliable and unreliable/sequenced traffic, channel behavior, disconnect/reconnect, and malformed packets. Validate interoperability against the pinned Godot build.

Secure remote sessions must use a **tested confidentiality and authentication layer**, not a fictional `rusty_enet`/Godot DTLS compatibility assumption. Prefer a solution built on existing libraries. If Godot-native DTLS cannot talk to the Rust ENet endpoint, choose and document a proven alternative (such as a minimal authenticated encryption layer or tunnel) and isolate any strictly necessary native integration. Unencrypted ENet is acceptable **only in explicitly marked local development** and must not be described as private or secure for Internet use.

## 14. Authoritative TPS and client rate reduction

Keep three independent concepts:

1. **Local render/XR frame rate:** chosen by headset/runtime/client, never limited by network TPS.
2. **Server authoritative simulation TPS:** admin-set, e.g. **60 ticks/s** by default.
3. **Per-client network input and output update rates:** individually negotiated/limited, possibly below server TPS.

A client may explicitly request a lower send and/or receive rate, for example:

| Server base | Client factor | Network updates |
| --- | ---: | ---: |
| 60 TPS | 1× | 60/s |
| 60 TPS | Divide by 2 | 30/s |
| 60 TPS | Divide by 4 | 15/s |

These values are illustrations/default options, **not** a requirement that all servers always run at 60 TPS. Expose rate division in the client settings. The server enforces configured minimum/maximum rates, can automatically reduce rates for poor connections, and announces the effective negotiated rate. The input rate a client sends and the output rate it receives may differ.

A lower-rate client **must not slow the entire world simulation or force other clients to reduce their updates**. The Rust server continues authoritative ticks and samples/coalesces appropriate state for that connection. A server administrator can choose whether to perform additional aggregation/computation when combining ticks before a low-rate transmission; this must not change authoritative gameplay semantics.

### 14.1 No prediction or rollback

- **No extrapolation, speculative movement prediction, or gameplay rollback.**
- Remote avatars interpolate between **known authoritative samples**, using a bounded interpolation buffer. Once the buffer reaches the latest known sample, **hold that position/pose** until another update arrives.
- If the server receives no new tracking/movement input for an avatar, it must not invent movement from the avatar's last velocity. Apply a defined stale-input timeout and hold the last valid remote pose.
- When input becomes sparse, reduce processing and transmission rates without producing predicted intermediate positions.
- A player's **own** head and hands always follow immediate local OpenXR tracking; holding remote poses does not freeze the local headset.
- An already-authorized **server-simulated** projectile or moving game object may advance under server physics even when no client packet arrives; this does not count as extrapolating missing player tracking.
- Shared state transitions and object spawning remain reliable and ordered, even for clients requesting low update rates. Never downsample away a crucial event.

## 15. Server authority, gameplay physics, and secondary physics

The **server is authoritative** for shared identities, instance membership, game rules, object ownership/control leases, spawning/despawning, shared collisions, score/damage outcomes, and persistent-within-instance state.

Clients may request actions and control objects but do not unilaterally declare game outcomes. A control lease can be transferred from one player to another, but it is permission to **drive an object**, not final authority to rewrite all other players' state.

Provide a compatible server-side simulation/collision layer in Rust (evaluate **Rapier 3D** rather than writing a physics engine) and author-specified collision proxies. Godot may handle local visual physics; exact deterministic matching between Godot and Rust is **not required**, because shared results come from server state.

Implement:

- Interactive static switches and doors.
- Movable objects and player-held props.
- Vehicles and projectiles, including a tank battle.
- Server-governed collisions, interactions, scores, and damage.
- Physics ownership/control handoff, including disconnect and release handling.
- Dynamic instantiation and deletion of **prefabs already delivered in the world package**.
- Reliable object creation/deletion and initial state, plus rate-controlled transform updates.

For hair, clothing, ear movement, and other secondary/avatar effects, transmit authoritative events/constraints and let clients solve the resulting physics locally.

## 16. Initial state, late join, and resynchronization

A joining or reconnecting client must be able to reconstruct the same live instance without having observed earlier interactions.

Required protocol behavior:

1. Authenticate and authorize admission to the instance.
2. Identify the immutable world asset version and required prefabs/avatar assets.
3. Transfer a consistent **snapshot** of the current entity graph, transforms, control leases, replicated script variables, animation states, active interactions, radio/media objects, and current video playback state.
4. Queue subsequent ordered authoritative changes during snapshot transfer.
5. Apply the snapshot, then queued changes, then live unreliable pose/transform streams.
6. Discard outdated samples by version/sequence number and safely recover from a missed or invalid baseline.

A reconnect does **not** replay the person's old local state over the instance. It receives the current server state. Snapshotting is in memory and does not imply persistence across empty-instance destruction.

## 17. Creator scripting runtime

Creators use **Luau** for both world and avatar scripting. The project uses GDScript for ordinary client logic and Rust for backend logic.

- Run **authoritative world Luau** in the Rust instance server, using a mature host such as `mlua` with Luau support.
- Run permitted **client-side Luau** for visual/audio/animation behavior using a compatible sandboxed Godot integration.
- Expose a **project-owned, documented API**; do not expose unrestricted Godot objects, Rust native functions, OS calls, arbitrary networking, or arbitrary filesystem access.
- Keep the API semantics consistent across server and client while exposing only capabilities appropriate to the execution side.
- Enforce execution time/instruction limits, memory limits, event/rate limits, and cleanup on instance termination.
- Every script declares its attachment, execution side, and requested permitted capabilities in package metadata. Validate the manifest before publication.
- Define deterministic authorization rules for any client request that changes shared state. Arbitrary client scripts cannot authoritatively rewrite server state.
- Script errors must be isolated so one broken script does not crash the client/server or other instances.

### 17.1 World API capabilities

World creators must be able to:

- Receive interaction, touch, join/leave, grab/release, timer, and collision events.
- Get objects by stable ID/tag and inspect allowed properties.
- Read/write explicitly replicated instance variables.
- Change supported lights, material properties, object visibility, and animations.
- Play spatial sounds and configure supported acoustic zones.
- Start/stop/seek synchronized video playback.
- Use supported public/private voice routing and radio objects, subject to admin/user permissions.
- Request server-authoritative entity motion, physics, attachment, control leasing, damage, and score changes.
- Instantiate, customize, and destroy prefab objects **already packaged with the world**.
- Implement substantial interactive games such as multiplayer tank battles with shared rules, projectiles, health, and scores.

### 17.2 Avatar API capabilities

Avatar creators must be able to:

- Define actions and toggle any supplied accessories, meshes, or clothing layers.
- Control authored animations, expressions, bones, permitted material parameters, and spring-bone behaviors.
- React to touch/head-pat/hair-grab and other authorized interactions.
- Control custom avatar-local sound and visual effects.
- Replicate permitted discrete action/animation states so everyone sees the same avatar choices, without transmitting every bone transform.
- Use the normalized tracking/IK state, including whether relevant joints are tracked, estimated, or animation-driven.

The avatar script can control its owner's avatar, **not** silently alter another player's avatar, override a block, or escape the sandbox. Explicit or comedic visual animations are allowed; platform permission rules still apply.

### 17.3 Minimal conceptual creator API

The following illustrates the desired **capabilities**, not frozen API names or signatures:

```lua
-- Authoritative world script (illustrative)
world.on_interact("light_switch", function(player)
    world.state.set("lights_enabled", not world.state.get("lights_enabled"))
end)

world.on_interact("tank_fire", function(player)
    world.spawn("shell_prefab", {
        owner = player.id,
        transform = world.object("tank_turret").muzzle_transform
    })
end)
```

Do not build compatibility with Roblox's entire API or allow world creators to call unrestricted Godot methods. The final API should be small, composable, typed/documented, and independently versioned.

## 18. Audio architecture

Use **Opus** for microphone voice transport and for all **uploaded standalone audio files** (Ogg Opus, `.opus`). Files uploaded inside avatar or world packages must not use MP3, WAV, Vorbis, AAC, or other standalone audio formats. Decode to internal PCM as needed for playback and mixing. External **video media** may use whatever audio codec the media decoder supports; this does not make it a permitted package-audio format.

Use Godot's native audio buses, 3D placement, and effects wherever possible. Add a thin mature codec integration for live Opus and packaged Opus loading where native Godot APIs are insufficient.

### 18.1 Voice

- Always provide proximity/spatial voice and distance attenuation.
- Support world-authored sound routing, effects, and named acoustic zones.
- Support reverb, echo/delay, noise/distortion, compression, and both processed and dry signals.
- A world creator may make a small room extremely reverberant or a large room completely dry; effect strength is **not inferred or mandated by room geometry**.
- Provide a world option for **realistic acoustics**, using inexpensive approximations such as occlusion, attenuation, and zone-based effects. This is an optional acoustic simulation mode, not a physically exact propagation engine.
- Voice quality, bitrate ceilings/floors, and available effect ranges are administrator-controlled with sensible defaults.
- World settings may configure effects only within the administrator's allowed parameters.

### 18.2 Private calls, radios, and proximity bypass

Support non-positional private calls and radio voice paths that can bypass normal proximity. The world creator controls whether bypass is allowed or how it interacts with the world's acoustic environment; users also have a preference for spatial voice, private call, or automatic switching when two call participants are physically near one another.

Define precedence and mixing policies so someone audible over a call and nearby in-world is not accidentally heard twice. Use selection/crossfading or an explicitly designed mixing policy. Blocking and muting always take precedence over world scripts.

Private calls require authorization and must be routed only to their participants.

### 18.3 Text chat

- Support text chat associated with players, with short messages visible **above their avatar's head**.
- Send text over reliable networking.
- Apply blocking/muting and permissions to displayed messages.
- Ensure text is readable in VR without requiring a desktop UI.

## 19. Global cross-world amateur radio

Provide a **virtual radio network** allowing radio objects in otherwise separate world instances to communicate. World builders opt in by placing an object/component with a standardized radio tag and configuring its channel(s). Worlds without radio objects do not automatically receive transmissions.

Required modes:

- Very-low-quality radio **voice**.
- Built-in **SSTV** image transmissions.
- Built-in **FT8-style** digital message transmissions.

The administrator configures allowable channels, quality presets, Opus bitrates, bandwidth-like filters, noise, interference, and rate limits. World creators configure their radio objects within those policies.

A narrowband/low-bitrate path may deliberately degrade quality. The system does **not** need clean decoded images: visually poor SSTV is acceptable and desirable as a radio effect. However, a built-in feature must still be demonstrably usable. Recognize that aggressively lossy Opus can destroy real FT8/SSTV modem tones; implement internal transmission/decoding or a protected signal representation when needed while applying audible/visual degradation in a controlled way. Do not promise arbitrary external modem decoding through an 8 kbps voice path.

An in-world radio's audio should be spatialized around its speaker, and may use world audio effects as permitted by routing configuration. The cross-world radio service is separate from individual ephemeral instances; its current channel activity is transient and must not be stored as gameplay state.

**Future only:** Interoperability with external programs such as WSJT-X or SSTV software using real audio waveforms/PipeWire virtual devices. Do not require this for the initial release.

No real RF transmissions are part of the system.

## 20. Synchronized in-world video

World creators can place video surfaces/players and expose controls through Luau.

Required sources:

- **YouTube** videos through a maintained yt-dlp-compatible resolver.
- Direct video URLs/files hosted by a **publicly available server**, using mpv-supported network formats.

Prefer existing **libmpv/mpv** integration and yt-dlp instead of writing a decoder. Check extension compatibility with the pinned Godot release and Linux/OpenXR rendering. Video may display on a world-space mesh/surface; decoded audio must enter the intended in-world audio path.

The **server** owns media session state and a shared time reference: source, start time, play/pause, seek, rate, and current target time. All clients receive authoritative media commands, retrieve streams **directly from the public media source**, and correct drift against the shared timeline. Late joiners receive the current playback position. Do not proxy/relay the whole video through the game server by default.

Define who may control each player (world script rules, instance role, or object ownership) and ensure actions are synchronized even at reduced network TPS. Media URL fetching/resolution must be constrained to media-specific code, not exposed as unrestricted HTTP from creator scripts. Support failure messages for removed, restricted, or unsupported media; YouTube availability can vary with external platform changes.

## 21. In-VR interface

The main menu is a **virtual smartphone** spawned/held in either the left or right hand, chosen by the player. It is the normal interface for game controls, world discovery, instance joining/invitations, avatar selection, calls, user interaction options, and settings.

Holding the controller menu button opens a **circular radial quick-action menu** controlled by the controller stick, similar in concept to VRChat's quick menu. The long-press duration and quick actions should be configurable. Do not replace the smartphone with the radial menu; they have separate purposes.

Build menus from **native Godot UI controls** and 3D surfaces. AI-generated art can supply initial icons, textures, and decorative assets but not replace actual functional UI layout/components. All menus must support VR pointing and readable scaling.

Users must have quick access to mute, audio settings, blocks, touch permissions, avatar actions, world/instance controls, video controls (where permitted), and reduced network update-rate preferences.

## 22. User settings, privacy, and blocking

Each user can configure at least:

- `can_touch_me`.
- `can_touch_others`.
- Their voice playback/recording preferences and input/output devices.
- Preferred behavior when a private-call participant is within proximity.
- Shader quality tier, resolution scale, antialiasing, supported upscaling, texture reduction, and other rendering options.
- Requested network update-rate divisor (including full, half, and quarter rate as examples).
- VR interaction/menu preferences and comfortable sleeping pose.

Provide **several levels of blocking**, not just a single all-or-nothing button. Distinguish at minimum voice mute, text hiding, contact prohibition, avatar hiding/replacement, and complete block. Define a single clear precedence rule: safety/privacy blocks override world code, calls, radio channels, animation requests, and contact events. Avoid leaking private voice/message content to a blocked participant.

Privacy settings are persistent account data, even though instances are ephemeral.

## 23. Reliability and resource handling

The complete product must be usable for long VR sessions, including sleeping in VR, without progressive memory growth or common crashes.

Required behavior:

- Load world/avatars asynchronously or in bounded work units to avoid unnecessary XR freezes.
- Cache and identify assets by hash/version; handle broken downloads cleanly.
- Release world, video, voice, audio, scripts, physics, and avatar resources when no longer referenced.
- Tolerate headset/runtime focus loss and resume as well as supported sleep/wake behavior.
- Recover from network interruptions by resynchronizing with the existing instance **only if it still exists**.
- Keep local headset/controller motion responsive even during temporary network problems.
- Use bounded queues, script resource budgets, and controlled memory allocations; fail or substitute visible fallback representations rather than crashing on excessive assets.
- Never pretend that unlimited model complexity guarantees a minimum FPS. Do not impose an arbitrary polygon cap merely to make such a claim.
- Keep per-instance failures isolated where practical so a broken world script does not terminate all other instances.

Quality reductions must never change the authoritative state, remove world interactions, or silently drop required reliable updates.

## 24. Configuration policy

All server-quality settings are owned by the **administrator**, with documented sane defaults. Examples include:

- Base simulation TPS (default proposal: **60 TPS**).
- Minimum/maximum per-client input/output network update rates and automatic rate adaptation.
- Enable/disable tick aggregation/coalescing optimization for low-rate connections.
- Voice Opus bitrate/routing profiles, radio quality profiles, audio-effect bounds.
- Maximum instances/players and world-defined capacity ceilings.
- Script execution/memory/event budgets.
- World/avatar slot counts and package size limits.
- Media source policy and allowable radio channels.

World authors may specify allowed world-local behavior (effects, instance capacity, scripts, media/radio objects); users control their personal quality and interaction choices **within the server's limits**.

Do not confuse three policy scopes: **admin upper/lower bounds**, **world-authored behavior**, and **client-local presentation/preferences**.

## 25. Security and isolation for a private project

The platform is private, but untrusted uploaded packages and network clients still require basic isolation. Prioritize getting the complete working product built; avoid building an enterprise security framework.

Minimum design requirements:

- Deny access without Discord OAuth plus whitelist, except protected administrator login.
- Validate uploads and isolate their Luau scripts with strict capabilities and budgets.
- Never run creator native binaries or unrestricted Godot scripting/resources.
- Verify user permissions on the server before applying shared state changes.
- Separate reliable state management from untrusted media URLs and script APIs.
- Authenticate ENet game sessions and use a tested secure transport/wrapping solution for remote deployment; see Section 13.1.
- Do not mistake privately listed avatars for unrecoverable/encrypted client assets.
- Ensure malformed or outdated network packets fail safely.

## 26. Protocol and compatibility contracts

Version independently:

1. HTTP API.
2. Realtime ENet wire protocol.
3. World package manifest.
4. Avatar metadata/schema.
5. Creator Luau API and built-in object/component definitions.
6. Server/client application compatibility.

Use stable IDs for instance entities, packaged prefabs, script components, and replicated variables. Define coordinate system, units, serialization types, timing/clock conventions, handedness, message limits, and backwards-compatibility policy before shipping creator examples.

A world must provide collision/interaction metadata when visual geometry alone is insufficient. Replicated variables are explicitly declared, typed, bounded, and authored/changed only by the appropriate authority. Distinguish reliable events from ephemeral pose samples, with consistent join snapshot semantics.

## 27. Acceptance criteria

The complete first release is accepted only when all of these scenarios work:

1. `nix run .#client` launches the NixOS PCVR client; the NixOS server package and module run the complete backend and website.
2. An admin whitelists a Discord account; the user signs in on the website and game. Unlisted accounts cannot access the service.
3. A user uploads, replaces, and deletes worlds/avatars from the website with enforced configurable **4 world / 8 avatar** slot defaults and **1 GB / 100 MB** package defaults.
4. A VRM avatar displays with expressions, spring bones, fallback body IK, scripted visual actions, and user-selectable avatar slots; users without uploads see the minimal default avatar.
5. A user-created GLB world loads, is searchable by authorized users, and supports public and private/invite-only ephemeral instances.
6. Two or more clients interact with a switch/door and see the same state. A complex scripted example supports moving tanks, projectiles, object spawning, hit detection, and synchronized results.
7. A late join receives authoritative state; a reconnect recovers the state **only while at least one player kept the instance alive**. Leaving the last player destroys it immediately. A fresh instance has no previous game state.
8. The server runs a configurable base simulation TPS; one client requests half/quarter input/output update rates without reducing other clients' rates. The server may perform optional tick coalescing without changing outcomes.
9. Remote player poses interpolate only between received samples and **freeze at the last known authoritative pose** when updates cease. The local user's OpenXR headset/hands remain immediate. No speculative movement/rollback is implemented.
10. Head pats and hair grabbing work with bilateral permission checks, server-authoritative interaction events, and client-local cosmetic spring-bone physics.
11. Full-body tracker input works where OpenXR supplies it. The same avatar remains usable with simulated FBT, sitting/lying/sleeping postures, and no trackers beyond head/hands.
12. A world creator attaches sandboxed Luau scripts; scripts execute server-side/client-side according to their permissions, synchronize intended changes, and cannot access arbitrary OS or engine APIs.
13. Opus proximity voice, text above heads, private call/radio routing, world-defined reverb/echo/noise/compression, and optional realistic acoustics work. Admin voice-quality policies apply.
14. Radios placed in two different world instances can communicate using low-quality voice, in-app SSTV, and in-app FT8-style messages; a world without a radio is not forced into radio playback.
15. Multiple clients watch one synchronized YouTube video and one direct publicly hosted video. Play/pause/seek/late-join state remain synchronized.
16. The smartphone can appear in either hand and the menu-button long press opens a stick-controlled radial quick menu.
17. All three rendering tiers apply to **both avatar and world**. Ultra unlit/dithered mode still supports ambient brightness and all essential interactions. Available antialiasing, resolution/upscaling, and texture-reduction settings work according to renderer capabilities.
18. Multi-level blocking, `can_touch_me`, and `can_touch_others` work consistently against world and avatar scripts.
19. Repeated join/leave, avatar swaps, media playback, and extended VR sessions do not exhibit uncontrolled memory growth. Invalid media/assets/scripts fail without taking down the whole application.
20. Native Godot ↔ Rust ENet interoperability is proven through real integration tests, including reliable, sequenced, low-rate, and reconnect scenarios. Remote-session encryption/authentication is tested rather than assumed.

## 28. Implementation order (does not reduce release scope)

Recommended integration order to reduce wasted work:

1. Set up the flake, Rust/Axum/SQLite backend, static website, and Discord allowlist/authentication.
2. Build an **actual** Godot ENet ↔ Rust ENet compatibility and secure-transport test; prove rate negotiation and snapshots.
3. Load world GLB packages and VRM avatars at runtime; implement material normalization, three quality tiers, and an OpenXR rig.
4. Implement instance creation/destruction, world discovery, public/private joining, authoritative transforms, and snapshot/reconnect.
5. Add sandboxed Luau on both sides, versioned creator API, prefab spawning, and basic/complex world interactions.
6. Complete body IK/FBT, sleeping poses, contact permissions, and client-local secondary physics.
7. Complete Opus voice, spatial and zone effects, private calls, global radio/SSTV/FT8, and text chat.
8. Complete mpv/yt-dlp synchronized video, smartphone/radial UI, user/admin settings, performance/reliability tests, and packaging.

These are engineering milestones, **not** permission to omit later items from the first release. Keep end-to-end behavior testable as components are integrated.

## 29. Explicit future possibilities (not part of first release)

- External radio software interoperability through PCM/audio devices such as PipeWire/WSJT-X/SSTV decoders.
- Windows packaging and general distribution to other Linux desktops.
- Standalone-headset/mobile/browser clients.
- Alternative Godot renderers and optional ray tracing if supported in a future stable engine.

Do not silently add an avatar marketplace, public avatar sharing, persistent world-instance saves, a custom rendering engine, or cloud/microservice infrastructure; those contradict or exceed the agreed scope.

## 30. Useful implementation references

- Godot stable release archive: https://godotengine.org/download/archive/
- Godot ENet connection API: https://docs.godotengine.org/en/4.7/classes/class_enetconnection.html
- `rusty_enet`: https://docs.rs/rusty_enet/
- Godot renderers: https://docs.godotengine.org/en/4.7/tutorials/rendering/renderers.html
- Godot runtime file loading: https://docs.godotengine.org/en/4.7/tutorials/io/runtime_file_loading_and_saving.html
- Rust `mlua` (Luau support): https://github.com/mlua-rs/mlua
- Godot Luau extension candidate: https://github.com/fernforestgames/luau-gdextension
- VRM specification: https://vrm.dev/en/vrm1/
- mpv: https://mpv.io/
- yt-dlp: https://github.com/yt-dlp/yt-dlp

**Implementation directive to AI agents:** Treat the functional requirements and explicit exclusions above as authoritative. Treat named third-party integration libraries as candidates until tested against the pinned toolchain. When a listed library is incompatible, substitute the smallest maintained implementation that preserves the specification; document the choice and do not silently drop the feature. Do not add unnecessary abstraction, new infrastructure, or persistent instance state.
