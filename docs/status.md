# Initial-release status

The full repository specification remains required. The work below is not a
scope reduction or a declaration that unfinished requirements are optional.
Headless tests establish integration behavior, not headset comfort or complete
production security. There is no development account-login bypass in the app;
tests seed their own isolated temporary database sessions.

## Current verification update

Compilation/testing has resumed. The former deferred implementation batch now
passes local Clippy and its native/server unit regressions (43 native tests,
86 server tests; isolated child probes are launched by their parent tests).
The headless settings suite passes material/light/ambient/animation, acoustic
noise/occlusion, renderer admission, fair scheduling, focus, transport transitions,
cache pins and import-queue/cancellation fixtures. Local actual-GLB/VRM, packaged
audio and helper-process video/PCM regression runs also pass. Older "unverified"
notes below describe the prior verification state, not new passing evidence for
hardware or deployment acceptance.

New implementation includes packet-owned outgoing ENet reservations (4 MiB/peer,
64 MiB/process), real slow-peer queue teardown coverage, serialized FIFO GLTF
imports, and a separate Landlock-confined media helper with bounded IPC and
per-process resource limits. Helper initialization/file-source rejection and
HTTP video/PCM tests pass. An opt-in live YouTube helper run also passes after
enabling separate bounded-height video/audio format selection.

Two actual headless Godot peers now also pass the refreshed isolated-helper
YouTube **and** public HTTPS MP4 synchronization paths, including pause/seek,
late join and resume. The latest full local workspace/all-targets run passes 43 native
unit tests, 86 server unit tests, two helper integrations and three server
integrations (including the website DOM tests). A later rendering refinement adds
alpha-hash Tier-0 transparency, authored cutoff/UV/roughness restoration, fixed
MToon uniform interpretation and tier-independent retained texture accounting;
its focused headless fixtures pass. Persistent draw-distance control (10–2000 m,
default 250 m) now has server bounds/default migration, phone/camera and website
numeric-preservation regressions.

The latest full local pass also includes non-blocking avatar-import retirement
(with a semaphore-blocked worker regression), finite/depth-bounded material
uniform validation, safe normalized color defaults and composed-transform
overflow/affine checks. A preceding all-targets attempt exceeded the tool's
wall-clock limit while compiling/testing alongside a release build; the rerun
completed successfully without that external limit. This timeout was not counted
as a passing run.

The newest health change exposes packet-owned realtime-output reservations and
both byte ceilings through the authenticated administrator endpoint, with unit
and real HTTP integration assertions. Media-worker regressions cover the
fork-to-exec parent-death race, expired read/write deadlines and nonfinite PCM.
The full pass includes real descendant descriptor teardown and explicit
process-group escape-denial probes. Client/server
packages pass for this lifecycle/health snapshot and the subsequent loader and
cache-lock regression corrections.

That refresh exposed a cache-lock test race under concurrent subprocess startup:
a forked child briefly inherits an OFD pin until exec despite `CLOEXEC`. The
evictor correctly returns busy in this interval. The regression now requires
bounded eventual release after the final reader closes, and an additional
descriptor-clone test explicitly checks retained OFD ownership. This correction
passes Clippy, the full local workspace suite and refreshed client/server package
checks. Ten consecutive native-unit-suite runs also pass (43 tests each), including
parallel subprocess startup and cache pin retirement. The failed package attempt
is not a pass.

The subsequent loader reentrancy fix queues replacement requests made from
completion/failure callbacks until old import cleanup finishes, and binds failed
cache removal to the completed path. Its headless cancellation/retirement/cache
fixture and the full workspace regression pass for this addition.

The real encrypted ENet integration now passes through GLB/VRM imports, headless
settings, positional video/PCM/failure recovery, contact/avatar policies,
cross-world radio, physics, GC and ephemeral instance teardown. A separate fresh
1,000-cycle avatar/audio/Luau replacement stress run passes. Combined Nix client/
server packages now also pass for the rendering/draw-distance, asynchronous
retirement and material/composed-transform hardening snapshot. The packaged launcher
also starts headlessly from a fresh writable cache without script errors.
Importer crash/peak-memory isolation, aggregate decoder-descendant resource guarantees, GPU/headset/audio
acceptance, long sessions and live OAuth/NixOS deployment remain separate gaps.

