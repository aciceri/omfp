# ENV-066: the home-manager module evaluated with stand-ins for the home-manager options it sets (`home.packages`,
# `systemd.user.services`), so `just package` catches a broken module without home-manager as a flake input. The
# result is the unit the module asks for, as JSON.
{
  lib,
  pkgs,
  writeText,
  module,
  package,
}:
let
  evaluate =
    settings:
    (lib.evalModules {
      modules = [
        module
        {
          options = {
            home.packages = lib.mkOption { type = lib.types.listOf lib.types.package; };
            systemd.user.services = lib.mkOption { type = lib.types.attrsOf lib.types.anything; };
          };
          config = {
            _module.args.pkgs = pkgs;
            services.omfp = {
              enable = true;
              inherit package;
            }
            // settings;
          };
        }
      ];
    }).config;
  headless = (evaluate { }).systemd.user.services.omfp;
  tray =
    (evaluate {
      tray = true;
      extraArgs = [
        "--bind"
        "127.0.0.1:7421"
      ];
    }).systemd.user.services.omfp;
  omfp = lib.getExe package;
in
assert headless.Service.ExecStart == "${omfp} daemon";
assert headless.Service.Restart == "on-failure";
assert headless.Service.KillMode == "mixed";
assert headless.Install.WantedBy == [ "default.target" ];
assert tray.Service.ExecStart == "${omfp} daemon --tray --bind 127.0.0.1:7421";
assert tray.Unit.PartOf == [ "graphical-session.target" ];
assert tray.Install.WantedBy == [ "graphical-session.target" ];
writeText "omfp-home-manager-unit.json" (builtins.toJSON { inherit headless tray; })
