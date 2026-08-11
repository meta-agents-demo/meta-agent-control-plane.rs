{
  description = "Meta-agent Rust daemon and Leptos control-plane development shell";

  # Keep the shell reproducible even before a reviewed flake.lock is present.
  inputs.nixpkgs.url =
    "github:NixOS/nixpkgs/2fcb964de67fcf60b43471c55d5d99e61a9ccb5a";
  inputs.ores-sops.url =
    "github:ORESoftware/ores-sops/bcedd169490775d58f418a59248a3e2354451cf2";

  outputs = { self, nixpkgs, ores-sops }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          oresSops = ores-sops.packages.${system}.default;
        in {
          default = import ./.nix/devshell.nix {
            inherit pkgs oresSops;
            oresSopsShellHook = ores-sops.lib.shellHook;
          };
        });

      formatter = forAllSystems (system: (import nixpkgs { inherit system; }).nixfmt-rfc-style);
    };
}
