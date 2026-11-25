{
  description = "omfp: web and desktop GUI for oh-my-pi (TypeScript UI, Rust daemon)";

  # ENV-034: the project's binary cache (CI pushes the dev shell and the packages); Nix asks before using it.
  nixConfig = {
    extra-substituters = [ "https://omfp.cachix.org" ];
    extra-trusted-public-keys = [
      "omfp.cachix.org-1:WcxTiZMXSYmi9k6fxowUuIMmszu9aWymk2djl9kXjHc="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # omp pinned to an upstream release tag (never main).
    # Must match the `tag` of the `omp-rpc` git dependency in Cargo.toml.
    # Shares our nixpkgs and rust-overlay; the Intel-macOS inputs are collapsed
    # because x86_64-darwin is not a supported system here.
    oh-my-pi = {
      url = "github:can1357/oh-my-pi/v19.2.0";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        nixpkgs-darwin-x64.follows = "nixpkgs";
        rust-overlay.follows = "rust-overlay";
        bun2nix-darwin-x64.follows = "oh-my-pi/bun2nix";
        bun2nix.inputs.flake-parts.follows = "flake-parts";
      };
    };

    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ inputs.git-hooks.flakeModule ];

      # ENV-066: `omfp daemon` as a systemd user service of home-manager.
      flake.homeManagerModules.default = import ./nix/home-manager.nix { inherit (inputs) self; };

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      perSystem =
        {
          config,
          system,
          lib,
          ...
        }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ inputs.rust-overlay.overlays.default ];
          };

          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          omp = inputs.oh-my-pi.packages.${system}.omp;
          # Linux WebKitGTK decodes HTML media through GStreamer. Keep the runtime codecs explicit: Nix does not
          # discover plug-ins outside this closure, and WebKit's own dependency carries only core/base.
          gstPlugins = lib.optionals pkgs.stdenv.hostPlatform.isLinux (
            with pkgs.gst_all_1;
            [
              gstreamer
              gst-plugins-base
              gst-plugins-good
              gst-plugins-bad
              gst-libav
            ]
          );

          # bun2nix (the oh-my-pi input's, shared via `follows`) with nix-community/bun2nix#110 (open, commit
          # 0456acb): 2.1.2 refuses the `lockfileVersion: 2` of Bun 1.4 although the `packages` layout is
          # unchanged. Drop it once the PR is released and the oh-my-pi input moves to that bun2nix.
          bun2nix = inputs.oh-my-pi.inputs.bun2nix.packages.${system}.bun2nix.overrideAttrs (old: {
            patches = (old.patches or [ ]) ++ [
              (pkgs.fetchpatch {
                name = "bun2nix-pr-110-lockfile-v2-v3.patch";
                url = "https://github.com/nix-community/bun2nix/commit/0456acb1b7394fc14c414b056aa889df5124943a.patch";
                hash = "sha256-mGmIuMrizTJPKwcB9Jqiewjza9HJ4n9Evxynb00YVsg=";
              })
            ];
            # The patch is relative to the bun2nix repository; the package builds programs/bun2nix.
            patchFlags = [ "-p3" ];
          });

          # ENV-001: the JavaScript dependencies of ui/ as a Bun install cache, built from ui/bun.nix
          # (`just gen-bun-nix`); `just ui-install` installs from it offline. Workspace packages (`@omfp/*`) are
          # linked from ui/packages by `bun install`, not read from the cache: leaving them out keeps the cache,
          # and with it the shell, from changing with every source edit. `@napi-rs/canvas` (an optional dependency
          # of `pdfjs-dist`, its canvas under Node, with one native package per platform) is left out too: the
          # browser has its own canvas, and the install skips an optional package missing from the cache.
          uiBunNix = import ./ui/bun.nix;
          uiBunDeps = bun2nix.fetchBunDeps {
            bunNix = lib.setFunctionArgs (
              args:
              lib.filterAttrs (name: _: !(lib.hasPrefix "@omfp/" name || lib.hasPrefix "@napi-rs/canvas" name)) (
                uiBunNix args
              )
            ) (lib.functionArgs uiBunNix);
          };

          # ENV-030..033: the installable app (nix/package.nix) with the release web bundle (nix/web.nix). The
          # toolchain's channel without its components: with `rust-src` installed, rustc writes the real path of the
          # std sources into the binary's panic locations, which pulls the whole toolchain into the closure.
          packageToolchain = pkgs.rust-bin.fromRustupToolchain {
            inherit ((lib.importTOML ./rust-toolchain.toml).toolchain) channel;
            profile = "minimal";
          };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = packageToolchain;
            rustc = packageToolchain;
          };
          omfpWeb = pkgs.callPackage ./nix/web.nix {
            inherit bun2nix;
            bunDeps = uiBunDeps;
            inherit ((lib.importTOML ./Cargo.toml).workspace.package) version;
          };
          omfpPackage =
            desktop:
            pkgs.callPackage ./nix/package.nix {
              inherit
                rustPlatform
                omp
                desktop
                gstPlugins
                ;
              web = omfpWeb;
            };
        in
        {
          packages = {
            omfp = omfpPackage true;
            omfp-headless = omfpPackage false;
            omfp-web = omfpWeb;
            default = config.packages.omfp;
          };

          # ENV-066: the home-manager module evaluates to the expected unit (built by `just package`).
          checks.home-manager-module = pkgs.callPackage ./nix/home-manager-check.nix {
            module = import ./nix/home-manager.nix { inherit (inputs) self; };
            package = config.packages.omfp-headless;
          };

          formatter = pkgs.nixfmt;

          # Hooks run on commit (installed by the dev shell) and as `checks.<system>.pre-commit`.
          pre-commit.settings = {
            # ENV-040: nixpkgs' pre-commit sets DOTNET_ROOT and checks against a source-built .NET SDK, which
            # cache.nixos.org does not have for aarch64-darwin (hours of building): skip its tests and that env there.
            package =
              if pkgs.stdenv.hostPlatform.isDarwin then
                (pkgs.pre-commit.overridePythonAttrs { env = { }; }).overrideAttrs {
                  doCheck = false;
                  doInstallCheck = false;
                  # nixpkgs' pytest hook runs as a pre-dist phase whatever doCheck says.
                  dontUsePytestCheck = true;
                }
              else
                pkgs.pre-commit;
            hooks = {
              # Nix
              nixfmt.enable = true;
              deadnix = {
                enable = true;
                # Generated by bun2nix (`just gen-bun-nix`), which declares every fetcher it may use.
                excludes = [ "^ui/bun\\.nix$" ];
              };
              statix.enable = true;
              # Rust (runs only once .rs files exist)
              rustfmt = {
                enable = true;
                packageOverrides = {
                  cargo = rustToolchain;
                  rustfmt = rustToolchain;
                };
              };
              # TOML, Markdown, spelling
              taplo.enable = true;
              markdownlint = {
                enable = true;
                settings.configuration = {
                  MD013 = false; # line length: tables and long requirement rows
                  MD024.siblings_only = true; # repeated headings across sections
                };
              };
              typos = {
                enable = true;
                # Recorded omp output: streamed deltas split words at arbitrary points.
                excludes = [ "^fixtures/" ];
              };
              # Generic hygiene
              check-merge-conflicts.enable = true;
              check-toml.enable = true;
              check-yaml.enable = true;
              end-of-file-fixer.enable = true;
              trim-trailing-whitespace.enable = true;
              # Conventional Commits on the commit message
              # (commitizen instead of convco: nixpkgs' convco 0.7.2 fails to build against libgit2.)
              commitizen.enable = true;
            };
          };

          devShells.default = pkgs.mkShell {
            packages = [
              # Agent (version pinned by the oh-my-pi input)
              omp

              # Rust
              rustToolchain

              # Testing (TDD): runner, snapshots, coverage, mutation, dependency lint
              pkgs.cargo-nextest
              pkgs.cargo-insta
              pkgs.cargo-llvm-cov
              pkgs.cargo-mutants
              pkgs.cargo-deny

              # TypeScript UI (D-10, D-18): Bun is runtime, package manager and test runner. The JS dependencies
              # come from Nix (ENV-001): bun2nix turns ui/bun.lock into ui/bun.nix, whose Bun cache `just
              # ui-install` installs from offline. Biome comes from nixpkgs: the npm build is a dynamically linked
              # glibc binary NixOS cannot run.
              pkgs.bun
              bun2nix
              pkgs.biome

              # Utilities
              pkgs.just
              pkgs.git
              pkgs.jq
              pkgs.nixfmt
              # GitHub Actions workflow lint (`just check`, .github/workflows)
              pkgs.actionlint
              # The release packages download the pinned omp's release build (ENV-061)
              pkgs.curl
            ]
            ++ config.pre-commit.settings.enabledPackages
            ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              # Web UI E2E over WebDriver (versions aligned in nixpkgs)
              pkgs.chromium
              pkgs.chromedriver
              # Desktop smoke test under a virtual X server; real wheel input there for WebKitGTK (e2e/tests/scroll.rs)
              pkgs.xvfb-run
              pkgs.xdotool
              # Desktop (M7, S-04): Tauri 2 on WebKitGTK 4.1 (GTK3, GLib, cairo come through webkitgtk's .pc files)
              pkgs.pkg-config
              pkgs.webkitgtk_4_1
              pkgs.libsoup_3
              # TLS for `omfp desktop --connect https://…`: GIO loads glib-networking from GIO_EXTRA_MODULES (env below)
              pkgs.glib-networking
              # A private session bus with a stub settings portal for the desktop tests (UI-195)
              pkgs.dbus
              # The Linux release packages: `just package-linux` links against an older glibc's symbols (ENV-062),
              # `just package-flatpak` builds the Flatpak (ENV-065)
              pkgs.cargo-zigbuild
              pkgs.zig
              pkgs.flatpak
              pkgs.flatpak-builder
              # flatpak-builder composes the AppStream metadata with appstreamcli
              pkgs.appstream
            ]
            ++ gstPlugins;

            env = {
              RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
              OMFP_BUN_CACHE = "${uiBunDeps}/share/bun-cache";
            }
            // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
              GST_PLUGIN_SYSTEM_PATH_1_0 = lib.makeSearchPath "lib/gstreamer-1.0" gstPlugins;
              CHROME_BIN = lib.getExe pkgs.chromium;
              # The WebKitGTK journeys' driver (e2e::WebKitDriver): `cargo test` does not find it on PATH, whose
              # entry for it holds a `=`.
              OMFP_WEBKIT_WEBDRIVER = "${pkgs.webkitgtk_4_1}/bin/WebKitWebDriver";
              # The only fonts of the e2e browsers (e2e/src/lib.rs), never the host's: CI's purged runner has none, and
              # Chromium's renderer crashes without any. fontconfig's own rules map the generic families (and a "Mono"
              # or "Serif" in a family name) onto DejaVu.
              OMFP_E2E_FONTCONFIG_FILE = pkgs.writeText "omfp-e2e-fonts.conf" ''
                <?xml version="1.0"?>
                <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
                <fontconfig>
                  <dir>${pkgs.dejavu_fonts}</dir>
                  <cachedir prefix="xdg">fontconfig</cachedir>
                  ${lib.concatMapStrings
                    (rules: "<include>${pkgs.fontconfig.out}/etc/fonts/conf.d/${rules}</include>")
                    [
                      "45-latin.conf"
                      "48-guessfamily.conf"
                      "49-sansserif.conf"
                      "60-latin.conf"
                    ]
                  }
                </fontconfig>
              '';
              # Adding glib-networking to `packages` does not set it: the shell's GIO_EXTRA_MODULES holds only dconf.
              GIO_EXTRA_MODULES = "${pkgs.glib-networking}/lib/gio/modules";
            };

            shellHook = ''
              ${config.pre-commit.installationScript}
              echo "omfp dev shell — $(omp --version 2>/dev/null | head -n1), $(rustc --version), bun $(bun --version)"
            '';
          };
        };
    };
}
