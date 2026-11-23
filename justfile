# Development tasks. Run inside the dev shell (`nix develop`).

# List recipes
default:
    @just --list

# Fast Rust tests: unit and integration; no omp process (L0, L1, L2; the TypeScript tests, L4, are `test-ts`). `e2e`
# builds on its own: thirtyfour turns on `serde_json/preserve_order`, which would change every crate's `Value` if
# built together. `just ci` runs these tests once, instrumented, in `cov`.
test:
    cargo nextest run --workspace --exclude e2e -E 'not binary(/^contract_/)'

# Contract tests against the pinned omp and the fake LLM (L3)
test-contract:
    cargo nextest run --workspace --exclude e2e -E 'binary(/^contract_/)'

# spec: TST-113 — end-to-end journeys (L5): headless Chromium on the web bundle, a daemon with the pinned omp and the
# fake LLM; their screenshots for a look by eye go to a new directory of the run (or `OMFP_SCREENSHOT_DIR`)
test-e2e: (with-web-bundle 'export OMFP_SCREENSHOT_DIR="${OMFP_SCREENSHOT_DIR:-$(mktemp -d /tmp/omfp-shots-XXXXXX)}"; echo "screenshots: $OMFP_SCREENSHOT_DIR"; cargo nextest run -p e2e')

# spec: TST-115 — the benchmarks (`benchmark_*` in e2e/tests, ignored): timings of the transcript, typing and big files
# in headless Chromium and WebKitGTK and of view switches, printed, one at a time on a release build; by hand, not in
# `just ci`. Arguments go to nextest, e.g. a test name: `just bench benchmark_file_webkit`
bench *args: (with-web-bundle "cargo nextest run -p e2e --release --run-ignored only --no-capture --test-threads 1 -E 'test(/^benchmark_/)' " + args)

# spec: TST-033, TST-090, TST-091, ENV-040 — the desktop window (`--features desktop`): on Linux (WebKitGTK) under
# `xvfb-run`, on macOS (WKWebView, the menu bar item; the darwin dev shell) on the logged-in user's screen; the smoke
# test and the background daemon shared by two windows; one test at a time, as parallel `xvfb-run -a` can pick the same
# display
test-desktop: (with-web-bundle "cargo nextest run -p omfp --features desktop --test-threads 1")

# spec: TST-074 — `command` with a web bundle of its own in `OMFP_WEB_BUNDLE`: the one given (`just ci` builds it once for its
# recipes, TST-113), else a release bundle built into a temporary directory.
# Never `ui/packages/app/dist`: another `just build-web` deletes and rebuilds it, and a test serving it meanwhile gets
# 404 pages.
[private]
with-web-bundle command:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${OMFP_WEB_BUNDLE:-}" ]]; then
        OMFP_WEB_BUNDLE=$(mktemp -d)
        trap 'rm -rf "$OMFP_WEB_BUNDLE"' EXIT
        export OMFP_WEB_BUNDLE
        {{ quote(just_executable()) }} --justfile {{ quote(justfile()) }} build-web "$OMFP_WEB_BUNDLE"
    fi
    {{ command }}

# Re-record omp fixtures under fixtures/omp/<version>/ (TST-012)
record-fixtures:
    OMFP_RECORD_FIXTURES=1 just test-contract

# JavaScript dependencies of ui/ exactly as locked in ui/bun.lock, offline from the dev shell's Bun cache, which Nix
# builds from ui/bun.nix (ENV-001); the cache is read-only, bun copies the packages into ui/node_modules
ui-install: check-bun-nix
    cd ui && BUN_INSTALL_CACHE_DIR="${OMFP_BUN_CACHE:?run inside the dev shell (nix develop)}" \
        bun install --frozen-lockfile --offline

# Regenerate ui/bun.nix from ui/bun.lock (after every `bun add`/`bun update`, then re-enter the dev shell)
gen-bun-nix:
    cd ui && bun2nix -o bun.nix && nixfmt bun.nix

# spec: ENV-001 — ui/bun.nix matches ui/bun.lock
check-bun-nix:
    cd ui && bun2nix | nixfmt - | diff -u bun.nix - || { echo 'ui/bun.nix is stale: run `just gen-bun-nix`' >&2; exit 1; }

