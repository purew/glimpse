{
  description = "glimpse-rs — personal photo-blog server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, rust-overlay }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };

      # Toolchain read directly from rust-toolchain.toml — version and components stay in sync
      toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

      craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

      # Native build tools needed at compile time
      nativeBuildInputs = [ pkgs.pkg-config ];

      # Libraries linked against at build / runtime
      buildInputs = [ ];

      # Runtime tools expected on PATH
      runtimeDeps = [ pkgs.ffmpeg ];

      # cleanCargoSource strips non-Rust files; extend the filter to keep themes
      src = pkgs.lib.cleanSourceWith {
        src = craneLib.path ./.;
        filter = path: type:
          (builtins.match ".*/themes/.*" path != null)
          || (craneLib.filterCargoSources path type);
      };

      commonArgs = {
        inherit src nativeBuildInputs buildInputs;
        pname = "glimpse-rs";
        version = "0.1.0";
        strictDeps = true;
      };

      # Dependency crate compilation cached separately from app code
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

      glimpse = craneLib.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        nativeBuildInputs = nativeBuildInputs ++ [ pkgs.makeWrapper ];
        postInstall = ''
          mkdir -p $out/share/glimpse-rs
          cp -r --no-preserve=mode ${src}/themes $out/share/glimpse-rs/
          wrapProgram $out/bin/glimpse-rs \
            --set GLIMPSE_THEME_DIR $out/share/glimpse-rs/themes/default
        '';
      });
    in
    {
      packages.${system} = {
        inherit glimpse;
        default = glimpse;
      };

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = [ pkgs.pkg-config toolchain ];

        buildInputs = buildInputs ++ runtimeDeps ++ [
          pkgs.cargo-watch  # optional: cargo watch for development
        ];
      };
    };
}
