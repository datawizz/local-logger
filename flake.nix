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
        packages.default = pkgs.callPackage ./nix/package.nix { inherit self; };
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
      # NixOS/Darwin modules (not system-specific)
      nixosModules.default = import ./nix/modules/nixos-service.nix;
      nixosModules.local-logger = self.nixosModules.default;

      darwinModules.default = import ./nix/modules/darwin-service.nix;
      darwinModules.local-logger = self.darwinModules.default;

      # Cross-platform base module
      nixModules.default = import ./nix/modules/local-logger.nix;
      nixModules.local-logger = self.nixModules.default;

      # Overlay for adding package to nixpkgs
      overlays.default = final: prev: {
        local-logger = final.callPackage ./nix/package.nix {
          inherit self;
        };
      };
      overlays.local-logger = self.overlays.default;
    };
}