## Working and exercised by automated integration tests

- Real low-level Godot 4.7.2 ↔ rusty_enet traffic, payload encryption/MAC,
  malformed packet rejection, reliable state/text, and independent full/quarter
  pose rates. Cipher unit tests also cover replay, tampering, and direction.
- GLB world packages and actual packaged prefab meshes, not just placeholders;
  bounded received-sample interpolation and holding the latest known transform.
- In-memory instances, late join/reconnect, immediate destruction on the last
  disconnect, and a persistent catalog independent of gameplay state.
- Rapier 3D authority, doors/switches, props, exclusive leases, stale vehicle
  input stopping, disconnect cleanup, packaged scripted spawning, projectile
  collisions, damage, destruction and scores in the tank-yard example.
- Server Luau time/memory isolation and bounded command queues; client Luau
  loaded through a JSON-only native bridge, capability restrictions and watchdog.
- Procedural VRM 1.0 fixture with humanoid mapping, expression blend shapes,
  spring bones, native limb IK, local/sleeping poses, and a synthetic native
  `XRBodyTracker` input with estimated fallback for unavailable joints.
- Owner-only avatar selection, immutable equipped version identifiers,
  replicated declared actions, bilateral contact opt-in, server-owned hair
  grabs, and block-driven release. Cosmetic grab constraints run locally.
- Real libopus encode/decode through Godot, encrypted proximity routing and
  voice blocking; authorized private-call invitation/acceptance, single-path
  spatial/non-positional routing, and exclusion of nearby nonparticipants.

## Implemented but not fully accepted

- Optional world ray occlusion and bounded listener-zone noise are newly
  implemented and unverified. Queries rotate in the physics callback; private
  calls bypass obstruction and zone effects. Generator lifecycle, exclusions and
  dry-route regression sources are written but unexecuted. This approximation is
  not full geometric acoustics; audible/hardware acceptance remains outstanding.

- Discord OAuth plus allowlist and protected administrator secret: live OAuth
  validation requires the deployment's Discord application/configuration.
- World/avatar upload, versioning, replacement, deletion and quotas: validation
  still needs deeper malformed-resource/fuzz coverage and bounded texture/
  allocation validation. Do not treat the current validator as fully hardened.
  Bounded real ZIP-length/CRC verification and publication-time Luau UTF-8 checks
  pass the full local regression suite. Exact GLB chunk lengths, embedded buffer
  ranges and bounded acyclic single-parent hierarchies pass local integration.
  Accessor/sparse ranges and declared decoded-array budgets pass local integration.
  Maintained PNG/JPEG/WebP decode, dimension and per-image/package allocation
  checks, including duplicate-view budget charging, pass full local workspace
  regression and Clippy. These do not replace index-content validation,
   importer/GPU budgets or aggregate concurrent-upload limits.
   New unverified hardening adds glTF scene/skin/morph/animation/texture/light
   reference validation, bounded VRM expression/spring work, and streaming BIN
   checks for actual index bounds, finite floats, sparse ordering and overlaid
   animation-time ordering. New regression sources are unexecuted; see
   [asset-validation.md](asset-validation.md).
- Private instances/invitations: implemented; the full lifecycle/privacy matrix
  now has registry tests for visibility, invitation identity/expiry/consumption,
  full-instance rejection, owner/member return and empty-instance destruction.
  Failed new-instance construction/capacity and private admission now preserve
  previous membership/connections. Live takeover preserves authoritative state
   and releases old constraints; real-peer tests pass. Voice epoch restarts and
   stricter peer-error checks pass full local and combined Nix package checks.
  Broader deployment/privacy acceptance remains.
   New unverified client work stages replacement transports until admission,
   explicitly tears down on disconnect, resets by instance identity, and discards
   cancelled loader generations before scene publication. New headless transition/
   cancellation fixtures are written but unexecuted.
