# NixOS systemd service for local-logger proxy
{
  config,
  lib,
  pkgs,
  ...
}:

with lib;

let
  cfg = config.services.local-logger;
in
{
  # Import base module
  imports = [ ./local-logger.nix ];

  config = mkIf (cfg.enable && cfg.proxy.enable) {
    # System-level systemd user service for local-logger proxy
    systemd.user.services.local-logger-proxy = {
      description = "Local Logger Proxy for Claude Code";
      wantedBy = [ "default.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${cfg.package}/bin/local-logger proxy --port ${toString cfg.proxy.port}";
        Restart = "on-failure";
        RestartSec = "5s";

        # Security hardening
        PrivateTmp = true;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = "read-only";
        ReadWritePaths = [ "%h/.local-logger" ];
      };
    };
  };
}