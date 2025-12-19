# Home-manager module for per-user local-logger configuration
# This module handles all user-specific setup:
# - CA certificate generation
# - Claude Code MCP/hooks configuration
# - Per-user launchd agent (Darwin) or systemd user service (Linux)
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
  options.services.local-logger = {
    enable = mkEnableOption "Local Logger for Claude Code";

    package = mkOption {
      type = types.package;
      default = pkgs.local-logger;
      defaultText = literalExpression "pkgs.local-logger";
      description = "The local-logger package to use";
    };

    proxy = {
      enable = mkEnableOption "HTTPS proxy service" // {
        default = true;
      };

      port = mkOption {
        type = types.port;
        default = 6969;
        description = "Port for the HTTPS proxy to listen on";
      };
    };

    configureClaudeMcp = mkOption {
      type = types.bool;
      default = true;
      description = "Whether to configure Claude Code MCP server and hooks";
    };

    generateCACert = mkOption {
      type = types.bool;
      default = true;
      description = "Whether to auto-generate CA certificate on activation";
    };
  };

  config = mkIf cfg.enable (mkMerge [
    # Install the package for the user
    {
      home.packages = [ cfg.package ];
    }

    # CA certificate generation via activation script
    (mkIf cfg.generateCACert {
      home.activation.local-logger-init = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
        # Initialize local-logger (generates CA certificate if needed)
        $DRY_RUN_CMD ${cfg.package}/bin/local-logger init --quiet 2>/dev/null || true
      '';
    })

    # Claude MCP/hooks configuration via activation script
    (mkIf cfg.configureClaudeMcp {
      home.activation.local-logger-claude = lib.hm.dag.entryAfter [ "local-logger-init" ] ''
        # Configure Claude Code to use local-logger
        $DRY_RUN_CMD ${cfg.package}/bin/local-logger install-claude --quiet 2>/dev/null || true
      '';
    })

    # Darwin: launchd agent for proxy
    (mkIf (pkgs.stdenv.isDarwin && cfg.proxy.enable) {
      launchd.agents.local-logger-proxy = {
        enable = true;
        config = {
          Label = "org.cogent-creation-co.local-logger-proxy";
          ProgramArguments = [
            "${cfg.package}/bin/local-logger"
            "proxy"
            "--port"
            (toString cfg.proxy.port)
          ];
          RunAtLoad = true;
          KeepAlive = true;
          StandardOutPath = "${config.home.homeDirectory}/.local-logger/proxy.log";
          StandardErrorPath = "${config.home.homeDirectory}/.local-logger/proxy.err";
          EnvironmentVariables = {
            HOME = config.home.homeDirectory;
            CLAUDE_MCP_LOCAL_LOGGER_DIR = "${config.home.homeDirectory}/.local-logger";
          };
        };
      };
    })

    # Linux: systemd user service for proxy
    (mkIf (pkgs.stdenv.isLinux && cfg.proxy.enable) {
      systemd.user.services.local-logger-proxy = {
        Unit = {
          Description = "Local Logger Proxy for Claude Code";
          After = [ "network.target" ];
        };
        Service = {
          Type = "simple";
          ExecStart = "${cfg.package}/bin/local-logger proxy --port ${toString cfg.proxy.port}";
          Restart = "on-failure";
          RestartSec = "5s";
          Environment = [
            "HOME=${config.home.homeDirectory}"
            "CLAUDE_MCP_LOCAL_LOGGER_DIR=${config.home.homeDirectory}/.local-logger"
          ];
        };
        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    })
  ]);
}