- Native OpenXR rig, smartphone/pointing, and tracker adapter: real hardware,
  focus loss/resume, standing/crouching/sitting/lying and comfort checks remain.
  New unverified focus handling now watches desktop/OpenXR focus and presence,
  mutes capture without automatic re-enable, cancels gestures/control input,
  releases authoritative constraints and resumes without reacquiring them.
  New headless focus/privacy/lease fixtures are written but unexecuted.
- Material tiers apply to worlds/avatars, but the complete standardized advanced
  feature schema, presentation controls, texture policies and quality testing
  are not finished. The quick menu now draws eight circular stick-selectable
  actions with a cancel dead zone; XR placement and usability remain unverified.
  Import-time texture resampling now has a separate persistent 0/1/2/3 setting
  (original/half/quarter/eighth dimensions), shared by worlds/prefabs/avatars.
  Full local regression tests verify actual texture dimensions, all material
  tiers and account persistence; Nix client/server checks pass. Existing content
   needs reloading; physical GPU memory and visual-quality acceptance remain.
   New fixed client-side `set_material`/`set_light` commands are implemented but
   **unverified**: compilation and tests are paused at the user's request.
   Deferred coverage includes flat-value validation, shared-resource isolation,
   material-override precedence, scoped targets and light-group tier persistence.
   World-only global ambient tint/brightness (including Tier-0 darkness) is now
   implemented with an incremental recoloring walk, untinted base preservation
   and teardown reset; it and its new regression cases are also unverified.
   A fixed advanced schema now adds bounded outline/rim/specular controls,
   project-generated matcaps and per-package/prefab material declarations, with
   tier approximations and deferred regression cases. It remains unverified;
   complete GPU/visual acceptance is not established.
   Supplied client animation play/stop/seek controls and import-time track
   sanitization are also implemented but unverified. Only own-model spatial/
   bone/blend-shape tracks remain; arbitrary method/audio/property tracks are
   stripped, with deferred regression coverage for escape and resource isolation.
  Persistent eight-sector action bindings and long-press timing now have phone/
  website controls, allowlisted actions, a disabled-slot option and per-gesture
  policy snapshots. Full local workspace/Clippy checks, targeted phone-control
  emission tests and website JavaScript syntax checks pass. Combined Nix client/
  server checks also pass.
- Voice capture/playback and local jitter buffers need real microphone/audio
  device and long-session tests. Named source zones now route voice/video
  through native Godot reverb, delay, distortion, compression and limiter buses,
  under an administrator effect ceiling, with a dry call path. Noise, optional
  occlusion and complete scripted routing remain unfinished. Persistent audio
  device/gain controls and conservative rendering/AA/upscaler settings have local
  tests; physical device and renderer/XR quality acceptance remain. See [settings.md](settings.md).
- Packaged standalone Ogg Opus now has a thin libopusfile loader, bounded PCM
  feeding native 3D players, world/avatar declarations, local looping, shared
  source-zone effects and capability-gated creator play/stop/seek commands.
  Godot tests pass decode, client-Luau commands, looping/drain and cleanup;
  server tests cover capability/declaration checks and reject spoofed Opus
  headers. The typed-state/packaged-Opus Nix client/server checks pass.
