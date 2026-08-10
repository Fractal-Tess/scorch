{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.scorch;
  system = pkgs.stdenv.hostPlatform.system;
in
{
  options.programs.scorch = {
    enable = lib.mkEnableOption "the Scorch HTTP client and MCP adapter";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${system}.scorch;
      defaultText = lib.literalExpression "inputs.scorch.packages.${pkgs.system}.scorch";
      description = "Scorch client package to install.";
    };
    apiUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "http://127.0.0.1:3000";
      description = "Optional SCORCH_API_URL exported to the user environment.";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];
    home.sessionVariables = lib.mkIf (cfg.apiUrl != null) {
      SCORCH_API_URL = cfg.apiUrl;
    };
  };
}
