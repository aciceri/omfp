#!/usr/bin/env bash
# Spike S-16: builds omfp-cef-spike and runs it with its arguments, e.g. `./run.sh --url='http://127.0.0.1:7420/#token=…'`.
# Run inside omfp's dev shell (`nix develop`). CEF's prebuilt Linux binaries load the usual desktop libraries (GLib,
# NSS, X11, …) by name; on NixOS they come from nixpkgs through LD_LIBRARY_PATH.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cache="${OMFP_CEF_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/omfp-cef-spike}"
export CEF_PATH="$cache/cef"
export CARGO_TARGET_DIR="$cache/target"
mkdir -p "$CEF_PATH"
cargo build --quiet --manifest-path "$here/Cargo.toml"

# The build copies libcef.so and CEF's resources next to the executable.
export LD_LIBRARY_PATH="$CARGO_TARGET_DIR/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
if [ -e /etc/NIXOS ]; then
  libraries=$(nix build --no-link --print-out-paths \
    nixpkgs#glib.out nixpkgs#nspr nixpkgs#nss nixpkgs#at-spi2-core nixpkgs#dbus.lib nixpkgs#cups.lib \
    nixpkgs#libX11 nixpkgs#libXcomposite nixpkgs#libXdamage nixpkgs#libXext nixpkgs#libXfixes nixpkgs#libXrandr \
    nixpkgs#libxcb nixpkgs#libxkbcommon nixpkgs#libgbm nixpkgs#expat nixpkgs#cairo nixpkgs#pango.out \
    nixpkgs#systemd nixpkgs#alsa-lib nixpkgs#libGL nixpkgs#fontconfig.lib \
    | sed 's|$|/lib|' | paste -sd:)
  export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:$libraries"
fi
exec "$CARGO_TARGET_DIR/debug/omfp-cef-spike" "$@"
