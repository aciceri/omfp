# 01 — Development environment and versions

## Dev shell

`flake.nix` (flake-parts) provides the development shell and the installable app (see [Packages](#packages)).

```sh
nix develop          # or: direnv allow (.envrc contains `use flake`)
```

The shell's packages are listed in `flake.nix` (`devShells.default`); versions come from `flake.lock`,
`rust-toolchain.toml` and `ui/bun.lock`.

| Group | Tools |
| --- | --- |
| Agent | `omp` built from the pinned upstream release (`inputs.oh-my-pi`), its inputs following ours |
| Rust | toolchain from `rust-toolchain.toml` via rust-overlay (clippy, rustfmt, rust-analyzer, rust-src, llvm-tools) |
| Testing | `cargo-nextest`, `cargo-insta`, `cargo-llvm-cov`, `cargo-mutants`, `cargo-deny` |
| TypeScript (`ui/`) | `bun`, `bun2nix` (patched, see below), `biome` (from nixpkgs: the npm build is a glibc binary NixOS cannot run); `typescript` (native `tsc`) and `@types/bun` from `ui/bun.lock` |
| E2E / smoke (Linux) | `chromium`, `chromedriver`, `xvfb-run`, `xdotool` |
| Desktop (Linux) | `pkg-config`, `webkitgtk_4_1`, `libsoup_3`, `glib-networking`, GStreamer core/base/good/bad/libav, `dbus`; no Tauri CLI: `cargo build -p omfp --features desktop` is enough (S-04) |
| Utilities | `just`, `git`, `jq`, `nixfmt`, `actionlint`, `curl` |
| Release packages (Linux) | `cargo-zigbuild` and `zig` (ENV-062); `flatpak`, `flatpak-builder` and `appstream` (ENV-065) |

Environment set by the shell: `RUST_SRC_PATH`, `OMFP_BUN_CACHE` (the Bun install cache of `ui/`'s dependencies, built by
Nix from `ui/bun.nix`); on Linux also `CHROME_BIN`, `OMFP_E2E_FONTCONFIG_FILE` (a fontconfig file with DejaVu, the only
fonts of the e2e browsers: the host's fonts would change text metrics, and CI's purged runner has none, without which
Chromium's renderer crashes, TST-075), `GST_PLUGIN_SYSTEM_PATH_1_0` (the GStreamer plug-ins WebKitGTK uses for HTML
media) and `GIO_EXTRA_MODULES` (glib-networking's GIO modules: listing the package in the shell does not set it, and
WebKitGTK shows "TLS support is not available" without it).

Known workarounds:

- bun2nix 2.1.2 (the `oh-my-pi/bun2nix` input) rejects Bun 1.4's `bun.lock` only because it checks
  `lockfileVersion == 1`; versions 2 and 3 parse unchanged. The flake applies the open upstream PR
  nix-community/bun2nix#110 (commit `0456acb`) with `fetchpatch`. Drop it once the PR is released and the input moves
  to that bun2nix (or an omp upgrade, ENV-020, brings it).
- The desktop window (M7) needs no variable under Xvfb or headless weston. With NVIDIA's proprietary driver on Wayland,
  WebKitGTK is known to show a blank window (untested: the S-04 machine has an AMD GPU): run it with
  `WEBKIT_DISABLE_DMABUF_RENDERER=1`; it is not set by default.

| ID | Requirement |
| --- | --- |
| ENV-001 | Every tool needed by `just ci` MUST come from the dev shell; nothing is installed with `cargo install`, `npm` or downloaded by tools at run time. The JavaScript dependencies of `ui/` come from Nix too: `bun2nix` turns `ui/bun.lock` into `ui/bun.nix` (`just gen-bun-nix`, committed; `just check-bun-nix`, run by `just ui-install`, fails when it is stale), Nix builds Bun's install cache from it (`OMFP_BUN_CACHE`), and `just ui-install` runs `bun install --frozen-lockfile --offline` against that cache. |

Supported dev systems (ENV-003): `x86_64-linux`, `aarch64-linux`, `aarch64-darwin`; Linux-only tools (Chromium,
chromedriver, xvfb, WebKitGTK and the desktop window) are excluded on macOS. `flake.lock` pins every input (ENV-002);
`nixpkgs` and `rust-overlay` are updated deliberately, in their own commits, with `just ci` green.

## Packages

```sh
nix build .#omfp            # result/bin/omfp: daemon, theme, desktop (Linux)
nix build .#omfp-headless   # the daemon without the desktop window (no WebKitGTK)
nix run .#omfp -- daemon    # or: nix profile install .#omfp
```

`nix/package.nix` builds the `omfp` crate with the channel of `rust-toolchain.toml` in its minimal profile (with
`rust-src` installed, rustc writes the real path of the std sources into panic locations and the whole toolchain ends up
in the closure) and `nix/web.nix` the release web bundle (`bun run build` in `ui/`, as `just build-web`) from the same
Bun cache as the shell (`OMFP_BUN_CACHE`). nixpkgs' cargo hook sets `LIBSQLITE3_SYS_USE_PKG_CONFIG=1`, so the package
links nixpkgs' sqlite where the dev build compiles rusqlite's bundled copy. Installed layout:

| Path | Content |
| --- | --- |
| `bin/omfp` | wrapper (wrapGAppsHook3 for `omfp` on Linux, makeWrapper for `omfp-headless`, a compiled makeBinaryWrapper on macOS) around `bin/.omfp-wrapped`, the real binary; a daemon `omfp desktop` starts in the background is `.omfp-wrapped` and inherits the wrapper's environment |
| `bin/web` → `share/omfp/web` | the web bundle (`packages.omfp-web`), found next to the executable (DMN-008) |
| `share/applications/omfp.desktop`, `share/icons/hicolor/512x512/apps/omfp.png` | desktop entry running `omfp desktop` (`omfp` on Linux only) |
| `lib/systemd/user/omfp.service` | Linux (ENV-067): the systemd user unit running `bin/omfp daemon` |
| `Applications/omfp.app` | macOS (`omfp` only, ENV-041): `Contents/Info.plist`, `Contents/Resources/omfp.icns`, `Contents/MacOS/omfp` (a binary wrapper running `bin/.omfp-wrapped desktop` with the same `PATH`), ad-hoc signed |

| ID | Requirement |
| --- | --- |
| ENV-030 | `nix build .#omfp` (also `packages.default`) MUST build the `omfp` binary with the `desktop` feature, on Linux wrapped by `wrapGAppsHook3` with `webkitgtk_4_1`, `libsoup_3`, `glib-networking` (`GIO_EXTRA_MODULES`, TLS for `--connect https://…`), `gsettings-desktop-schemas` (`XDG_DATA_DIRS`) and GStreamer core/base/good/bad/libav under `GST_PLUGIN_SYSTEM_PATH_1_0`, so `omfp desktop` runs outside the dev shell and decodes common HTML media including H.264 (macOS: ENV-041). The `omp-rpc` git dependency is vendored by `cargoLock.outputHashes` in `nix/package.nix`, which changes with every omp upgrade (ENV-020). |
| ENV-031 | The package MUST ship the release web bundle built by Nix from `ui/` with bun2nix (`ui/bun.nix`, offline, `--frozen-lockfile`) where the daemon finds it without `--static-dir` (`bin/web`, DMN-008): `omfp daemon` serves the UI at `/` and `/theme.css`. |
| ENV-032 | The wrapper MUST put the pinned omp (the `oh-my-pi` input's `omp`) first on `PATH` (DMN-006, DMN-001) and `gitMinimal` last, so the user's own git wins; no `omp` is installed into `bin/`, so the package does not shadow another omp in a profile. |
| ENV-033 | `nix build .#omfp-headless` MUST build the daemon-only binary (no `desktop` feature, no WebKitGTK in its closure) with the same web bundle and `PATH`, for servers. |
| ENV-034 | The flake's `nixConfig` MUST name the project's binary cache (`https://omfp.cachix.org`, key `omfp.cachix.org-1:WcxTiZMXSYmi9k6fxowUuIMmszu9aWymk2djl9kXjHc=`) as `extra-substituters`/`extra-trusted-public-keys`; Nix asks the user to accept it (or `--accept-flake-config`). |
| ENV-066 | The flake MUST export the home-manager module `homeManagerModules.default`: `services.omfp.enable` runs `omfp daemon` as the systemd user service `omfp.service` (options `package`, `tray`, `extraArgs`; `tray` adds `--tray` and ties it to `graphical-session.target`, else `default.target`), with `Restart=on-failure` and `KillMode=mixed`, so a home-manager switch to a new package restarts the daemon onto it. |
| ENV-067 | On Linux the packages MUST ship the systemd user unit `lib/systemd/user/omfp.service` running the package's `bin/omfp daemon` with the settings of ENV-066, wanted by `default.target`: NixOS picks it up with `systemd.packages`, a profile with `systemctl --user link`; elsewhere the file is the template. |

### macOS (M11)

The flake builds on `aarch64-darwin` (S-15): the dev shell, omp (from source when Cachix has no darwin build), the
desktop window (Tauri on the system's WKWebView: AppKit, WebKit and Security come from the darwin stdenv's SDK; no
pkg-config, WebKitGTK, glib-networking or D-Bus) and the packages. The Linux-only shell packages (Chromium,
chromedriver, `xvfb-run`, `xdotool`, WebKitGTK, D-Bus) stay out of the darwin shell, so `just test-e2e` and the
Linux-only checks run on Linux; `just test-desktop` runs on both (TST-090, TST-091). A Mac whose Nix crashes in the
remote-build hook builds with `--option builders ''`.

| ID | Requirement |
| --- | --- |
| ENV-040 | `nix develop` MUST work on `aarch64-darwin`. The binary MUST link for macOS 14.0 (`otool -l`: `LC_BUILD_VERSION` `minos 14.0`, no linker warning about newer objects), the floor of WKWebView's data store identifiers (UI-250); the Info.plist's `LSMinimumSystemVersion` is 14.5, the first WebKit with the `light-dark()` the themes use (UI-374). |
| ENV-041 | On macOS `nix build .#omfp` MUST give `bin/omfp` (the desktop build, a compiled wrapper as on Linux, ENV-032) and `Applications/omfp.app` (`Info.plist` with the workspace version, `Resources/omfp.icns`, `MacOS/omfp` running `omfp desktop`), signed ad-hoc as the last step so macOS knows the app as `dev.omfp.desktop`. Installed with nix-darwin or home-manager, the app is linked into `/Applications/Nix Apps` or `~/Applications/Home Manager Apps`. |
| ENV-042 | (M11) CI MUST build and test the macOS desktop on GitHub's arm64 macOS runner on pushes to `master` and on pull requests labeled `ci/macos` (TST-092), whose Cachix daemon pushes the darwin closure (omp, the dev shell, the packages) to the `omfp` cache (on `master` only). |

## Release packages

For people without Nix (M15): downloads that carry everything `omfp` needs but git, built by plain scripts in the dev
shell (`contrib/release/`) and attached to a GitHub release by `.github/workflows/release.yml`.

```sh
just package-linux [out]     # out/omfp-<version>-linux-x86_64.tar.gz (x86_64 Linux, headless)
just package-flatpak [out]   # out/omfp-<version>-linux-x86_64.flatpak (x86_64 Linux, the desktop window)
just package-macos [out]     # out/omfp-<version>-macos-arm64.dmg and .zip (arm64 Mac); `out` defaults to dist/
```

| Download | Content |
| --- | --- |
| `omfp-<version>-linux-x86_64.tar.gz` | `omfp-<version>-linux-x86_64/`: `omfp` (daemon only, no desktop window), `omp`, `web/`, `LICENSE`, `README.txt`; runs on glibc ≥ 2.34 (Ubuntu 22.04, Debian 12, RHEL 9) |
| `omfp-<version>-linux-x86_64.flatpak` | the Flatpak `io.github.aciceri.omfp` on GNOME 51 (window, tray, daemon): `/app/bin/omfp` (a script setting the data and config directories), `/app/lib/omfp/{omfp,omp,web/}`, the desktop entry, icon and AppStream metadata; installing it fetches the runtime from Flathub |
| `omfp-<version>-macos-arm64.dmg`, `.zip` | `omfp.app` (window and menu bar item): `Contents/MacOS/omfp`, `Contents/MacOS/omp`, `Contents/Resources/web/`, `Contents/Resources/omfp.icns`, `Contents/Info.plist`; macOS 14.5 or newer |
| `SHA256SUMS` | the SHA-256 of every download |

The Flatpak (`contrib/flatpak/`) is distributed from the GitHub release, not Flathub: `flatpak install --user
omfp-<version>-linux-x86_64.flatpak`, then `flatpak run io.github.aciceri.omfp desktop` (or the desktop entry). omp acts
on the user's machine — git, shells, build tools, the `nix` of a repository's dev shell — which a sandbox cannot offer,
so the sandbox holds the window and the daemon while omp, git and the shells of jobs run **on the host** through
`flatpak-spawn --host` (ENV-064): the permission `--talk-name=org.freedesktop.Flatpak` gives the app the host's
privileges, deliberately. The other permissions: Wayland or X11, the GPU, PulseAudio (the microphone), the network,
the notification and tray (`org.kde.*` names) buses, and the home directory, where workspaces (`~/.omfp`), the isolated
omp profile (`<data-dir>/omp-profile`, ARC-021) and linked repositories live at the same paths on both sides; the data
and config directories are the host's (`~/.local/share/omfp`, `~/.config/omfp`, DMN-170, DMN-009), not `~/.var/app`,
so the host's `omfp` commands find the Flatpak's daemon. Workspaces and repositories outside the home directory are
out of reach. The bundled omp is a generic glibc binary, which runs on most distributions but not on NixOS (no
`/lib64/ld-linux-x86-64.so.2`): there set `omp_bin` (DMN-009) to an omp of the pinned version, or use the Nix package.
A job's processes left behind after its shell exited are not killed on the host (DMN-081): the daemon sees
`flatpak-spawn`, not the host's process group.

Signing on macOS: without secrets the app is signed ad-hoc, which Gatekeeper refuses on first open for a download (the
user allows it in System Settings → Privacy & Security → Open Anyway, or removes the quarantine with `xattr -dr
com.apple.quarantine /Applications/omfp.app`), and macOS asks for the microphone again after every update. With an
Apple Developer account the workflow signs with a Developer ID under the hardened runtime and notarizes and staples the
app and the disk image. Secrets (repository settings → Secrets and variables → Actions; locally the same environment
variables, or `OMFP_SIGN_IDENTITY` naming a Developer ID identity of your keychain):

| Name | Use |
| --- | --- |
| `APPLE_CERTIFICATE` | the Developer ID Application certificate and its private key as a base64 `.p12` (`base64 -i cert.p12`) |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_ID`, `APPLE_APP_PASSWORD` | the Apple ID notarytool submits with and an app-specific password of it (appleid.apple.com) |
| `APPLE_TEAM_ID` | the team of the certificate |

Entitlements under the hardened runtime: `contrib/release/omfp.entitlements` (the microphone,
`com.apple.security.device.audio-input`; WKWebView's JavaScript runs in WebKit's own processes) and
`contrib/release/omp.entitlements` (what Bun documents for its single-file executables: JIT, unsigned executable
memory, no library validation for the native addons it extracts).

Release notes live in [`CHANGELOG.md`](../../CHANGELOG.md), not the commit log. Add significant user-facing changes
under `Unreleased`, in English, grouped into Added, Changed, Fixed, Removed or Security as needed. Describe effects
and migrations rather than internal refactoring or test changes; the initial release also lists its platform limits.

Releasing:

1. Set the workspace version in `Cargo.toml` and update the lockfile.
2. Move the `Unreleased` entries under `## <version> — YYYY-MM-DD`, using the release date, and leave a fresh
   `## Unreleased` section above it.
3. Preview the notes with `contrib/release/notes.sh v<version>` and commit the release preparation.
4. Push a signed tag `v<version>`. The workflow builds the three packages, then drafts the release with `SHA256SUMS`
   and exactly that version's changelog body; a missing, undated or empty section fails instead of creating a draft.
5. Check the draft and publish it.

Run by hand (Actions → Release → Run workflow), the workflow builds the packages as the run's artifacts only;
it does not require a released changelog section or create a release.

| ID | Requirement |
| --- | --- |
| ENV-060 | `.github/workflows/release.yml` MUST build the packages on a pushed tag `v*` and on `workflow_dispatch`, each in the dev shell with one recipe (`just package-linux`, `just package-flatpak`, `just package-macos`), and on a tag only attach them with `SHA256SUMS` to a draft release; only the release job writes contents. On a tag the packages MUST fail unless it is `v` + the workspace version. Release notes MUST be the body of that version's dated `CHANGELOG.md` section; a missing, undated or empty section fails before the draft (`just check-workflow`). |
| ENV-061 | (M15) The packages MUST bundle the pinned omp's official release build (`omp-linux-x64`, `omp-darwin-arm64` from `github.com/can1357/oh-my-pi/releases/download/<tag>/`, a Bun single-file executable), checked against the SHA-256 pinned in `contrib/release/omp.sha256`, whose tag `cargo xtask check-versions` checks against the pinned omp (ENV-010). It goes next to `omfp`, where the daemon finds it first (DMN-006). |
| ENV-063 | (M15) `just package-macos` MUST build `omfp.app` without a `/nix/store` reference in any Mach-O (`otool -L`, `otool -l`: nixpkgs' libiconv is relinked to the system's), the web bundle in `Contents/Resources/web` (DMN-008), the Info.plist of ENV-041 with the version, and sign it: ad-hoc, or with a Developer ID, the hardened runtime and the entitlements above, then notarized and stapled (the app first, then the disk image) when the Apple ID secrets are set. Run without arguments, an app bundle's `omfp` (`<name>.app/Contents/MacOS/omfp`) opens `omfp desktop`. |
| ENV-064 | In a Flatpak sandbox (`/.flatpak-info` exists) the daemon MUST start every process that acts for the user — omp, git and the shells of jobs — on the host through `flatpak-spawn --host --watch-bus`, naming a program under `/app` by its host path (`app-path`), so the bundled omp runs from the app's deployment; outside a sandbox commands run unchanged. The daemon itself stays in the sandbox, so it finds a standalone omp credential store under `~/.omp` but not under the host's `$XDG_DATA_HOME/omp` (DMN-229). |

The release scripts check what they build; the release workflow runs them, `just ci` does not:

- **ENV-062**: `just package-linux` builds `omfp` without the `desktop` feature with `cargo zigbuild` for
  `x86_64-unknown-linux-gnu.2.34`: interpreter `/lib64/ld-linux-x86-64.so.2`, no RUNPATH, no glibc symbol version
  newer than 2.34 (weak ones included), only libc, libm, libgcc_s and the loader as libraries, std's paths remapped (no
  `/nix/store` string); the archive is reproducible (sorted, no owners, the commit's time).
- **ENV-065**: `just package-flatpak` builds `omfp-<version>-linux-x86_64.flatpak` from
  `contrib/flatpak/io.github.aciceri.omfp.yml` with flatpak-builder on `org.gnome.Platform`/`Sdk` 51 (WebKitGTK 4.1,
  glibc 2.44) in a Flatpak installation of its own (`target/flatpak/user`, never the user's): the desktop build of
  `omfp` linked in the dev shell, then given the runtime's loader and no RUNPATH, the pinned omp unstripped (ENV-061),
  the web bundle, the permissions above, the data and config directories of the host (`/app/bin/omfp` sets
  `XDG_DATA_HOME` and `XDG_CONFIG_HOME` from `HOST_XDG_*` or `~/.local/share`, `~/.config`), a bundle whose runtime
  comes from Flathub; it installs the bundle there and runs its `omfp --version` in the sandbox.

## Continuous integration

GitHub Actions (`.github/workflows/ci.yml`) runs each recipe of `just ci` as its own job, `nix develop -c just <recipe>`
inside this dev shell, so CI and a local `just ci` run the same tools at the same versions (ENV-001); the jobs, caches
and gates are [06 — Continuous integration](06-testing.md#continuous-integration) (TST-070..TST-076, TST-092, TST-118).

Secrets and variables (repository settings → Secrets and variables → Actions):

| Name | Kind | Use |
| --- | --- | --- |
| `CACHIX_AUTH_TOKEN` | secret | write token of the `omfp` Cachix cache (ENV-034), named in `.github/actions/dev-shell/action.yml`; without it (forks) CI reads the public cache only |

Dependency updates:

| ID | Requirement |
| --- | --- |
| ENV-050 | `.github/dependabot.yml` MUST have Dependabot open one grouped weekly pull request (`ci(deps): …`) updating the GitHub Actions of `.github/workflows` and of the composite action `.github/actions/dev-shell` (listed as its own directory, which Dependabot does not scan under `/`). |
| ENV-051 | `.github/workflows/update-flake-lock.yml` MUST open a weekly (and manually dispatchable) pull request `chore(nix): update flake.lock` with `DeterminateSystems/update-flake-lock`, updating every input but `oh-my-pi` (pinned by hand, ENV-020), with `contents: write` and `pull-requests: write` on its job only. Opened with `GITHUB_TOKEN`, the pull request does not start CI: close and reopen it, or store a fine-grained token (contents and pull requests read/write) as a secret and pass it as the action's `token`. |

## Version pinning

The omp RPC protocol changes with every release, and `omp-rpc` is generated from it. The app MUST always run against the
exact omp version its client was built for.

| Item | Where | Current |
| --- | --- | --- |
| omp binary | `flake.nix` → `inputs.oh-my-pi.url = "github:can1357/oh-my-pi/v19.2.0"` (locked rev `5c5ecb16fa4903c09a57a84983e482d9c9c2232e`) | 19.2.0 |
| `omp-rpc` crate | workspace `Cargo.toml` → `omp-rpc = { git = "https://github.com/can1357/oh-my-pi", tag = "v19.2.0" }` | 19.2.0 |
| Expected version in code | `omfp_core::OMP_VERSION` (DMN-001, contract tests) | `19.2.0` |
| Rust | `rust-toolchain.toml` | 1.99.1 |

| ID | Requirement |
| --- | --- |
| ENV-010 | `cargo xtask check-versions` (part of `just ci`) MUST check that the oh-my-pi tag and commit in `flake.lock`, the `omp-rpc` tag and commit in `Cargo.lock`, `omfp_core::OMP_VERSION`, `omp --version` and the tag of every omp release asset in `contrib/release/omp.sha256` (ENV-061) all agree. |
| ENV-011 | omp MUST be pinned to a release tag, never to `main` or an untagged commit. |

## omp upgrade playbook

**ENV-020**: upgrading omp MUST follow these steps, in one change:

1. Pick the new release tag (`https://github.com/can1357/oh-my-pi/releases`).
2. Read the upstream diff of `docs/rpc.md` and `packages/coding-agent/CHANGELOG.md` between the two tags.
3. Update `inputs.oh-my-pi.url` and run `nix flake lock --update-input oh-my-pi`.
4. Update the `omp-rpc` tag and `omfp_core::OMP_VERSION`; run `cargo update -p omp-rpc`, then update
   `outputHashes."omp-rpc-…"` in `nix/package.nix` to the hash `nix build .#omfp` reports (ENV-030), and the lines of
   `contrib/release/omp.sha256` to the new tag and the hashes of its `SHA256SUMS.txt` (ENV-061).
5. Re-record contract fixtures (`OMFP_RECORD_FIXTURES=1 just test-contract`) and review their diff: new event kinds,
   renamed fields, changed tool argument shapes (TST-012, UI-051).
6. Diff omp's live prompts (`packages/coding-agent/src/live/prompts/`) and live controller
   (`packages/coding-agent/src/live/controller.ts`): omfp's own live prompt `crates/omfp-daemon/src/live_instructions.txt`
   keeps omp's terms for the delegation protocol (DMN-115), and omfp's extension marks omp's `live-delegation`
   requests (DMN-116).
7. Check that the daemon's MCP client still matches omp's credential store and refresh (DMN-352, DMN-353): diff
   `packages/ai/src/auth/sqlite-credential-store.ts`, `packages/ai/src/auth/refresh.ts` and
   `packages/coding-agent/src/mcp/oauth-credentials.ts` (`contract_mcp_servers` fails when the schema moves).
8. Re-check `crates/omfp-daemon/src/supervisor/import.rs` against the new omp's `packages/utils/src/dirs.ts`
   (`DirResolver`, `getAgentDbPath`: which variables pick the credential store, DMN-229).
9. Fix what broke, then `just ci`.
