# Super Simple Chat in VR

An in-development private social VR app: Rust/Axum/SQLite server, Godot 4.7.2
GDScript client, sandboxed Luau, and a vanilla JavaScript website.
The repository specification remains the full release target, not a list of
features already finished.

Project code follows the repository's AGPL-3.0 license. Pinned third-party
integrations retain their own licenses and notices.

## Local development

```sh
nix develop           # or: nix-shell
export SSC_ADMIN_TOKEN="$(head -c 32 /dev/urandom | basenc --base16)"
cargo run --bin super-simple-chat-server
```

Open `http://127.0.0.1:8080`, unlock administrator access with that secret, and
upload an example world generated in another development shell:

```sh
cargo run --example make-example -- /tmp/opencode/quiet-room.zip
cargo run --example make-example -- /tmp/opencode/tank-yard.zip --tanks
cargo run --example make-example -- /tmp/opencode/reference-avatar.zip --avatar
```

The generator does not overwrite existing files. Both tooling and integration
tests use Rust and GDScript; there is no additional tooling language to install.

### Without a headset

```sh
nix run .#client -- --xr-mode off
# Or inside the development shell, with pinned add-ons and native libraries:
godot4 --path "$SSC_TEST_CLIENT" --xr-mode off
```

Create a connection grant on the website and paste its JSON into the desktop
phone panel. Use WASD to move when a text field is not focused. The phone panel
supports text chat, nearby-object interaction, vehicle control, avatar slots,
touch settings, proximity voice, and private calls. Right-drag to look; arrow
keys drive a claimed vehicle and Space fires. Microphone capture is muted by
default. The client has a desktop
fallback if no XR runtime is available; explicitly disabling XR avoids runtime
discovery errors during headset-free testing.

### With a headset later

```sh
nix run .#client
```

First configure your system's OpenXR runtime and graphics drivers. Headset,
controller, tracker, and comfort acceptance checks still require real hardware;
headless tests do not establish those requirements as working.

## Builds and tests

```sh
nix build .#server .#client
nix flake check
nix develop --command cargo test --locked
nix develop --command cargo test --locked --workspace --all-targets
nix develop --command cargo test --locked --test enet -- --nocapture
```

The integration test launches the actual Rust backend and multiple headless
Godot clients, using temporary account/session fixtures, not a production login
bypass. It exercises authenticated encrypted packets, shared switch state and
Luau variables, text, independent full/quarter pose rates, malformed packets,
late join, reconnect, and empty-instance destruction. It also checks procedural
VRM expressions/springs/limb IK, synthetic body-tracker inputs and fallback,
client Luau watchdog/capabilities, real Opus encoding/decoding, private-call
bystander exclusion, bilateral touch/grab rules, control leases, and Rapier
projectile damage/scores. No headset is needed.

The flake pins toolchain and engine versions; Cargo.lock pins Rust dependencies.
Nix uses binary substitutes when available. Development builds disable debug
symbols and incremental compilation to reduce disk writes. The engine, codecs,
and other system libraries use Nix substitutes when available. One isolated
native extension supplies facilities missing from Godot: bounded Luau, Opus,
and the in-progress libmpv bridge. Ordinary gameplay remains GDScript. The
`nix develop .#native` shell permits editing/checking that extension without
rebuilding its release package just to enter the shell.

Production video decoding runs in a separate `ssc-media-worker`, built alongside
the extension and selected automatically by the packaged launcher. Its filesystem
and signal sandbox requires Linux 6.12+ with Landlock enabled; media fails closed
if that boundary is unavailable. See [media isolation](docs/media-security.md)
for resource limits and remaining security boundaries. In the lightweight edit
shell, build both artifacts with `cargo build -p super-simple-chat-native` before
testing video.

## NixOS integration

Import `inputs.super-simple-chat.nixosModules.server` and configure:

```nix
services.super-simple-chat = {
  enable = true;
  environmentFile = "/run/secrets/super-simple-chat.env";
  publicUrl = "https://vr.example.org";
  # Default API and ENet listeners are loopback-only.
};
```

The environment file must be outside the Nix store and contain:

```text
SSC_ADMIN_TOKEN=<random secret of at least 32 characters>
SSC_DISCORD_ID=<Discord OAuth application ID>
SSC_DISCORD_SECRET=<Discord OAuth application secret>
```

Register `https://vr.example.org/api/v1/auth/callback` with Discord. Put the HTTP
listener behind an HTTPS reverse proxy. Configure ENet reachability and firewall
policy explicitly; HTTP proxies do not forward UDP. Runtime data defaults to
`/var/lib/super-simple-chat`, managed by systemd. For direct local runs use
`SSC_DATA`, `SSC_LISTEN`, `SSC_ENET`, `SSC_PUBLIC_URL`, `SSC_TPS`, and
`SSC_VOICE_BITRATE` (8,000–64,000 bit/s, default 24,000) as needed.
`SSC_AUDIO_EFFECT_MAX` bounds world-authored effect amounts (0–1, default 0.9).

**Development build, not the complete release.** See [release status](docs/status.md)
for the remaining requirements and verification gaps. In particular radios,
synchronized video, full audio/creator/material/settings capabilities, live
Discord validation, real XR hardware checks, and long-session testing are not
complete. Passing the current suite does not mean all acceptance criteria pass.
