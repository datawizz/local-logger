# Base cross-platform module for local-logger
# This module provides common configuration and package installation
# Platform-specific service configuration is in separate modules
{
  config,
  lib,
  pkgs,
  ...
}:

with lib;

{
  options.services.local-logger = {
    enable = mkEnableOption "Local Logger for Claude Code";

    package = mkOption {
      type = types.package;
      default = pkgs.local-logger;
      defaultText = literalExpression "pkgs.local-logger";
      description = "The local-logger package to use";
    };

    logDir = mkOption {
      type = types.str;
      default = "$HOME/.local-logger";
      description = "Directory for storing log files";
    };

    proxy = {
      enable = mkEnableOption "HTTPS proxy service" // { default = true; };

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
  };

  config = mkIf config.services.local-logger.enable {
    # Install the package
    environment.systemPackages = [ config.services.local-logger.package ];

    # Set environment variables for all users
    environment.variables = mkIf config.services.local-logger.proxy.enable {
      CLAUDE_MCP_LOCAL_LOGGER_DIR = config.services.local-logger.logDir;
    };
  };
}