- Creator APIs now validate typed replicated variable declarations/defaults and
  reject undeclared or incompatible writes. Complete event/capability coverage
  and audio/media/radio APIs remain; current APIs are experimental.
  Bounded VM-local one-shot timers pass local checks on both hosts, including
  timer-only server scripts, cancellation, shared CPU budgets and teardown.
  Authoritative damage/destruction callbacks are now wired through a bounded
  deferred queue instead of being accepted but never dispatched; regression
  and Nix package checks pass. Automatic avatar-contact cleanup/disconnect
  release callbacks now use that queue and pass local workspace regression.
  Client snapshot-derived releases and explicit-packet deduplication pass targeted
  and full local/Nix package tests. Avatar-change cleanup now removes grabs tied
  to the previous skeleton; unit/real-peer tests and combined Nix client/server
  checks pass. This does not complete
  all creator event/API gaps.
  Bounded object/prefab tags and paginated authoritative `world.find_tag` queries
  now pass local integration, including detached data and inheritance,
  publication/live tag-byte budgets and budget release. Prop/vehicle control
  events now cover acquisition, explicit release, handoff, expiry, destruction
  and disconnect, with a hand-constraint cleanup fix. Their full local/Nix package
  checks pass, including omitted inapplicable hand fields so Lua sees `nil`.
   Client-world shared JSON-only object/tag queries now compile cleanly and pass
   targeted Godot tests for startup availability, detached tables, live poses,
   stale graphs/deletion, isolation, nil semantics and teardown. Query-only
   combined Nix packages and full local regression pass. Reliable client
   control/damage/destruction events now pass local regression and combined Nix
   package checks, including bounded deferred dispatch and real-peer lease/release
   delivery. Avatar-local bounded bone rotation/reset and spring-gravity commands
   pass full local native/Godot/VRM checks; 1000 avatar/audio/Luau replacement
   cycles with bone overrides pass bounded settled-count/teardown checks.
   Actual imported-VRM modifier ordering is now checked: bone overrides and
   cosmetic grabs follow the importer's internal spring modifier. Custom hair
   bone lookup now bypasses invalid humanoid-profile queries without engine errors.
   Read-only client-world `world.state.get` now passes full local regression and
   combined Nix client/server package checks, including startup, detached-vector,
   false/nil, stale-tick and teardown checks. The subsequent modifier-order and
   custom-bone lookup fixes pass local integration, another 1000-cycle stress
   run and combined Nix client/server package checks. Opt-in avatar-local
   node/bone queries now pass targeted Godot tests for startup, live detached
   samples, per-script capabilities, foreign-avatar isolation, deletion and
   teardown. Full local native/server/Godot and actual imported-VRM regression
   passes; combined package verification is pending. Complete
   object/light/material APIs remain.
   Client-world `world.media` now uses the authenticated player-control path,
   with shared native/server request validation, attachment restrictions and an
   aggregate client request limit. Full local native/server/Godot and real-peer
   tests pass, including owner denial and synchronized load/seek. The server
   creator command queue additionally has an aggregate 64-KiB owned-payload
   budget and bounded IDs; full local regression and combined Nix client/server
   package checks pass.

## Not yet complete

- Radio completion: cross-world Opus, protected SSTV/FT8-style payloads and
  world opt-in now have integration coverage. Admin channel/bitrate/filter/noise
  and digital rate policies are wired, with transient overlapping-sender
  interference, illustrative tones/hiss and local image degradation. Creator
  Client-world creator FT8/SSTV requests, listener-wide preset UI, admin preset
  allowlist and avatar-sound voice blocking pass local/package tests. Server
  scripts now transmit only for authenticated interaction initiators, through
  the same router/cooldowns; local cross-instance tests pass. Per-object named
  speaker presets now have typed publication declarations, independent filter
  buses and administrator-allowlist fallback; full local workspace regression
  and Clippy pass. The combined package check passed after an earlier intermittent
  synthetic tracker assertion, whose fixture initialization is now corrected.
  Real headset/audio checks remain.
- Video acceptance: server-owned media timelines, owner/public controls,
  world-space textures, positional PCM, drift correction and the packaged
  yt-dlp hook are implemented. A single headless client passed decode,
  pause/seek/resume and clean teardown against both a public direct MP4 and
  live YouTube and public HLS. Restricted/removed-source cases and long-session
  testing remain. Offline HTTP 404/403/unsupported-container failure messages,
  stopped failed generations, actual decode/PCM recovery and teardown pass full
  local/Nix integration; they do not establish live YouTube restriction handling.
  Two actual Godot peers also passed public MP4 and YouTube
  decode/PCM convergence, pause/seek, late join and resume against authoritative
  encrypted server timelines. Public-only media enforcement uses a checked
  client-local relay and decoder-thread Linux socket guard. Offline thread/exec
  inheritance, private redirect/HLS and local-file M3U/EDL canary tests pass.
  Async DNS cancellation and joined worker shutdown are added; media-hardening
  Nix client/server package checks pass. This is not a general process sandbox
  or complete decoder/resolver memory budget; see [media-security.md](media-security.md).
