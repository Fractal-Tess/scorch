{
  description = "Scorch web search, scraping, mapping, and crawling";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      mkPackages =
        pkgs:
        let
          lib = pkgs.lib;
          version = "0.1.0";
          source = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.lock
              ./Cargo.toml
              ./LICENSE
              ./README.md
              ./crates
              ./vendor
            ];
          };
          rustyV8Hashes = {
            x86_64-linux = "sha256-omgf3lMBir0zZgGPEyYX3VmAAt948VbHvG0v9gi1ZWc=";
            aarch64-linux = "sha256-42jQy0HBecQ6mQ5OxKVeRN2XYvHTS+FWlqzEQz+KbJI=";
          };
          librustyV8 = pkgs.stdenv.mkDerivation {
            pname = "librusty-v8";
            version = "137.3.0";
            src = pkgs.fetchurl {
              url = "https://github.com/denoland/rusty_v8/releases/download/v137.3.0/librusty_v8_release_${pkgs.stdenv.hostPlatform.rust.rustcTarget}.a.gz";
              hash = rustyV8Hashes.${pkgs.stdenv.hostPlatform.system};
            };
            dontUnpack = true;
            installPhase = ''
              gzip -cd "$src" > "$out"
            '';
            meta = {
              sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
              platforms = builtins.attrNames rustyV8Hashes;
            };
          };
          unwrapped = pkgs.rustPlatform.buildRustPackage {
            pname = "scorch-unwrapped";
            inherit version;
            src = source;

            outputs = [
              "out"
              "server"
            ];
            cargoHash = "sha256-zZq0D9Myv9UWmtV6l1OQoI4UgZg9/iYh0xJt5uEn2Bg=";
            cargoBuildFlags = [
              "--workspace"
              "--bins"
            ];
            nativeBuildInputs = [
              pkgs.clang
              pkgs.cmake
              pkgs.git
              pkgs.perl
              pkgs.pkg-config
              pkgs.rustPlatform.bindgenHook
            ];
            buildInputs = [ pkgs.openssl ];
            env = {
              OPENSSL_NO_VENDOR = "1";
              RUSTY_V8_ARCHIVE = librustyV8;
            };

            # V8 isolate initialization is incompatible with the build sandbox's
            # resource restrictions. The workspace tests run outside this derivation.
            doCheck = false;
            installPhase = ''
              runHook preInstall
              releaseDir="target/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/release"
              install -Dm755 "$releaseDir/scorch" "$out/bin/scorch"
              install -Dm755 "$releaseDir/scorchd" "$server/bin/scorchd"
              runHook postInstall
            '';

            meta = {
              description = "Self-contained web search and extraction service";
              homepage = "https://github.com/Fractal-Tess/scorch";
              license = lib.licenses.mit;
              platforms = supportedSystems;
              sourceProvenance = with lib.sourceTypes; [
                fromSource
                binaryNativeCode
              ];
            };
          };
          scorch =
            pkgs.runCommand "scorch-${version}"
              {
                meta = unwrapped.meta // {
                  description = "HTTP-only Scorch command-line and MCP client";
                  mainProgram = "scorch";
                };
              }
              ''
                mkdir -p "$out/bin"
                ln -s ${unwrapped}/bin/scorch "$out/bin/scorch"
              '';
          scorchd =
            pkgs.runCommand "scorchd-${version}"
              {
                meta = unwrapped.meta // {
                  description = "Scorch HTTP API, metasearch, browser, and crawl service";
                  mainProgram = "scorchd";
                };
              }
              ''
                mkdir -p "$out/bin"
                ln -s ${unwrapped.server}/bin/scorchd "$out/bin/scorchd"
              '';
          scorchdWithChromium =
            pkgs.runCommand "scorchd-with-chromium-${version}"
              {
                nativeBuildInputs = [ pkgs.makeWrapper ];
                meta = scorchd.meta // {
                  description = "Scorch service with Chromium compatibility available";
                };
              }
              ''
                mkdir -p "$out/bin"
                makeWrapper ${unwrapped.server}/bin/scorchd "$out/bin/scorchd" \
                  --set-default SCORCH_BROWSER_PATH ${pkgs.chromium}/bin/chromium
              '';
          skill =
            pkgs.runCommand "scorch-agent-skill-${version}"
              {
                meta.description = "Agent Skill for using a local Scorch service";
              }
              ''
                mkdir -p "$out/share/agent-skills/scorch"
                cp ${./.agents/skills/scorch/SKILL.md} "$out/share/agent-skills/scorch/SKILL.md"
              '';
        in
        {
          inherit scorch scorchd skill;
          scorch-unwrapped = unwrapped;
          scorchd-unwrapped = unwrapped.server;
          scorchd-with-chromium = scorchdWithChromium;
          default = scorch;
        };
    in
    {
      packages = forAllSystems (system: mkPackages nixpkgs.legacyPackages.${system});

      apps = forAllSystems (system: {
        default = self.apps.${system}.scorch;
        scorch = {
          type = "app";
          program = "${self.packages.${system}.scorch}/bin/scorch";
          meta.description = "Run the Scorch HTTP client";
        };
        scorchd = {
          type = "app";
          program = "${self.packages.${system}.scorchd}/bin/scorchd";
          meta.description = "Run the Scorch API service";
        };
      });

      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          packages = self.packages.${system};
          moduleSystem = nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              self.nixosModules.default
              {
                system.stateVersion = "26.05";
                programs.scorch.enable = true;
                services.scorchd.enable = true;
              }
            ];
          };
        in
        {
          inherit (packages) scorch scorchd skill;
          module =
            assert nixpkgs.lib.hasInfix "--browser obscura"
              moduleSystem.config.systemd.services.scorchd.serviceConfig.ExecStart;
            assert nixpkgs.lib.hasInfix "--bind 127.0.0.1:3000"
              moduleSystem.config.systemd.services.scorchd.serviceConfig.ExecStart;
            pkgs.runCommand "scorch-module-check" { } ''
              touch "$out"
            '';
          version = pkgs.runCommand "scorch-version-check" { } ''
            test "$(${packages.scorch}/bin/scorch --version)" = 'scorch 0.1.0'
            test "$(${packages.scorchd}/bin/scorchd --version)" = 'scorchd 0.1.0'
            touch "$out"
          '';
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.scorch-unwrapped ];
            packages = with pkgs; [
              bun
              cargo
              chromium
              clippy
              jujutsu
              rust-analyzer
              rustc
              rustfmt
            ];
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            SCORCH_BROWSER_PATH = "${pkgs.chromium}/bin/chromium";
          };
        }
      );

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);

      overlays.default =
        final: _prev:
        let
          packages = mkPackages final;
        in
        {
          scorch = packages.scorch;
          scorchd = packages.scorchd;
          scorchd-with-chromium = packages.scorchd-with-chromium;
          scorch-agent-skill = packages.skill;
        };
      nixosModules.default = import ./nix/module.nix { inherit self; };
      nixosModules.scorch = self.nixosModules.default;
      homeManagerModules.default = import ./nix/home-manager.nix { inherit self; };
      homeManagerModules.scorch = self.homeManagerModules.default;
    };
}
