{ pkgs, ... }:

{
  packages = with pkgs; [
    bun
    chromium
    clang
    cmake
    git
    jujutsu
    llvmPackages.libclang
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
    LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
    RUST_BACKTRACE = "1";
    SCORCH_BROWSER_PATH = "${pkgs.chromium}/bin/chromium";
  };

  scripts = {
    check.exec = "cargo check --workspace --all-targets";
    fmt.exec = "cargo fmt --all";
    lint.exec = "cargo clippy --workspace --all-targets --all-features -- -D warnings";
    test.exec = "cargo test --workspace --all-targets";
  };

  enterShell = ''
    echo "Scorch development environment"
    bun --version
    rustc --version
    chromium --version
    jj --version
  '';
}
