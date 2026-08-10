{ pkgs, oresSops, oresSopsShellHook }:
pkgs.mkShell {
  packages = with pkgs; [
    cargo
    clippy
    git
    jq
    just
    nodejs_22
    python313
    rust-analyzer
    rustc
    rustfmt
  ] ++ [ oresSops ];

  RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
  shellHook = oresSopsShellHook;
}
