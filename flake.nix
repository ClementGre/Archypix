{
  description = "Archypix development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Stable Rust + useful extensions
        rust-toolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
        };

        # Native build dependencies (present at compile time)
        nativeBuildInputs = with pkgs; [
          rust-toolchain
          pkg-config
          sqlx-cli          # for sqlx migrate + sqlx prepare
          cargo-watch       # for cargo-watch hot-reload
          cargo-nextest     # faster test runner
          git               # for development
        ];

        # Runtime / link-time libraries
        buildInputs = with pkgs; [
          # Worker imaging deps
          imagemagick       # libMagickWand — thumbnail generation
          gexiv2            # libgexiv2 — EXIF read/write via GExiv2/Exiv2
          exiv2             # required by gexiv2
          glib              # required by gexiv2 (GLib/GObject)
          ffmpeg            # ffprobe/ffmpeg — video metadata + frame-grab thumbnails

          # Back / resolver TLS
          openssl

          # Dev infrastructure (used via docker-compose, not compiled against)
          docker-compose
          postgresql_16     # psql client for manual queries

          # macOS: libiconv is required by some Rust crates on Darwin.
          # Apple SDK frameworks (Security, CoreFoundation, etc.) are provided
          # automatically by the system Xcode CLT — do not list them explicitly
          # here to avoid version-mismatch errors with nixpkgs-unstable.
        ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];

      in {
        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs buildInputs;

          # Expose pkg-config paths for the native imaging libraries so cargo
          # can find them when compiling rexiv2 and magick_rust.
          shellHook = ''
            export PKG_CONFIG_PATH="${pkgs.gexiv2.dev}/lib/pkgconfig:${pkgs.glib.dev}/lib/pkgconfig:${pkgs.exiv2.dev}/lib/pkgconfig:${pkgs.imagemagick.dev}/lib/pkgconfig:${pkgs.openssl.dev}/lib/pkgconfig:$PKG_CONFIG_PATH"
            echo "Archypix dev shell ready."
            echo "  Rust:        $(rustc --version)"
            echo "  sqlx-cli:    $(sqlx --version 2>/dev/null || echo 'not found')"
            echo "  ImageMagick: $(magick --version | head -1)"
            echo "  GExiv2:      ${pkgs.gexiv2.version}"
            echo "  ffmpeg:      $(ffmpeg -version 2>/dev/null | head -1 || echo 'not found')"
          '';
        };
      }
    );
}
