{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    naersk.url = "github:nix-community/naersk";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    naersk,
    rust-overlay,
  }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      inherit system;
      overlays = [rust-overlay.overlays.default];
    };

    rustToolchain = pkgs.rust-bin.nightly.latest.default;
    naerskLib = pkgs.callPackage naersk {
      rustc = rustToolchain;
      cargo = rustToolchain;
    };
  in {
    packages.${system}.default = naerskLib.buildPackage {
      src = ./.;
      cargoLock = ./Cargo.lock;
      hardeningDisable = ["fortify"];
      nativeBuildInputs = builtins.attrValues {
        inherit
          (pkgs)
          pkg-config
          gnumake
          autoconf
          automake
          libtool
          gawk
          ;
      };
    };

    devShells.${system}.default = pkgs.mkShell {
      hardeningDisable = ["fortify"];

      packages = [
        rustToolchain
        pkgs.rust-analyzer

        pkgs.gcc
        pkgs.gnumake
        pkgs.autoconf
        pkgs.automake
        pkgs.libtool
        pkgs.pkg-config
        pkgs.gawk
        pkgs.ffmpeg
      ];

      CFLAGS = "-DJEMALLOC_STRERROR_R_RETURNS_CHAR_WITH_GNU_SOURCE";
    };
  };
}
