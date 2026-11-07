# Account and local presentation settings

The phone and website expose persistent account preferences through authenticated
`GET /api/v1/me` and `PUT /api/v1/preferences`. PUT replaces the preference record;
clients preserve fields they do not edit. Older records receive conservative
defaults for newly introduced fields. The server rejects invalid ranges/options.

| Preference | Supported values / default |
|---|---|
| `input_device`, `output_device` | Device name, 1–256 UTF-8 bytes without control characters; `Default` |
| `voice_volume` | Live proximity/call/radio voice gain 0–1; 1 |
| `microphone_gain` | Microphone gain 0–2; 1; does not unmute |
| `resolution_scale` | 3D render scale 0.5–1.5; 1 |
| `antialiasing` | `off`, `msaa_2x`, `msaa_4x`, `msaa_8x`; 2× |
| `upscaler` | `bilinear`, `fsr1`; bilinear |
| `texture_lod_bias` | Coarser texture sampling bias 0–3; 0 |
| `texture_reduction` | Original/half/quarter/eighth dimensions as 0/1/2/3; 0; next package load |
| `radial_hold_seconds` | Quick-menu long-press threshold 0.15–1.5 seconds; 0.45 |
| `radial_actions` | Eight action IDs in clockwise stick-sector order; original eight bindings by default |
| `radio_preset` | `standard`, `narrow`, `weak`; standard, subject to admin allowlist |
| `quality` | Existing basic/standard/advanced material tiers 0/1/2; 1 |
| `send_divisor`, `receive_divisor` | Independent 1/2/4 divisors; 1 |
| `phone_hand`, `sleeping` | Left/right phone and sleeping pose; left, awake |
| `can_touch_me`, `can_touch_others` | Independent opt-ins; both false |
| `call_mode` | `automatic`, `spatial`, `private`; automatic |

## Safety and application

- Missing saved audio devices fall back to `Default` with a visible notice.
  Missing microphone selection also mutes recording. Loading a preference never
  automatically enables the microphone; the user must explicitly enable capture.
- Device names are local to each computer. Account synchronization does not
  imply that a named device exists or works on every machine.
- The phone coalesces rapid saves into one pending preference state. Switching
  connection credentials cancels the previous account's pending request/state.
  Audio/rendering controls merge only their own fields, never stale touch flags.
- Voice gain preserves single-path call/radio/proximity routing. Voice and full
  blocks are enforced separately and override avatar sound scripts too.
- Material tier, phone-hand and sleeping controls now persist rather than only
  changing the current client session. Preferred radio profiles are stored but
  fall back to an allowed profile when an administrator restricts the list.
- Rate divisors apply to the next browser-generated connection grant. This UI
  does not implement live rate adaptation, administrator rate ceilings or tick
  aggregation. An already open website may need refreshing to load newer account
  preferences before generating its next grant.

## Quick-menu bindings

Phone and website controls configure all eight radial sectors and the hold delay.
Allowed action IDs are `microphone`, `sleep`, `phone_hand`, `interact`,
`avatar_action`, `video_pause`, `phone`, `leave`, and `none` (disabled). Repeated
bindings are allowed. Unknown actions or arrays other than eight slots are
rejected. Creators cannot supply arbitrary commands or replace these bindings.
The central dead zone still cancels; a short press still toggles the phone.
Bindings and timing are captured at button press so a preference update cannot
change an active gesture. The phone-hand quick action now saves the preference
through the same path as the phone button. Loading these settings never enables
microphone capture. Tests use synthetic button events; XR usability remains open.

## Rendering limits and acceptance

The default Mobile renderer exposes bilinear scaling and MSAA. FSR 1 is offered
conservatively only on desktop Forward+, outside XR and at scales no larger than
1; otherwise the official client reports a bilinear fallback. Unsupported
temporal/stereo upscalers are not advertised as working.

Texture LOD bias changes sampling detail, **not texture allocation or VRAM
usage**. The separate texture-dimension setting resamples PNG/JPEG/WebP images
inside the GLB/VRM importer before GPU texture creation. It applies to worlds,
their packaged prefabs and avatars, across all material tiers, on the next
package load (rejoin/reload to update already loaded content). Each dimension is
divided by 2/4/8, floored and clamped to at least one pixel. Source ZIPs are not
rewritten, and no original-size texture is retained for material-tier switching.
The original image must still be decoded transiently on the CPU; this is not a
reduction in download size or a complete peak-memory budget. Headless tests check
actual texture dimensions, not physical VRAM usage. The complete advanced
material schema, GPU/renderer quality comparisons and XR AA comfort remain unfinished.
These controls never alter server physics, interactions or reliable delivery,
and do not promise a minimum frame rate for arbitrarily complex assets.

Headless tests cover settings controls, conservative upscaler selection,
missing-device mute/fallback, gain values and audio-bus cleanup. Server tests
cover defaults, persistence and rejection of invalid updates. These do not
validate physical microphone/output devices, audible gain quality, visual AA
quality or headset comfort.
