{
  description = "Super Simple Chat in VR — NixOS-first private social VR";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.vrm = {
    url = "github:V-Sekai/godot-vrm/e15199f980064028bfa4fbee5e70dddb82dd55c3";
    flake = false;
  };
  outputs = { self, nixpkgs, vrm }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs { inherit system; };
    source = pkgs.lib.fileset.toSource {
      root = ./.;
      fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./server ./web ./client ./native ./shared ];
    };
    server = pkgs.rustPlatform.buildRustPackage {
      pname = "super-simple-chat-server";
      version = "0.1.0";
      src = source;
      cargoLock.lockFile = ./Cargo.lock;
      cargoBuildFlags = [ "-p" "super-simple-chat-server" ];
      cargoTestFlags = [ "-p" "super-simple-chat-server" ];
      nativeBuildInputs = [ pkgs.pkg-config ];
      buildInputs = [ pkgs.sqlite pkgs.libopus pkgs.opusfile ];
      nativeCheckInputs = [ pkgs.godot_4 pkgs.ffmpeg pkgs.nodejs ];
      SSC_TEST_CLIENT = clientSource;
      doCheck = true;
      preCheck = ''
        export HOME="$(mktemp -d)"
        export XDG_CACHE_HOME="$HOME/.cache"
        export XDG_DATA_HOME="$HOME/.local/share"
        export XDG_CONFIG_HOME="$HOME/.config"
        mkdir -p "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME"
      '';
    };
    native = pkgs.rustPlatform.buildRustPackage {
      pname = "super-simple-chat-native";
      version = "0.1.0";
      src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./native ./server/Cargo.toml ./server/src/main.rs ./shared ];
      };
      cargoLock.lockFile = ./Cargo.lock;
      cargoBuildFlags = [ "-p" "super-simple-chat-native" ];
      cargoTestFlags = [ "-p" "super-simple-chat-native" ];
      nativeBuildInputs = [ pkgs.pkg-config ];
      nativeCheckInputs = [ pkgs.ffmpeg ];
      buildInputs = [ pkgs.libopus pkgs.opusfile pkgs.mpv-unwrapped ];
      installPhase = ''
        runHook preInstall
        mkdir -p "$out/lib"
        cp target/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/release/libsuper_simple_chat_native.so "$out/lib/"
        runHook postInstall
      '';
    };
    ytResolver = pkgs.writeShellScript "ssc-yt-dlp" ''
      unset no_proxy NO_PROXY
      exec ${pkgs.coreutils}/bin/timeout --kill-after=5s 30s ${pkgs.yt-dlp}/bin/yt-dlp "$@"
    '';
    clientSource = pkgs.runCommand "super-simple-chat-client-project" {} ''
      mkdir -p "$out/addons"
      cp -R ${./client}/. "$out/"
      cp -R ${vrm}/addons/vrm ${vrm}/addons/Godot-MToon-Shader "$out/addons/"
      mkdir -p "$out/lib" "$out/.godot"
      cp ${native}/lib/libsuper_simple_chat_native.so "$out/lib/"
      echo 'res://ssc.gdextension' > "$out/.godot/extension_list.cfg"
    '';
    client = pkgs.writeShellApplication {
      name = "super-simple-chat-client";
      runtimeInputs = [ pkgs.godot_4 ];
      text = ''
        export SSC_YTDLP=${ytResolver}
        unset no_proxy NO_PROXY SSC_TEST_MEDIA_ORIGIN
        # Import data belongs in a writable cache, never in the Nix store.
        cache="''${XDG_CACHE_HOME:-$HOME/.cache}/super-simple-chat/${builtins.substring 0 16 (builtins.hashString "sha256" (toString clientSource))}"
        mkdir -p "$cache"
        cp -ru --no-preserve=mode ${clientSource}/. "$cache/"
        if [ ! -d "$cache/.godot" ]; then
          godot4 --headless --path "$cache" --editor --import --xr-mode off >/dev/null 2>&1
        fi
        exec godot4 --path "$cache" "$@"
      '';
    };
  in {
    packages.${system} = { inherit server client native; client-project = clientSource; default = server; };
    apps.${system} = {
      server = { type = "app"; program = "${server}/bin/super-simple-chat-server"; };
      client = { type = "app"; program = "${client}/bin/super-simple-chat-client"; };
    };
    devShells.${system} = {
    default = pkgs.mkShell {
      SSC_TEST_CLIENT = clientSource;
      SSC_YTDLP = "${ytResolver}";
      buildInputs = [ pkgs.libopus pkgs.opusfile pkgs.mpv-unwrapped pkgs.sqlite ];
      packages = with pkgs; [ cargo rustc rustfmt clippy pkg-config sqlite godot_4 ffmpeg nixfmt nodejs ];
      shellHook = ''
        export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
        echo "Super Simple Chat in VR: cargo run | godot4 --path client"
      '';
    };
    # Small edit/check shell: does not rebuild the packaged extension on entry.
    native = pkgs.mkShell {
      SSC_VRM_SOURCE = vrm;
      SSC_YTDLP = "${ytResolver}";
      packages = with pkgs; [ cargo rustc rustfmt clippy pkg-config godot_4 ffmpeg nodejs ];
      buildInputs = [ pkgs.libopus pkgs.opusfile pkgs.mpv-unwrapped pkgs.sqlite ];
      shellHook = ''export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0'';
    };
    };
    checks.${system} = {
      inherit server;
      # The Rust test suite includes real headless Godot ENet interoperability.
      integration = server;
    };
    nixosModules.server = import ./nix/server-module.nix self;
  };
}