# spec: TST-030, TST-050 — TypeScript tests (`bun run test` in ui/: unit, view-model and component tests, L4), with the
# client's transport suite against a daemon started here; the component tests run with coverage, ≥ 90% of the lines of
# the view models (ui/packages/app/src/vm/), summary in target/bun-coverage/gate.txt
test-ts: ui-install
    #!/usr/bin/env bash
    set -euo pipefail
    data=$(mktemp -d)
    coproc daemon { exec cargo run -q -p omfp-daemon --example test_daemon -- "$data"; }
    trap 'kill "$daemon_PID" 2>/dev/null; wait "$daemon_PID" 2>/dev/null || true; rm -rf "$data"' EXIT
    read -r url token <&"${daemon[0]}"
    out="$PWD/target/bun-coverage"
    cd ui
    export OMFP_TEST_DAEMON_URL="$url" OMFP_TEST_DAEMON_TOKEN="$token"
    bun run test:plain
    bun run test:dom --coverage --coverage-reporter=lcov --coverage-dir="$out"
    awk -F: '/^SF:/ { vm = index($2, "packages/app/src/vm/") > 0 } vm && /^LF:/ { n += $2 } vm && /^LH:/ { h += $2 }
      END { p = h * 100 / n; printf "ui view models: %.2f%% of lines (min 90%%)%s\n", p, (p < 90 ? " FAIL" : "") }' \
      "$out/lcov.info" > "$out/gate.txt"
    cat "$out/gate.txt"
    ! grep -q FAIL "$out/gate.txt"

# Regenerate ui/packages/protocol/src/generated.ts from the daemon's JSON Schemas (`just test` fails when it is stale)
gen-ts:
    cargo xtask gen-ts

# Regenerate the golden reducer states in fixtures/states/<omp version>/ (`just test` fails when they are stale)
gen-states:
    OMFP_UPDATE_STATES=1 cargo nextest run -p omfp-core --test golden_states

# Release web bundle of the TypeScript UI (`bun run build` in ui/packages/app, see its build.ts) into
# ui/packages/app/dist, which the daemon serves (DMN-008), or into the absolute path `out`
build-web out="": ui-install
    cd ui && OMFP_WEB_OUT={{ quote(out) }} bun run build

# Release bundle of the single-page project website, sharing @omfp/kit with the app, into
# ui/packages/site/dist or the absolute path `out`
build-site out="": ui-install
    cd ui && OMFP_SITE_OUT={{ quote(out) }} bun run build:site

# Try the app: the release web bundle on a daemon with your HOME and its own omp profile in the default data dir;
# arguments go to the daemon, e.g. `just serve --bind 127.0.0.1:7421`
serve *args: build-web
    cargo run --release -p omfp -- daemon --static-dir ui/packages/app/dist {{ args }}

# Try the desktop app (M7): one more window on the local daemon of your default data dir, which is started in the
# background with the release web bundle when none runs; arguments go to `omfp desktop`, e.g. `just desktop --connect
# https://host` (then no local daemon)
desktop *args: build-web
    #!/usr/bin/env bash
    set -euo pipefail
    case " {{ args }} " in
        *" --connect"*) daemon=() ;;
        *) daemon=(-- --static-dir "$PWD/ui/packages/app/dist") ;;
    esac
    cargo run --release -p omfp --features desktop -- desktop {{ args }} "${daemon[@]}"

# The logo (UI-220, UI-222) from the bundled Commit Mono Bold (docs/brand/generate.py, fontTools from nixpkgs):
# docs/brand/*.svg, the kit's outlines and the desktop icon's HTML, then the PNG images rendered headlessly: the desktop
# icon (512 px RGBA, which Tauri needs), the same mark at 192 px for the web app manifest (UI-425) and the web
# bundle's 32 px favicon fallback
brand:
    nix shell --impure --expr 'with import <nixpkgs> {}; python3.withPackages (p: [p.fonttools p.brotli])' \
        -c python3 docs/brand/generate.py
    "${CHROME_BIN:?run inside the dev shell (nix develop)}" --headless=new --hide-scrollbars --window-size=512,512 \
        --default-background-color=00000000 --screenshot=crates/omfp/icons/icon.png crates/omfp/icons/icon.html
    "$CHROME_BIN" --headless=new --hide-scrollbars --window-size=192,192 --default-background-color=00000000 \
        --screenshot=docs/brand/app-icon-192.png crates/omfp/icons/icon.html
    "$CHROME_BIN" --headless=new --hide-scrollbars --window-size=32,32 --default-background-color=00000000 \
        --screenshot=docs/brand/icon-32.png docs/brand/icon.svg

# Formatting and lints (Rust; TypeScript: Biome format + lint, then `tsc` in strict mode)
lint: ui-install
    cargo fmt --all --check
    cargo clippy --workspace --exclude e2e --all-targets -- -D warnings
    cargo clippy -p e2e --all-targets --no-deps -- -D warnings
    cargo clippy -p omfp --features desktop --all-targets -- -D warnings
    cd ui && biome ci .
    cd ui && bun run check

# spec: ENV-010, ENV-011 — omp and omp-rpc versions agree
check-versions:
    cargo xtask check-versions

# spec: TST-040 — requirement traceability
spec-trace:
    cargo xtask spec-trace