- Full administration/settings acceptance and complete resource budgets remain.
   New per-peer min/max rates, asymmetric fractional negotiation, hysteretic
   ENet-quality adaptation and optional unchanged-sample aggregation are
   implemented but **unverified**, with deferred policy/client regression cases.
   Reliable state/events remain independent of pose scheduling. See
   [networking.md](networking.md). Immutable-version garbage collection
  now has exclusive data-directory ownership, coordinated reference handoffs,
  catalog/live-world/avatar/grant pins, bounded background passes and an
  administrator-only manual pass. Full local/Nix integration passes, including
  deleted-catalog live downloads, pending-grant handoff and final reclamation.
  Bounded concurrent upload/staging reservations, receive deadlines, cancellation
  guards and exclusive-startup orphan cleanup now pass full local integration,
  including stalled raw HTTP streams, 429 admission and disconnect cleanup.
  A global 64-server-VM ceiling, failed-admission rollback and prompt disabled-VM
  teardown pass full local regression; combined package verification is pending.
  Client-world join/leave/collision notifications, bilateral visibility filtering,
  load-time backlog preservation and deferred server collision callbacks pass
  full local regression. Explicit lock teardown now covers transient inherited
  descriptors. The administrator web GC form has authenticated/failure-safe DOM
  action tests, and client object notifications no longer overwrite a newer or
  same-tick authoritative graph with creation-time data. Combined Nix client/
  server package checks pass for these changes; avatar queries are newer and
  under verification.
   See [asset-storage.md](asset-storage.md).
   New aggregate native renderer reservations and packaged-audio memory/clip
   admission are implemented but unverified. See [resource-policy.md](resource-policy.md)
   for exact ceilings, fallback behavior and important importer/RSS/CPU gaps.
   New cooperative client-wide creator scheduling queues callbacks in order,
   rotates fairly across runtimes, coalesces ticks and yields at a shared frame
   target; deterministic scheduler fixtures are written but unexecuted.
   World-only acoustic-zone setters now coalesce/rebuild at most two declared
   zones per frame under administrator ceilings, with unexecuted regression cases.
   Live server creator callbacks now use a fair cross-instance queue with tick
   coalescing and execution-time authenticated-interaction checks; global live
   entity admission has RAII teardown and preserves old membership on failure.
   New scheduler/entity-capacity regression cases are written but unexecuted.
   Native fixed-joint entity attach/detach and authoritative rotation now include
   cycle/depth checks, lease release/conflict handling, snapshot/query metadata,
   deferred notifications and parent-destruction cleanup. New Rapier/API tests
   are written but unexecuted.
   Replicated Lua writes now decode by their declared flat shape before touching
   JSON, avoiding recursive host traversal of cyclic/deep creator tables. Bounded
   diagnostic formatting/log summaries and avatar/media request controls are also
   newly implemented and unverified.
   Server-world `voice` creator requests now bind call invite/leave to the actual
   authenticated interaction initiator, use the ordinary acceptance/block/policy
   router and have persistent bounded invite cooldowns. New tests are unexecuted.
  Completed
  client ZIP-cache eviction now has 1 GiB/128-file limits, in-memory recency,
  import pins, safe filename/symlink handling and focused/real-import tests.
  Full local/Nix package checks pass for completed ZIP eviction. Process-owned
  random partial filenames and dead-process cleanup pass full local and combined
  Nix package checks (subsequent stricter filename parsing has targeted coverage).
   Cross-process completed-ZIP pins, no-replace publication and pin handoff are
   newly implemented but unverified, with independent-descriptor/process-crash
   regression cases. Unowned legacy partials remain conservatively preserved;
   see [asset-cache.md](asset-cache.md).
