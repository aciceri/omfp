# ENV-030..034, ENV-041: the installable `omfp`. `desktop = true` builds the `desktop` feature: on Linux Tauri 2 on
# WebKitGTK 4.1 and the tray over gio, wrapped by wrapGAppsHook3; on macOS Tauri 2 on the system's WKWebView and the
# menu bar item, plus `Applications/omfp.app`. `desktop = false` is the lean daemon-only build
# (`packages.omfp-headless`). Both ship the release web bundle as `bin/web` (DMN-008) and run the pinned omp from the
# wrapper's PATH (DMN-006).
{
  lib,
  stdenv,
  rustPlatform,
  makeWrapper,
  makeBinaryWrapper,
  pkg-config,
  wrapGAppsHook3,
  copyDesktopItems,
  makeDesktopItem,
  webkitgtk_4_1,
  libsoup_3,
  glib-networking,
  gsettings-desktop-schemas,
  gstPlugins,
  gitMinimal,
  libicns,
  rcodesign,
  sqlite,
  omp,
  web,
  desktop ? true,
}:
let
  # Linux: Tauri on WebKitGTK, wrapped by wrapGAppsHook3. macOS (ENV-041): Tauri on the system's WKWebView (nothing
  # to link from nixpkgs beyond the SDK), plus `Applications/omfp.app` around the wrapped binary.
  linuxDesktop = desktop && stdenv.hostPlatform.isLinux;
  darwinApp = desktop && stdenv.hostPlatform.isDarwin;
  # The Rust workspace only: the UI, docs and fixtures do not change the binary.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../xtask
      ../e2e
    ];
  };

  # The pinned omp first (DMN-001 refuses another version); git (without perl and python) last, so the user's own git
  # wins.
  pathArgs = [
    "--prefix"
    "PATH"
    ":"
    (lib.makeBinPath [ omp ])
    "--suffix"
    "PATH"
    ":"
    (lib.makeBinPath [ gitMinimal ])
  ];
  gstArgs = [
    "--prefix"
    "GST_PLUGIN_SYSTEM_PATH_1_0"
    ":"
    (lib.makeSearchPath "lib/gstreamer-1.0" gstPlugins)
  ];
in
rustPlatform.buildRustPackage {
  pname = if desktop then "omfp" else "omfp-headless";
  inherit (web) version;
  inherit src;

  cargoLock = {
    lockFile = ../Cargo.lock;
    # The omp-rpc git dependency (the oh-my-pi tag of ENV-010); changes with every omp upgrade (ENV-020).
    outputHashes."omp-rpc-0.1.0" = "sha256-RPSjNj/EOGUsIIZvUCtaPvhRLzx6QkExzz8eZGZPrxQ=";
  };

  cargoBuildFlags = [
    "--package"
    "omfp"
  ];
  buildFeatures = lib.optional desktop "desktop";

  nativeBuildInputs = [
    pkg-config
  ]
  ++ (
    if linuxDesktop then
      [
        wrapGAppsHook3
        copyDesktopItems
      ]
    # A compiled wrapper: the app bundle's executable must be a Mach-O, not a script.
    else if stdenv.hostPlatform.isDarwin then
      [ makeBinaryWrapper ]
    else
      [ makeWrapper ]
  )
  # The bundle's icon from the 512 px PNG, and its ad-hoc signature (ENV-041).
  ++ lib.optionals darwinApp [
    libicns
    rcodesign
  ];

  # nixpkgs' cargo setup hook sets LIBSQLITE3_SYS_USE_PKG_CONFIG=1: rusqlite links nixpkgs' sqlite instead of its
  # bundled copy. GTK3, GLib and cairo come through webkitgtk's .pc files; glib-networking is TLS for `omfp desktop
  # --connect https://…` (GIO_EXTRA_MODULES, set by the wrapper), gsettings-desktop-schemas the GTK settings schemas.
  # WebKitGTK delegates HTML media to GStreamer; the explicit plug-ins provide the demuxers and codecs in Nix's
  # isolated runtime.
  buildInputs = [
    sqlite
  ]
  ++ lib.optionals linuxDesktop (
    [
      webkitgtk_4_1
      libsoup_3
      glib-networking
      gsettings-desktop-schemas
    ]
    ++ gstPlugins
  );

  # `just ci` runs the tests (several need omp, a browser or a display).
  doCheck = false;

  desktopItems = lib.optionals linuxDesktop [
    (makeDesktopItem {
      name = "omfp";
      desktopName = "omfp";
      comment = "Web and desktop GUI for the omp coding agent";
      exec = "omfp desktop";
      icon = "omfp";
      categories = [ "Development" ];
    })
  ];

  # DMN-008: the bundle is `web/` next to the executable; the wrappers keep `bin/` as the real binary's directory
  # (`bin/.omfp-wrapped`). A symlink, which the wrappers do not try to wrap.
  postInstall = ''
    mkdir -p "$out/share/omfp"
    ln -s ${web} "$out/share/omfp/web"
    ln -s ../share/omfp/web "$out/bin/web"
  ''
  + lib.optionalString linuxDesktop ''
    install -Dm644 crates/omfp/icons/icon.png "$out/share/icons/hicolor/512x512/apps/omfp.png"
  '';

  preFixup = lib.optionalString linuxDesktop ''
    gappsWrapperArgs+=(${lib.escapeShellArgs (pathArgs ++ gstArgs)})
  '';

  postFixup =
    lib.optionalString (!linuxDesktop) ''
      wrapProgram "$out/bin/omfp" ${lib.escapeShellArgs pathArgs}
    ''
    # ENV-041: `omfp.app` opens a window (`omfp desktop`): its executable is a binary wrapper of `bin/.omfp-wrapped`,
    # whose directory holds `web/` (DMN-008), with the same PATH. The Info.plist is the one build.rs embeds in the
    # binary (bundle id, NSMicrophoneUsageDescription). The ad-hoc signature of the whole bundle comes last: macOS
    # then knows the app as `dev.omfp.desktop` (microphone permission, notifications).
    + lib.optionalString darwinApp ''
      app="$out/Applications/omfp.app/Contents"
      mkdir -p "$app/MacOS" "$app/Resources"
      substitute crates/omfp/Info.plist "$app/Info.plist" --replace-fail @VERSION@ "$version"
      png2icns "$app/Resources/omfp.icns" crates/omfp/icons/icon.png
      makeBinaryWrapper "$out/bin/.omfp-wrapped" "$app/MacOS/omfp" ${lib.escapeShellArgs pathArgs} \
        --add-flags desktop
      rcodesign sign "$out/Applications/omfp.app"
    '';

  meta = {
    description = "Web and desktop GUI for the omp coding agent (oh-my-pi)";
    homepage = "https://github.com/aciceri/omfp";
    changelog = "https://github.com/aciceri/omfp/blob/master/CHANGELOG.md";
    license = lib.licenses.gpl3Plus;
    mainProgram = "omfp";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
