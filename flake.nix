{
  description = "A sleek, customizable desktop shell crafted for Wayland.";

  inputs = {
    nixpkgs.url = "https://channels.nixos.org/nixos-unstable/nixexprs.tar.xz";
    # Pins the Rust toolchain for the migration dev shell (see nix/rust-devshell.nix).
    # rust-overlay over fenix: the toolchain pin lives in flake.lock alongside nixpkgs,
    # it needs no extra binary cache, and its stable-channel attrset maps directly to
    # upstream releases.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, nixpkgs, rust-overlay }:
    let
      inherit (nixpkgs.lib) genAttrs getExe;

      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forEachSystem =
        perSystem:
        genAttrs systems (
          system:
          let
            pkgs = nixpkgs.legacyPackages.${system};
          in
          perSystem { inherit pkgs system; }
        );
    in
    {
      overlays.default = final: prev: {
        noctalia = final.callPackage ./nix/package.nix { };
      };

      packages = forEachSystem (
        { pkgs, ... }:
        {
          default = pkgs.callPackage ./nix/package.nix { };
          cuda = pkgs.callPackage ./nix/package.nix { cudaSupport = true; };
        }
      );

      devShells = forEachSystem (
        { pkgs, system }:
        {
          default = pkgs.callPackage ./nix/devshell.nix {
            noctalia = self.packages.${system}.default;
          };
          # Rust migration shell — `nix develop .#rust` (auto-loaded by .envrc).
          rust = import ./nix/rust-devshell.nix {
            inherit pkgs;
            rust-bin = rust-overlay.lib.mkRustBin { } pkgs;
          };
        }
      );

      apps = forEachSystem (
        { system, ... }:
        {
          default = {
            type = "app";
            program = getExe self.packages.${system}.default;
          };
        }
      );

      homeModules.default =
        { pkgs, lib, ... }:
        {
          imports = [ ./nix/home-module.nix ];
          programs.noctalia.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };

      hjemModules.default =
        { pkgs, lib, ... }:
        {
          imports = [ ./nix/hjem-module.nix ];
          programs.noctalia.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };

      nixosModules.default =
        { pkgs, lib, ... }:
        {
          imports = [ ./nix/nixos-module.nix ];
          programs.noctalia.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
    };
}
