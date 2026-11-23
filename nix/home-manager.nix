# ENV-066: the home-manager module (`homeManagerModules.default`): `omfp daemon` as the systemd user service
# `omfp.service`, with the settings of the unit the packages ship (ENV-067, contrib/systemd/omfp.service). Every new
# package changes the unit's ExecStart, so home-manager's switch restarts the daemon onto it.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.omfp;
in
{
  options.services.omfp = {
    enable = lib.mkEnableOption "the omfp daemon as a systemd user service";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.omfp;
      defaultText = lib.literalExpression "omfp.packages.\${system}.omfp";
      description = "The omfp package; `omfp-headless` is the daemon without the desktop window and the tray.";
    };
    tray = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Show the daemon in the system tray (`omfp daemon --tray`, the `omfp` package): the service then starts and
        stops with the graphical session.
      '';
    };
    extraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [
        "--bind"
        "127.0.0.1:7421"
      ];
      description = "More arguments of `omfp daemon` (the config file's `[daemon]` table is the other way).";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];
    systemd.user.services.omfp = {
      Unit = {
        Description = "omfp daemon";
        Documentation = "https://github.com/aciceri/omfp";
      }
      // lib.optionalAttrs cfg.tray {
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
      };
      Service = {
        ExecStart = lib.escapeShellArgs (
          [
            (lib.getExe cfg.package)
            "daemon"
          ]
          ++ lib.optional cfg.tray "--tray"
          ++ cfg.extraArgs
        );
        Restart = "on-failure";
        RestartSec = 2;
        # SIGTERM to the daemon alone: it stops its omp processes and jobs itself, marking the turns it cuts.
        KillMode = "mixed";
      };
      Install.WantedBy = [ (if cfg.tray then "graphical-session.target" else "default.target") ];
    };
  };
}
