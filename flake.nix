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

  outputs = { self, nixpkgs, crane, rust-overlay } @inputs:
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
            --set GLIMPSE_THEME_DIR $out/share/glimpse-rs/themes/default \
            --prefix PATH : ${pkgs.lib.makeBinPath runtimeDeps}
        '';
      });
    in
    {
      packages.${system} = {
        inherit glimpse;
        default = glimpse;
      };

      nixosModules.glimpse = { config, lib, pkgs, ... }:
        let
          cfg = config.services.glimpse;
          configFile = (pkgs.formats.toml { }).generate "glimpse.toml" {
            listen = cfg.listen;
            site_title = cfg.siteTitle;
            posts_dir = cfg.postsDir;
            cache_dir = cfg.cacheDir;
            video_max_height = cfg.videoMaxHeight;
            preprocess_concurrency = cfg.preprocessConcurrency;
          };
        in
        {
          options.services.glimpse = {
            enable = lib.mkEnableOption "Glimpse photo-blog server";

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.system}.default;
              defaultText = lib.literalExpression "glimpse-rs from this flake";
              description = "The glimpse-rs package to use.";
            };

            stateDir = lib.mkOption {
              type = lib.types.str;
              default = "/var/lib/glimpse";
              description = "Working directory for posts/, cache/, and users.toml.";
            };

            sessionSecretFile = lib.mkOption {
              type = lib.types.path;
              description = ''
                Path to a file exporting GLIMPSE_SESSION_SECRET as a 128-character
                hex string (64 bytes).  Generate with: openssl rand -hex 64
              '';
            };

            usersFile = lib.mkOption {
              type = lib.types.path;
              default = "${cfg.stateDir}/users.toml";
              defaultText = lib.literalExpression ''"''${config.services.glimpse.stateDir}/users.toml"'';
              description = "Path to users.toml.";
            };

            postsDir = lib.mkOption {
              type = lib.types.str;
              default = "${cfg.stateDir}/posts";
              defaultText = lib.literalExpression ''"''${config.services.glimpse.stateDir}/posts"'';
              description = "Directory containing post subdirectories.";
            };

            cacheDir = lib.mkOption {
              type = lib.types.str;
              default = "${cfg.stateDir}/cache";
              defaultText = lib.literalExpression ''"''${config.services.glimpse.stateDir}/cache"'';
              description = "Directory where generated image/video derivatives are cached.";
            };

            listen = lib.mkOption {
              type = lib.types.str;
              default = "127.0.0.1:3000";
              description = "Address and port the server binds to.";
            };

            siteTitle = lib.mkOption {
              type = lib.types.str;
              default = "Glimpse";
              description = "Title shown in the browser tab and page header.";
            };

            videoMaxHeight = lib.mkOption {
              type = lib.types.ints.positive;
              default = 1080;
              description = "Videos taller than this are skipped at load time.";
            };

            preprocessConcurrency = lib.mkOption {
              type = lib.types.ints.positive;
              default = 2;
              description = "Maximum number of image derivatives generated concurrently during a reload.";
            };

            logLevel = lib.mkOption {
              type = lib.types.str;
              default = "info";
              description = "Value passed to RUST_LOG.";
            };
          };

          config = lib.mkIf cfg.enable {
            systemd.services.glimpse = {
              description = "Glimpse photo-blog server";
              wantedBy = [ "multi-user.target" ];
              after = [ "network.target" ];
              serviceConfig = {
                ExecStart = "${cfg.package}/bin/glimpse-rs --config ${configFile} --users ${cfg.usersFile}";
                EnvironmentFile = cfg.sessionSecretFile;
                Environment = "RUST_LOG=${cfg.logLevel}";
                WorkingDirectory = cfg.stateDir;
                StateDirectory = "glimpse";
                Restart = "on-failure";
              };
            };
          };
        };

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = [ pkgs.pkg-config toolchain ];

        buildInputs = buildInputs ++ runtimeDeps ++ [
          pkgs.cargo-watch  # optional: cargo watch for development
          pkgs.exiftool
        ];
      };
    };
}
