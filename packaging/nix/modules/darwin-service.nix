# Darwin system module for local-logger
# This module handles system-level configuration only.
# User-specific configuration (CA certs, Claude config, launchd agents)
# should use homeManagerModules.local-logger instead.
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

  config = mkIf cfg.enable (mkMerge [
    # System-level: Claude wrapper script
    # This wrapper sets proxy environment variables so users can simply run 'claude'
    # and have the proxy automatically configured. Uses $HOME so it works for any user.
    {
      environment.systemPackages = [
        (pkgs.writeShellScriptBin "claude" ''
          export NODE_EXTRA_CA_CERTS="$HOME/.local-logger/certs/ca.pem"
          export HTTPS_PROXY="http://127.0.0.1:${toString cfg.proxy.port}"
          export HTTP_PROXY="http://127.0.0.1:${toString cfg.proxy.port}"

          # Find the real claude binary from npm (not this wrapper)
          npm_bin=$(npm config get prefix --global 2>/dev/null || echo "$HOME/.npm-global")
          exec "$npm_bin/bin/claude" "$@"
        '')
      ];
    }

    # Optional: System-level CA trust activation script
    # This trusts any existing CA certificates found in user home directories.
    # It's user-agnostic - it doesn't require machineConfig.users.
    (mkIf cfg.proxy.enable {
      system.activationScripts.local-logger-trust-ca = {
        text = ''
          echo "Checking for local-logger CA certificates to trust..."

          # Check for conflicting PKG installation
          if [ -f "/usr/local/bin/local-logger" ] && [ ! -L "/usr/local/bin/local-logger" ]; then
            echo "WARNING: local-logger PKG installation detected at /usr/local/bin/local-logger"
            echo "Consider uninstalling it: sudo /usr/local/bin/local-logger-uninstall.sh"
          fi

          # Trust CA certificates that exist in user home directories
          # This is user-agnostic - it finds certificates dynamically
          for user_home in /Users/*; do
            [ -d "$user_home" ] || continue
            username=$(basename "$user_home")

            # Skip system directories
            case "$username" in
              Shared|.localized) continue ;;
            esac

            ca_cert="$user_home/.local-logger/certs/ca.pem"
            if [ -f "$ca_cert" ]; then
              # Check if already trusted
              if ${pkgs.darwin.security_tool}/bin/security verify-cert -c "$ca_cert" >/dev/null 2>&1; then
                echo "  CA certificate already trusted for $username"
              else
                echo "  Trusting CA certificate for $username..."
                if ${pkgs.darwin.security_tool}/bin/security add-trusted-cert \
                  -d -r trustRoot \
                  -k /Library/Keychains/System.keychain \
                  "$ca_cert" 2>/dev/null; then
                  echo "  Successfully trusted CA certificate for $username"
                else
                  echo "  Note: Could not auto-trust CA for $username. Run manually:"
                  echo "    sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain $ca_cert"
                fi
              fi
            fi
          done

          echo "local-logger CA trust check complete"
        '';
      };
    })
  ]);
}
