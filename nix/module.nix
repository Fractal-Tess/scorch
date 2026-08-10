{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    types
    ;
  system = pkgs.stdenv.hostPlatform.system;
  client = config.programs.scorch;
  server = config.services.scorchd;
  browserType = types.enum [
    "obscura"
    "chromium"
  ];
  engineType = types.enum [
    "bing"
    "brave"
    "duckduckgo"
    "google"
    "naver"
    "wikipedia"
  ];
  bindAddress =
    if lib.hasInfix ":" server.address then
      "[${server.address}]:${toString server.port}"
    else
      "${server.address}:${toString server.port}";
  serverCommand = lib.escapeShellArgs (
    [
      "${server.package}/bin/scorchd"
      "--bind"
      bindAddress
      "--browser"
      server.browser
      "--allowed-browsers"
      (lib.concatStringsSep "," server.allowedBrowsers)
      "--obscura-stealth"
      (lib.boolToString server.obscuraStealth)
      "--max-concurrency"
      (toString server.maxConcurrency)
      "--max-response-bytes"
      (toString server.maxResponseBytes)
      "--job-ttl-secs"
      (toString server.jobTtlSeconds)
      "--search-engines"
      (lib.concatStringsSep "," server.searchEngines)
    ]
    ++ lib.optionals (server.browserPath != null) [
      "--browser-path"
      server.browserPath
    ]
  );
in
{
  options = {
    programs.scorch = {
      enable = mkEnableOption "the Scorch HTTP client and MCP adapter";
      package = mkOption {
        type = types.package;
        default = self.packages.${system}.scorch;
        defaultText = lib.literalExpression "inputs.scorch.packages.${pkgs.system}.scorch";
        description = "Scorch client package to install system-wide.";
      };
      apiUrl = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "http://127.0.0.1:33000";
        description = "Optional SCORCH_API_URL exported to login sessions.";
      };
    };

    services.scorchd = {
      enable = mkEnableOption "the Scorch web search and extraction service";
      package = mkOption {
        type = types.package;
        default = self.packages.${system}.scorchd;
        defaultText = lib.literalExpression "inputs.scorch.packages.${pkgs.system}.scorchd";
        description = "Scorch service package to execute.";
      };
      address = mkOption {
        type = types.str;
        default = "127.0.0.1";
        description = "Address on which scorchd listens.";
      };
      port = mkOption {
        type = types.port;
        default = 33000;
        description = "TCP port on which scorchd listens.";
      };
      openFirewall = mkOption {
        type = types.bool;
        default = false;
        description = "Open the configured TCP port in the NixOS firewall.";
      };
      browser = mkOption {
        type = browserType;
        default = "obscura";
        description = "Default browser renderer.";
      };
      allowedBrowsers = mkOption {
        type = types.listOf browserType;
        default = [ "obscura" ];
        description = "Browser renderers requests are allowed to select.";
      };
      browserPath = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = lib.literalExpression "\"\${pkgs.chromium}/bin/chromium\"";
        description = "Chromium-compatible executable used when Chromium is enabled.";
      };
      obscuraStealth = mkOption {
        type = types.bool;
        default = true;
        description = "Use Obscura's stealth transport profile.";
      };
      maxConcurrency = mkOption {
        type = types.ints.positive;
        default = 4;
        description = "Maximum concurrent scrape and render operations.";
      };
      maxResponseBytes = mkOption {
        type = types.ints.positive;
        default = 5 * 1024 * 1024;
        description = "Maximum response body size in bytes.";
      };
      jobTtlSeconds = mkOption {
        type = types.ints.positive;
        default = 900;
        description = "Retention period for ephemeral crawl jobs.";
      };
      searchEngines = mkOption {
        type = types.listOf engineType;
        default = [
          "bing"
          "duckduckgo"
          "naver"
          "wikipedia"
        ];
        description = "Metasearch engines enabled by server policy.";
      };
      logLevel = mkOption {
        type = types.str;
        default = "scorch=info";
        description = "RUST_LOG filter used by scorchd.";
      };
      logFormat = mkOption {
        type = types.enum [
          "compact"
          "json"
        ];
        default = "compact";
        description = "Structured service log format.";
      };
      environmentFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "/run/secrets/scorchd.env";
        description = ''
          Optional systemd EnvironmentFile for credentials such as
          SCORCH_BRAVE_SEARCH_API_KEY. Keep secrets out of the Nix store.
        '';
      };
      environment = mkOption {
        type = types.attrsOf types.str;
        default = { };
        example = {
          SCORCH_GOOGLE_SEARCH_ENGINE_ID = "engine-id";
        };
        description = ''
          Additional unmanaged environment variables. Module-managed runtime
          policy values take precedence. Values are written to the Nix store;
          use environmentFile for secrets.
        '';
      };
    };
  };

  config = lib.mkMerge [
    (mkIf client.enable {
      environment.systemPackages = [ client.package ];
      environment.sessionVariables = mkIf (client.apiUrl != null) {
        SCORCH_API_URL = client.apiUrl;
      };
    })

    (mkIf server.enable {
      assertions = [
        {
          assertion = server.allowedBrowsers != [ ];
          message = "services.scorchd.allowedBrowsers must not be empty";
        }
        {
          assertion = builtins.elem server.browser server.allowedBrowsers;
          message = "services.scorchd.browser must be present in allowedBrowsers";
        }
        {
          assertion = server.searchEngines != [ ];
          message = "services.scorchd.searchEngines must not be empty";
        }
        {
          assertion = !(builtins.elem "chromium" server.allowedBrowsers) || server.browserPath != null;
          message = "services.scorchd.browserPath is required when Chromium is allowed";
        }
        {
          assertion =
            !(builtins.elem "chromium" server.allowedBrowsers) || config.security.allowUserNamespaces;
          message = "services.scorchd Chromium support requires security.allowUserNamespaces";
        }
      ];

      networking.firewall.allowedTCPPorts = mkIf server.openFirewall [ server.port ];

      systemd.services.scorchd = {
        description = "Scorch web search and extraction service";
        documentation = [ "https://github.com/Fractal-Tess/scorch" ];
        wantedBy = [ "multi-user.target" ];
        wants = [ "network-online.target" ];
        after = [ "network-online.target" ];
        environment = server.environment // {
          HOME = "/var/lib/scorchd";
          RUST_LOG = server.logLevel;
          SCORCH_LOG_FORMAT = server.logFormat;
        };
        serviceConfig = {
          ExecStart = serverCommand;
          Restart = "on-failure";
          RestartSec = "2s";
          DynamicUser = true;
          StateDirectory = "scorchd";
          WorkingDirectory = "/var/lib/scorchd";
          EnvironmentFile = lib.optional (server.environmentFile != null) server.environmentFile;
          CapabilityBoundingSet = "";
          LockPersonality = true;
          NoNewPrivileges = true;
          PrivateDevices = true;
          PrivateTmp = true;
          ProtectClock = true;
          ProtectControlGroups = true;
          ProtectHome = true;
          ProtectKernelLogs = true;
          ProtectKernelModules = true;
          ProtectKernelTunables = true;
          ProtectSystem = "strict";
          RestrictRealtime = true;
          RestrictSUIDSGID = true;
          UMask = "0077";
        };
      };
    })
  ];
}
