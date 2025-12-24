# Canonical home-manager module for local-logger
#
# This is the single cross-platform module for local-logger configuration.
# It handles all user-specific setup for both Darwin (macOS) and Linux:
#
# - Package installation
# - CA certificate generation (activation script)
# - Claude Code MCP/hooks configuration (activation script)
# - Proxy environment variables (NODE_EXTRA_CA_CERTS, HTTPS_PROXY)
# - Per-user launchd agent (Darwin) or systemd user service (Linux)
#
# Usage in home-manager:
#   imports = [ local-logger.homeManagerModules.default ];
#   services.local-logger.enable = true;
#
{
  config,
  lib,
  pkgs,
  ...
}:

with lib;

let
  cfg = config.services.local-logger;
  logDir = "${config.home.homeDirectory}/.local-logger";
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

      address = mkOption {
        type = types.str;
        default = "127.0.0.1";
        description = "Address for the proxy to bind to";
      };
    };

    generateCACert = mkOption {
      type = types.bool;
      default = true;
      description = "Whether to auto-generate CA certificate on activation";
    };

    configureClaudeMcp = mkOption {
      type = types.bool;
      default = true;
      description = "Whether to configure Claude Code MCP server and hooks";
    };

    injectProxyEnv = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Whether to set proxy environment variables in the user session.
        Sets NODE_EXTRA_CA_CERTS, HTTPS_PROXY, and HTTP_PROXY.
      '';
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

    # Proxy environment variables (replaces darwin wrapper script)
    (mkIf (cfg.proxy.enable && cfg.injectProxyEnv) {
      home.sessionVariables = {
        # Tell Node.js to trust the local-logger CA certificate
        NODE_EXTRA_CA_CERTS = "${logDir}/certs/ca.pem";
        # Route traffic through the proxy
        HTTPS_PROXY = "http://${cfg.proxy.address}:${toString cfg.proxy.port}";
        HTTP_PROXY = "http://${cfg.proxy.address}:${toString cfg.proxy.port}";
      };
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
            "--address"
            cfg.proxy.address
          ];
          RunAtLoad = true;
          KeepAlive = true;
          StandardOutPath = "${logDir}/proxy.log";
          StandardErrorPath = "${logDir}/proxy.err";
          EnvironmentVariables = {
            HOME = config.home.homeDirectory;
            CLAUDE_MCP_LOCAL_LOGGER_DIR = logDir;
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
          ExecStart = "${cfg.package}/bin/local-logger proxy --port ${toString cfg.proxy.port} --address ${cfg.proxy.address}";
          Restart = "on-failure";
          RestartSec = "5s";
          Environment = [
            "HOME=${config.home.homeDirectory}"
            "CLAUDE_MCP_LOCAL_LOGGER_DIR=${logDir}"
          ];
        };
        Install = {
          WantedBy = [ "default.target" ];
        };
      };
    })
  ]);
}
