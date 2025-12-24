{
  description = "Local Logger - MCP server, hook logger, and HTTPS proxy for Claude Code";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        # Package output
        packages.default = pkgs.callPackage ./packaging/nix/package.nix { inherit self; };
        packages.local-logger = self.packages.${system}.default;

        # Development shell
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc
            cargo
            rustfmt
            clippy
            pkg-config
          ];
        };

        # Formatter
        formatter = pkgs.nixfmt-rfc-style;

        # Checks
        checks = {
          build = self.packages.${system}.default;
          format = pkgs.runCommand "check-format" { } ''
            ${pkgs.nixfmt-rfc-style}/bin/nixfmt --check ${self}
            touch $out
          '';
        };
      }
    ) // {
      # Home-manager module (canonical cross-platform module)
      # Works on both Darwin (launchd) and Linux (systemd)
      homeManagerModules.default = import ./packaging/nix/modules/default.nix;
      homeManagerModules.local-logger = self.homeManagerModules.default;

      # Overlay for adding package to nixpkgs
      overlays.default = final: prev: {
        local-logger = final.callPackage ./packaging/nix/package.nix {
          inherit self;
        };
      };
      overlays.local-logger = self.overlays.default;
    };
}