- Extended VR/sleeping sessions, repeated avatar/world/media changes, leak and
  interruption testing, full NixOS deployment acceptance and external OAuth.

## Acceptance checklist (specification §27)

No row below waives any part of the corresponding scenario.

| # | Current evidence / remaining verification |
|---|---|
| 1 | Packaged headless client launched; full NixOS service/deployment and PCVR checks remain. |
| 2 | OAuth/allowlist implementation and isolated auth rejection tests; live Discord login remains. |
| 3 | Upload/catalog tests; complete replace/delete/quota/slot boundary matrix remains. |
| 4 | Procedural VRM fixture covers expressions/springs/IK/actions; representative uploaded avatars and XR remain. |
| 5 | GLB and ephemeral instances tested; private invitation/search matrix needs expansion. |
| 6 | Real peers exercise doors, tanks, projectiles, spawning, damage and scores. |
| 7 | Real peers exercise late join, reconnect and last-player destruction. |
| 8 | Full/quarter real-peer traffic and unit nondefault-TPS/asymmetric/adaptive/aggregation policies pass; packet-loss/load acceptance remains. |
| 9 | Received-sample interpolation/hold tested; real OpenXR responsiveness remains. |
| 10 | Permission/ownership/block release tested; real hand contact and cosmetic appearance remain. |
| 11 | Synthetic native body tracker/fallback tested; actual devices and complete posture matrix remain. |
| 12 | Both bounded Luau hosts tested; complete typed API/event coverage and security audit remain. |
| 13 | Opus/text/calls/radio and native zone effects pass locally, including noise lifecycle, obstruction/exclusions, private dry routing and creator-call consent; real audio/headset and full policy acceptance remain. |
| 14 | Cross-instance Opus/SSTV/FT8-style routing, blocking, channel denial and no-radio opt-out tested; client-world creator requests/presets locally tested; complete policy and real-audio checks remain. |
| 15 | Historical two-peer public MP4/YouTube synchronization plus current isolated-helper YouTube and HTTP/failure recovery pass; long sessions and new-helper multi-peer/package revalidation remain. |
| 16 | Hand-switchable phone and configurable eight-sector long-press radial menu have local persistence/gesture tests; XR usability remains. |
| 17 | Persistent scale/MSAA, conservative upscalers, texture reduction, fixed advanced schema, tier persistence and ambient darkness pass headless tests; visual/GPU/XR acceptance remains. |
| 18 | Voice/contact blocks tested; complete block/script capability matrix remains. |
| 19 | Packet-owned queue bounds, isolated media helper/IPC, FIFO imports and 1,000 synthetic replacement cycles pass locally; importer isolation, aggregate descendant budgets and extended headset/RSS/leak acceptance remain. |
| 20 | Real encrypted Godot↔Rust ENet suite covers reliable/sequenced/reduced-rate/reconnect traffic. |

