# Darwin launchd service for local-logger proxy
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
    # System-level per-user launchd agent for local-logger proxy
    launchd.user.agents.local-logger-proxy = {
      serviceConfig = {
        Label = "org.nix-community.local-logger-proxy";
        ProgramArguments = [
          "${cfg.package}/bin/local-logger"
          "proxy"
          "--port"
          (toString cfg.proxy.port)
        ];
        RunAtLoad = true;
        KeepAlive = true;
        StandardOutPath = "/tmp/local-logger-proxy.log";
        StandardErrorPath = "/tmp/local-logger-proxy.err";
      };
    };
  };
}