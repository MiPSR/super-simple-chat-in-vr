self: { config, lib, pkgs, ... }:
let cfg = config.services.super-simple-chat;
in {
  options.services.super-simple-chat = {
    enable = lib.mkEnableOption "Super Simple Chat in VR backend";
    package = lib.mkOption { type = lib.types.package; default = self.packages.${pkgs.stdenv.hostPlatform.system}.server; };
    environmentFile = lib.mkOption { type = lib.types.path; description = "Root-readable secrets file, outside the Nix store."; };
    listenAddress = lib.mkOption { type = lib.types.str; default = "127.0.0.1:8080"; };
    enetAddress = lib.mkOption { type = lib.types.str; default = "127.0.0.1:7777"; };
    publicUrl = lib.mkOption { type = lib.types.str; default = "https://vr.example.org"; };
    stateDirectory = lib.mkOption { type = lib.types.str; default = "super-simple-chat"; };
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
      };
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/super-simple-chat-server";
        EnvironmentFile = cfg.environmentFile;
        StateDirectory = cfg.stateDirectory;
        DynamicUser = true;
        Restart = "on-failure";
        UMask = "0077";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
      };
    };
  };
}
