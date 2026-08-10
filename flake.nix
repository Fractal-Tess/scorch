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
          version = "0.1.3";
          releaseArtifacts = {
            x86_64-linux = {
              target = "x86_64-unknown-linux-gnu";
              hash = "sha256-u9sKkqGUEHjOcXnRck2krBgJFHuz1XqM7XesvO0yPqs=";
            };
            aarch64-linux = {
              target = "aarch64-unknown-linux-gnu";
              hash = "sha256-WFC4qqHkES5OSpj4t+7wJsjqrrbXu9bcnsj9dHSWEqo=";
            };
          };
          artifact = releaseArtifacts.${pkgs.stdenv.hostPlatform.system};
          releaseArchive = pkgs.fetchurl {
            url = "https://github.com/Fractal-Tess/scorch/releases/download/v${version}/scorch-v${version}-${artifact.target}.tar.xz";
            inherit (artifact) hash;
          };
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
                makeWrapper ${scorchd}/bin/scorchd "$out/bin/scorchd" \
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
          scorch-unwrapped = scorch;
          scorchd-unwrapped = scorchd;
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
            assert nixpkgs.lib.hasInfix "--bind 127.0.0.1:33000"
              moduleSystem.config.systemd.services.scorchd.serviceConfig.ExecStart;
            pkgs.runCommand "scorch-module-check" { } ''
              touch "$out"
            '';
          version = pkgs.runCommand "scorch-version-check" { } ''
            test "$(${packages.scorch}/bin/scorch --version)" = 'scorch 0.1.3'
            test "$(${packages.scorchd}/bin/scorchd --version)" = 'scorchd 0.1.3'
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
            nativeBuildInputs = [ pkgs.rustPlatform.bindgenHook ];
            packages = with pkgs; [
              bun
              cargo
              chromium
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