Compilation/testing has resumed; see the current update above for new evidence.
The following records **historical packaged snapshots**, not verification of
every subsequent change. The most recent previously verified
`nix build .#server .#client --no-link` snapshot includes media hardening,
asynchronous DNS cancellation, joined decoder shutdown, CPU-time watchdogs,
typed replicated variables, packaged Opus, client radio APIs/presets, the admin
preset allowlist, avatar-audio blocking and authenticated server-script radio.
Persistent audio/rendering preferences also passed package checks. Newer
immediate teardown/error-reporting, lifecycle stress and creator timer changes
pass local regression checks, including the corrected numeric radio-policy
assertion after Godot's JSON round trip. Combined Nix client/server package
revalidation passed for timers, lifecycle cleanup, VM/command limits, radio pacing
and ZIP validation.
The native package checks passed, including isolated VM-limit/shutdown probes.
GLB structural/accessor checks also passed a subsequent Nix client/server build.
Image-decoder checks, including duplicate-view budget charging, passed full
local workspace integration, Clippy and exact-snapshot Nix client/server builds.
Import-time texture reduction, object radio speaker presets and deferred
damage/destruction events also passed combined Nix client/server checks. Newer
synthetic tracker fixture initialization and automatic server/client contact-
release callbacks and avatar-change cleanup also passed combined package checks.
Configurable radial-menu preferences/gestures also pass local workspace checks,
Clippy and combined Nix client/server package checks.
Radio/video test predicates now consistently compare Godot-round-tripped numeric
values numerically, with scenario-specific radio errors. A preceding loaded
local run timed out during radio; local regression passes with those predicates,
but broader packet-loss/load acceptance remains outstanding.
The check hook supplies temporary writable HOME/XDG directories; nonfatal
missing-default-fontconfig warnings remain in the sandbox. Checked packages do
not establish deployed NixOS or real PCVR acceptance.

Earlier local runs caught intermittent audio-generator cleanup warnings.
Radio, packaged-audio and video tests now observe weak playback references
until mixer/main-thread retirement, with bounded deadlines, rather than assuming
a fixed cleanup delay. Every playback created by seek/restart is tracked.
A separate headless smoke test passed 1,000 synthetic avatar/audio/Luau
replacement cycles with immediate decoder release and bounded settled node/
object counts; a rerun with pending timers and the client VM ceiling also passed.
This uses small synthetic model roots, not 1,000 full VRM imports,
does not bound every native allocation/RSS, and does not replace long VR sessions.
Repeated ENet runs also exposed false script watchdog trips under scheduling
load. Both Luau hosts now budget thread CPU time with a separate wall backstop;
regression and package checks pass after that change. Short public MP4 and
YouTube two-peer decode/PCM timeline runs also pass. These do not substitute for
the remaining long-session, hostile-resource and real-hardware acceptance.
An earlier loaded ENet run timed out during the private-call scenario. The test
driver now queues immutable, numbered commands rather than overwriting one
shared command file, and reports precise call/voice failure context. Regression
checks passed after fixing an initial reader-sequence off-by-one. Broader loaded
and packet-loss testing is still needed; retries alone do not establish reliability.
An intermittent packaged avatar test saw unexpected synthetic tracker joints.
The Godot tracker constructor does not initialize each joint flag, so the test
now publishes zero validity for every joint before enabling its single tracked
foot. It also checks the exact input joint set and fails immediately with source
diagnostics. This is a fixture correction, not evidence of physical tracker
driver behavior or completion of hardware acceptance.

## Dependencies and isolation decisions

`flake.lock` pins the GDScript VRM integration. Runtime hooks must run before
Godot's built-in ImporterMesh converter; otherwise expression bindings disappear.
A small preflight shim fills optional VRM defaults the pinned importer indexes
directly. Both fixes are covered by the procedural avatar integration test.

The examined Luau extension exposes sandboxing/interrupts but not the hard heap
budget needed here; the project instead hosts maintained `mlua` in a small
Godot-rust bridge with JSON-only inputs/outputs. The examined Opus candidate was
Windows-only with hardcoded quality; a thin binding to Nix's libopus supplies
Linux capture/codec support without implementing a codec.

The examined mpv Godot candidate renders through OpenGL/GLES. The project uses
libmpv's existing software render API instead, so video pixels can enter a
normal Godot texture under the Vulkan/Mobile renderer. No decoder or rendering
engine is being reimplemented. Single-client remote-source tests pass; complete
multi-client media acceptance and URL hardening remain.

Project Rust/GDScript/web code and development tools contain no Python. The user
explicitly permits Python solely as yt-dlp's runtime dependency. The Nix client
launcher supplies the pinned resolver to mpv's bundled YouTube hook; user mpv
configuration and external scripts remain disabled. Single-client live YouTube
playback is exercised; full synchronized playback acceptance remains outstanding.