# Dependency licenses, advisories and sources
deny:
    cargo deny check

# spec: TST-070, TST-071, TST-072, TST-073, TST-075, TST-076, TST-092, TST-112, TST-118, ENV-042, ENV-060 — the
# GitHub workflow (.github/workflows/ci.yml) passes actionlint and runs what `just ci` runs, in the dev shell, with its
# caches, a web bundle per test job, the runner's disk pooled for Nix, a macOS job and one stable required aggregate; the
# release workflow (release.yml) builds the packages in the dev shell and writes only from its tag-only job
# (.github/check-workflow.ts, its tests beside it)
check-workflow:
    bun .github/check-workflow.ts {{ quote(just_executable()) }}
    JUST={{ quote(just_executable()) }} bun test ./.github

# spec: ENV-030, ENV-031, ENV-032, ENV-033, ENV-041 — the Nix packages: `omfp` (web bundle included, the pinned omp on
# the wrapper's PATH; on macOS also `Applications/omfp.app`) and `omfp-headless` (without the desktop window)
package:
    nix build --no-link .#omfp .#omfp-headless

# spec: ENV-061, ENV-062 — the Linux x86_64 download without Nix (the release workflow, ENV-060): `omfp`, the pinned
# omp's release build and the web bundle as `omfp-<version>-linux-x86_64.tar.gz` in `out`
package-linux out="dist":
    contrib/release/package-linux.sh {{ quote(out) }}

# spec: ENV-061, ENV-063 — the macOS arm64 download without Nix (ENV-060): `omfp.app` with the pinned omp and the web
# bundle, ad-hoc or Developer ID signed (and notarized), as `omfp-<version>-macos-arm64.dmg` and `.zip` in `out`
package-macos out="dist":
    contrib/release/package-macos.sh {{ quote(out) }}

# spec: ENV-061, ENV-064, ENV-065 — the Linux desktop download without Nix (ENV-060): the Flatpak with the window, the
# daemon, the web bundle and the pinned omp (run on the host), as `omfp-<version>-linux-x86_64.flatpak` in `out`
package-flatpak out="dist":
    contrib/release/package-flatpak.sh {{ quote(out) }}

# Static checks: formatting and lints, version pins, traceability, dependencies, the CI workflow
check: lint check-versions spec-trace deny check-workflow

# Mutation testing of the pure crates (TST-005); pass `--in-diff <file>` to limit it to a change
mutants *args:
    cargo mutants --test-tool nextest -p omfp-core -p omfp-protocol -p fake-llm -p xtask {{ args }}

# spec: TST-050 — no surviving mutant in omfp-core and omfp-protocol (exceptions, with a reason: .cargo/mutants.toml);
# the report goes to target/mutants.out
mutants-gate:
    cargo mutants --test-tool nextest -p omfp-core -p omfp-protocol --jobs 2 --output target

# spec: TST-050 — the fast Rust tests (`test`), instrumented, with line coverage gates: ≥ 90% for omfp-core and
# omfp-protocol, ≥ 75% for omfp-daemon; HTML report in target/llvm-cov/html (the view models' gate is in `test-ts`)
cov:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo llvm-cov nextest --no-report --workspace --exclude e2e -E 'not binary(/^contract_/)'
    cargo llvm-cov report --html
    cargo llvm-cov report --json --summary-only --output-path target/llvm-cov/summary.json
    jq -r --argjson min '{"omfp-core": 90, "omfp-protocol": 90, "omfp-daemon": 75}' '
      [.data[0].files[] | (.filename | capture("/crates/(?<c>[^/]+)/src/")?.c) as $c | select($min[$c] != null)
        | {c: $c, n: .summary.lines.count, h: .summary.lines.covered}]
      | group_by(.c)[] | {c: .[0].c, p: ((map(.h) | add) * 100 / (map(.n) | add))}
      | "\(.c): \(.p * 100 | floor / 100)% of lines (min \($min[.c])%)\(if .p < $min[.c] then " FAIL" else "" end)"
    ' target/llvm-cov/summary.json > target/llvm-cov/gate.txt
    cat target/llvm-cov/gate.txt
    ! grep -q FAIL target/llvm-cov/gate.txt

# The recipes `just ci` runs, in order; CI runs each as a job (.github/workflows/ci.yml, `just check-workflow`). The
# fast Rust tests (`test`) run once, instrumented, in `cov`.
ci_recipes := "check test-ts test-contract test-e2e test-desktop package mutants-gate cov"

# spec: TST-113 — everything a change must pass: the recipes of `ci_recipes` in one run, with one web bundle for
# test-e2e and test-desktop (TST-074)
ci: (with-web-bundle quote(just_executable()) + " --justfile " + quote(justfile()) + " " + ci_recipes)
