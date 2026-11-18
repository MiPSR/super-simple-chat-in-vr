self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.super-simple-chat;
in
{
  options.services.super-simple-chat = {
    enable = lib.mkEnableOption "Super Simple Chat in VR backend";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.server;
    };
    environmentFile = lib.mkOption {
      type = lib.types.path;
      description = "Root-readable secrets file, outside the Nix store.";
    };
    listenAddress = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:8080";
    };
    enetAddress = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:7777";
    };
    publicUrl = lib.mkOption {
      type = lib.types.str;
      description = "Public HTTPS origin advertised to clients and registered with Discord; loopback http://127.0.0.1:… is allowed only for development.";
    };
    stateDirectory = lib.mkOption {
      type = lib.types.str;
      default = "super-simple-chat";
    };
    simulationTps = lib.mkOption {
      type = lib.types.ints.between 1 120;
      default = 60;
    };
    networkMinimumHz = lib.mkOption {
      type = lib.types.ints.between 1 120;
      default = 1;
    };
    networkMaximumHz = lib.mkOption {
      type = lib.types.ints.between 1 120;
      default = 120;
    };
    networkAdaptation = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };
    networkAggregation = lib.mkOption {
      type = lib.types.enum [
        "latest"
        "suppress-unchanged"
      ];
      default = "latest";
      description = "Per-peer disposable state aggregation; never changes simulation or reliable events.";
    };
    instanceLimit = lib.mkOption {
      type = lib.types.ints.between 1 512;
      default = 64;
      description = "Maximum live instances in one server process (SSC_INSTANCE_LIMIT).";
    };
    instancePlayers = lib.mkOption {
      type = lib.types.ints.between 1 128;
      default = 64;
      description = "Maximum players per instance (SSC_INSTANCE_PLAYERS).";
    };
    defaultWorldSlots = lib.mkOption {
      type = lib.types.ints.between 1 1000;
      default = 4;
      description = "World slots granted to newly allowed accounts (SSC_DEFAULT_WORLD_SLOTS).";
    };
    defaultAvatarSlots = lib.mkOption {
      type = lib.types.ints.between 1 1000;
      default = 8;
      description = "Avatar slots granted to newly allowed accounts (SSC_DEFAULT_AVATAR_SLOTS).";
    };
    defaultWorldBytes = lib.mkOption {
      type = lib.types.ints.between 0 1000000000;
      default = 1000000000;
      description = "World upload bytes granted to newly allowed accounts (SSC_DEFAULT_WORLD_BYTES).";
    };
    defaultAvatarBytes = lib.mkOption {
      type = lib.types.ints.between 0 1000000000;
      default = 100000000;
      description = "Avatar upload bytes granted to newly allowed accounts (SSC_DEFAULT_AVATAR_BYTES).";
    };
  };
  config = lib.mkIf cfg.enable {
    systemd.services.super-simple-chat = {
      description = "Super Simple Chat in VR";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      environment = {
        SSC_LISTEN = cfg.listenAddress;
        SSC_ENET = cfg.enetAddress;
        SSC_PUBLIC_URL = cfg.publicUrl;
        SSC_DATA = "/var/lib/${cfg.stateDirectory}";
        SSC_TPS = toString cfg.simulationTps;
        SSC_NETWORK_MIN_HZ = toString cfg.networkMinimumHz;
        SSC_NETWORK_MAX_HZ = toString cfg.networkMaximumHz;
        SSC_NETWORK_ADAPT = lib.boolToString cfg.networkAdaptation;
        SSC_NETWORK_AGGREGATION = cfg.networkAggregation;
        SSC_INSTANCE_LIMIT = toString cfg.instanceLimit;
        SSC_INSTANCE_PLAYERS = toString cfg.instancePlayers;
        SSC_DEFAULT_WORLD_SLOTS = toString cfg.defaultWorldSlots;
        SSC_DEFAULT_AVATAR_SLOTS = toString cfg.defaultAvatarSlots;
        SSC_DEFAULT_WORLD_BYTES = toString cfg.defaultWorldBytes;
        SSC_DEFAULT_AVATAR_BYTES = toString cfg.defaultAvatarBytes;
      };
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/super-simple-chat-server";
        EnvironmentFile = cfg.environmentFile;
        StateDirectory = cfg.stateDirectory;
        DynamicUser = true;
        Restart = "on-failure";
        UMask = "0077";
        LogRateLimitIntervalSec = "30s";
        LogRateLimitBurst = 1000;
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
      };
    };
  };
}
