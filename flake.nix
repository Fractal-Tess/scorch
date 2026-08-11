{
  description = "Scorch web search, scraping, mapping, and crawling";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      # Release CI publishes native GNU/Linux archives for these platforms.
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      mkPackages =
        pkgs:
        let
          lib = pkgs.lib;

          # Keep the release tag and fixed archive hashes together. Promotion CI
          # updates these only after both immutable artifacts have been prepared.
          version = "0.3.0";
          releaseArtifacts = {
            x86_64-linux = {
              target = "x86_64-unknown-linux-gnu";
              hash = "sha256-9B1wAWkAJHcNScPtsCt0WXVYj8psWlqQUeNH/8L0HKA=";
            };
            aarch64-linux = {
              target = "aarch64-unknown-linux-gnu";
              hash = "sha256-xz70PBMqCiI2QPcvHJ0dREF/LLl7FUvRMqzmHDa2bmw=";
            };
          };
          artifact = releaseArtifacts.${pkgs.stdenv.hostPlatform.system};
          releaseArchive = pkgs.fetchurl {
            url = "https://github.com/Fractal-Tess/scorch/releases/download/v${version}/scorch-v${version}-${artifact.target}.tar.xz";
            inherit (artifact) hash;
          };
          # Package release binaries directly: normal Nix installation must not
          # compile Rust or the embedded Obscura runtime.
          mkBinaryPackage =
            {
              pname,
              binary,
              description,
            }:
            pkgs.stdenvNoCC.mkDerivation {
              inherit pname version;
              src = releaseArchive;
              sourceRoot = "scorch-v${version}-${artifact.target}";

              # CI targets generic glibc; autoPatchelf binds the downloaded
              # executables to the host system's runtime libraries.
              nativeBuildInputs = [ pkgs.autoPatchelfHook ];
              buildInputs = [ pkgs.stdenv.cc.cc.lib ];
              strictDeps = true;
              dontBuild = true;
              dontStrip = true;
              doInstallCheck = true;
              installPhase = ''
                runHook preInstall
                install -Dm755 "bin/${binary}" "$out/bin/${binary}"
                install -Dm644 "share/licenses/scorch/LICENSE" "$out/share/licenses/scorch/LICENSE"
                install -Dm644 "share/licenses/scorch/THIRD_PARTY_LICENSES.html" \
                  "$out/share/licenses/scorch/THIRD_PARTY_LICENSES.html"
                runHook postInstall
              '';
              installCheckPhase = ''
                runHook preInstallCheck
                test "$($out/bin/${binary} --version)" = '${binary} ${version}'
                runHook postInstallCheck
              '';

              meta = {
                inherit description;
                homepage = "https://github.com/Fractal-Tess/scorch";
                license = lib.licenses.mit;
                mainProgram = binary;
                platforms = builtins.attrNames releaseArtifacts;
                sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
              };
            };
          scorch = mkBinaryPackage {
            pname = "scorch";
            binary = "scorch";
            description = "HTTP-only Scorch command-line and MCP client";
          };
          scorchd = mkBinaryPackage {
            pname = "scorchd";
            binary = "scorchd";
            description = "Scorch HTTP API, metasearch, browser, and crawl service";
          };
          # Expose the repository skill as a small standalone Nix package.
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
          scorch-unwrapped = scorch;
          scorchd-unwrapped = scorchd;
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

      # Validate packages and module policy on every supported architecture.
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
          # Evaluate the generated systemd command to catch module regressions
          # without starting a daemon during flake evaluation.
          module =
            assert nixpkgs.lib.hasInfix "--obscura-stealth true"
              moduleSystem.config.systemd.services.scorchd.serviceConfig.ExecStart;
            assert
              !(nixpkgs.lib.hasInfix "--browser" moduleSystem.config.systemd.services.scorchd.serviceConfig.ExecStart);
            assert nixpkgs.lib.hasInfix "--bind 127.0.0.1:33000"
              moduleSystem.config.systemd.services.scorchd.serviceConfig.ExecStart;
            pkgs.runCommand "scorch-module-check" { } ''
              touch "$out"
            '';
          version = pkgs.runCommand "scorch-version-check" { } ''
            test "$(${packages.scorch}/bin/scorch --version)" = 'scorch 0.3.0'
            test "$(${packages.scorchd}/bin/scorchd --version)" = 'scorchd 0.3.0'
            touch "$out"
          '';
        }
      );

      # `.envrc` enters this shell; it contains build tooling, not runtime
      # dependencies or a source-built Scorch package.
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = [ pkgs.rustPlatform.bindgenHook ];
            packages = with pkgs; [
              bun
              cargo
              clang
              clippy
              cmake
              git
              jujutsu
              perl
              pkg-config
              rust-analyzer
              rustc
              rustfmt
            ];
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          };
        }
      );

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);

      # Consumers can choose the overlay or the explicit NixOS/Home Manager
      # modules without importing implementation files directly.
      overlays.default =
        final: _prev:
        let
          packages = mkPackages final;
        in
        {
          scorch = packages.scorch;
          scorchd = packages.scorchd;
          scorch-agent-skill = packages.skill;
        };
      nixosModules.default = import ./nix/module.nix { inherit self; };
      nixosModules.scorch = self.nixosModules.default;
      homeManagerModules.default = import ./nix/home-manager.nix { inherit self; };
      homeManagerModules.scorch = self.homeManagerModules.default;
    };
}
