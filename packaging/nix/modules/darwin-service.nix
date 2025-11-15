# Darwin launchd service for local-logger proxy
{
  config,
  lib,
  pkgs,
  machineConfig ? { },
  ...
}:

with lib;

let
  cfg = config.services.local-logger;
in
{
  # Import base module
  imports = [ ./local-logger.nix ];

  config = mkIf cfg.enable (
    let
      # Wrapper script for claude that sets proxy environment variables
      # This allows users to type 'claude' and have the proxy automatically configured
      claudeWrapper = pkgs.writeShellScriptBin "claude" ''
        export NODE_EXTRA_CA_CERTS="$HOME/.local-logger/certs/ca.pem"
        export HTTPS_PROXY="http://127.0.0.1:${toString cfg.proxy.port}"
        export HTTP_PROXY="http://127.0.0.1:${toString cfg.proxy.port}"

        # Find the real claude binary from npm (not this wrapper)
        npm_bin=$(npm config get prefix --global 2>/dev/null || echo "$HOME/.npm-global")
        exec "$npm_bin/bin/claude" "$@"
      '';
    in
    mkMerge [
      # Install packages and configure Claude MCP/hooks
      {
        # Install claude wrapper (jq no longer needed - using Rust built-in commands)
        environment.systemPackages = with pkgs; [
          claudeWrapper
        ];
      }

      # Proxy service configuration (only when proxy is enabled)
      (mkIf cfg.proxy.enable {
        # System-level per-user launchd agent for local-logger proxy
        #
        # CURRENT LIMITATION: This only works for the primary user (system.primaryUser)
        # because nix-darwin's launchd.user.agents creates a single agent tied to the
        # primary user context, not separate agents for each system user.
        #
        # MIGRATION PATH FOR MULTI-USER SUPPORT:
        #   Option 1 (Recommended): Migrate to home-manager
        #     - home-manager properly supports per-user launchd agents
        #     - Each user gets their own service instance with proper environment
        #     - Example: launchd.agents.local-logger-proxy in each user's home-manager config
        #
        #   Option 2: Generate dynamic launchd agents
        #     - Loop through machineConfig.users and create separate launchd.user.agents
        #     - Each with user-specific HOME and CLAUDE_MCP_LOCAL_LOGGER_DIR
        #     - More complex, requires careful handling of user contexts
        #
        # NOTE: The activation script below handles CA setup for multiple users,
        # but the proxy service itself only runs for the primary user.
        launchd.user.agents.local-logger-proxy = {
      serviceConfig = {
        Label = "org.cogent-creation-co.local-logger-proxy";
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

        # Environment variables required for the proxy to find home directory
        # and log files. These are set for the primary user only.
        EnvironmentVariables = {
          HOME = config.users.users.${config.system.primaryUser}.home;
          CLAUDE_MCP_LOCAL_LOGGER_DIR = "${config.users.users.${config.system.primaryUser}.home}/.local-logger";
        };
      };
    };

    # Activation script to set up CA certificates for configured users
    system.activationScripts.local-logger-ca =
      mkIf (machineConfig ? users && builtins.length machineConfig.users > 0)
        {
          text = ''
            echo "Setting up local-logger CA certificates..."

            # Check for conflicting PKG installation
            if [ -f "/usr/local/bin/local-logger" ] && [ ! -L "/usr/local/bin/local-logger" ]; then
              echo "ERROR: local-logger PKG installation detected" >&2
              echo "" >&2
              echo "Found: /usr/local/bin/local-logger" >&2
              echo "" >&2
              echo "To use Nix-managed local-logger, first uninstall the PKG version:" >&2
              echo "  sudo /usr/local/bin/local-logger-uninstall.sh" >&2
              echo "" >&2
              exit 1
            fi

            # Function to set up CA for a single user
            setup_user_ca() {
              local username="$1"
              local user_home="$2"

              # Validate inputs
              if [ -z "$username" ] || [ -z "$user_home" ]; then
                echo "  ERROR: Invalid user configuration (username='$username', home='$user_home')"
                return 1
              fi

              # Check if home directory exists
              if [ ! -d "$user_home" ]; then
                echo "  WARNING: Home directory does not exist for user '$username': $user_home"
                return 1
              fi

              echo "  Processing user: $username"

              # Define paths
              local logger_dir="$user_home/.local-logger"
              local certs_dir="$logger_dir/certs"
              local ca_cert="$certs_dir/ca.pem"
              local ca_key="$certs_dir/ca.key"

              # Create directories with correct ownership
              if [ ! -d "$certs_dir" ]; then
                echo "    Creating certificate directory..."
                ${pkgs.coreutils}/bin/install -d -m 755 -o "$username" -g staff "$logger_dir" || {
                  echo "    ERROR: Failed to create $logger_dir"
                  return 1
                }
                ${pkgs.coreutils}/bin/install -d -m 755 -o "$username" -g staff "$certs_dir" || {
                  echo "    ERROR: Failed to create $certs_dir"
                  return 1
                }
              fi

              # Generate CA certificate if it doesn't exist
              if [ ! -f "$ca_cert" ] || [ ! -f "$ca_key" ]; then
                echo "    Generating CA certificate..."

                # Use the new init command to generate certificates
                if sudo -u "$username" ${cfg.package}/bin/local-logger init --quiet; then
                  echo "    CA certificate generated successfully"
                else
                  echo "    ERROR: Failed to generate CA certificate" >&2
                  return 1
                fi

                # Verify certificates were created
                if [ ! -f "$ca_cert" ]; then
                  echo "    ERROR: CA certificate not found after initialization"
                  return 1
                fi
              else
                echo "    CA certificate already exists"
              fi

              # Verify and fix ownership
              if [ -f "$ca_cert" ]; then
                ${pkgs.coreutils}/bin/chown "$username:staff" "$ca_cert" 2>/dev/null || true
                ${pkgs.coreutils}/bin/chmod 644 "$ca_cert"
              fi

              if [ -f "$ca_key" ]; then
                ${pkgs.coreutils}/bin/chown "$username:staff" "$ca_key" 2>/dev/null || true
                ${pkgs.coreutils}/bin/chmod 600 "$ca_key"
              fi

              # Trust the CA certificate in system keychain (idempotent check)
              if ${pkgs.darwin.security_tool}/bin/security verify-cert -c "$ca_cert" >/dev/null 2>&1; then
                echo "    CA certificate already trusted in system keychain"
              else
                echo "    Installing CA certificate to system keychain..."
                if ${pkgs.darwin.security_tool}/bin/security add-trusted-cert \
                  -d -r trustRoot \
                  -k /Library/Keychains/System.keychain \
                  "$ca_cert"; then
                  echo "    Successfully trusted CA certificate"
                else
                  echo "    WARNING: Failed to trust CA certificate (this may require manual intervention)" >&2
                  echo "    Run: sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain $ca_cert" >&2
                fi
              fi

              echo "    Setup complete for user: $username"
              return 0
            }

            # Process each configured user
            ${concatMapStringsSep "\n" (username: ''
              # Get user home directory from nix config
              user_home="${(config.users.users.${username} or { }).home or ""}"
              if [ -n "$user_home" ]; then
                setup_user_ca "${username}" "$user_home" || echo "  Failed to set up CA for ${username}, continuing..."
              else
                echo "  WARNING: User '${username}' not found in configuration, skipping..."
              fi
            '') machineConfig.users}

            echo "local-logger CA setup complete"
          '';
        };
      })

      # Claude MCP server and hooks configuration
      # This configures Claude Code to use local-logger as an MCP server and for hooks
      # NOTE: Activation script runs whether module is enabled or disabled to handle cleanup
      (mkIf (machineConfig ? users && builtins.length machineConfig.users > 0) {
        system.activationScripts.local-logger-claude-config = {
          text = ''
            ${if cfg.enable then ''
              # ============================================================
              # CONFIGURE: Add local-logger to Claude Code configuration
              # ============================================================
              echo "Configuring Claude Code for local-logger..."

              # Function to configure Claude for a single user
              configure_claude() {
                local username="$1"
                local user_home="$2"

                # Validate inputs
                if [ -z "$username" ] || [ -z "$user_home" ]; then
                  echo "  ERROR: Invalid user configuration (username='$username', home='$user_home')"
                  return 1
                fi

                # Check if home directory exists
                if [ ! -d "$user_home" ]; then
                  echo "  WARNING: Home directory does not exist for user '$username': $user_home"
                  return 1
                fi

                echo "  Configuring Claude for user: $username"

                # Use local-logger's built-in configuration installer
                if sudo -u "$username" ${cfg.package}/bin/local-logger install-claude --quiet; then
                  echo "  ✓ Claude configuration complete for user: $username"
                else
                  echo "  ERROR: Failed to configure Claude for user: $username"
                  return 1
                fi
              }

              # Process each configured user
              ${concatMapStringsSep "\n" (username: ''
                # Get user home directory from nix config
                user_home="${(config.users.users.${username} or { }).home or ""}"
                if [ -n "$user_home" ]; then
                  configure_claude "${username}" "$user_home" || echo "  Failed to configure Claude for ${username}, continuing..."
                else
                  echo "  WARNING: User '${username}' not found in configuration, skipping..."
                fi
              '') machineConfig.users}

              echo "Claude Code configuration complete"
            '' else ''
              # ============================================================
              # CLEANUP: Remove local-logger from Claude Code configuration
              # ============================================================
              echo "Removing local-logger from Claude Code configuration..."

              # Function to cleanup Claude config for a single user
              cleanup_claude() {
                local username="$1"
                local user_home="$2"

                # Validate inputs
                if [ -z "$username" ] || [ -z "$user_home" ]; then
                  echo "  ERROR: Invalid user configuration (username='$username', home='$user_home')"
                  return 1
                fi

                # Check if home directory exists
                if [ ! -d "$user_home" ]; then
                  echo "  WARNING: Home directory does not exist for user '$username': $user_home"
                  return 1
                fi

                echo "  Cleaning up Claude config for user: $username"

                # Use local-logger's built-in configuration uninstaller
                if sudo -u "$username" ${cfg.package}/bin/local-logger uninstall-claude --quiet; then
                  echo "  ✓ Cleanup complete for user: $username"
                else
                  echo "  ERROR: Failed to cleanup Claude for user: $username"
                  return 1
                fi
              }

              # Process each configured user
              ${concatMapStringsSep "\n" (username: ''
                # Get user home directory from nix config
                user_home="${(config.users.users.${username} or { }).home or ""}"
                if [ -n "$user_home" ]; then
                  cleanup_claude "${username}" "$user_home" || echo "  Failed to cleanup Claude for ${username}, continuing..."
                else
                  echo "  WARNING: User '${username}' not found in configuration, skipping..."
                fi
              '') machineConfig.users}

              echo "Claude Code cleanup complete"
            ''}
          '';
        };
      })
    ]
  );
}
