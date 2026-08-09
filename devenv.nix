{ pkgs, ... }:

{
  packages = with pkgs; [
    chromium
    git
    jujutsu
    pkg-config
  ];

  languages.rust = {
    enable = true;
    channel = "stable";
    components = [
      "cargo"
      "clippy"
      "rust-analyzer"
      "rust-src"
      "rustc"
      "rustfmt"
    ];
  };

  env = {
    RUST_BACKTRACE = "1";
    SCORCH_BROWSER_PATH = "${pkgs.chromium}/bin/chromium";
  };

  scripts = {
    check.exec = "cargo check --all-targets";
    fmt.exec = "cargo fmt --all";
    lint.exec = "cargo clippy --all-targets --all-features -- -D warnings";
    test.exec = "cargo test --all-targets";
  };

  enterShell = ''
    echo "Scorch development environment"
    rustc --version
    chromium --version
    jj --version
  '';
}
