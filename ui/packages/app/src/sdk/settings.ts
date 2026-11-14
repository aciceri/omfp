/**
 * UI settings kept by the daemon (DMN-009, UI-097): `[ui]` density, font size, motion, notifications, xdev notice
 * visibility, fonts and theme (UI-510), and `[keymap]` entries, pushed on `index` so windows apply changes live.
 * `settings.set` merges a patch into the config file.
 */

import type { BuiltinTheme, Density, Motion, NotificationMode, SetSettingsParams, Settings } from "@omfp/protocol";
import { useMemo } from "react";
import { reportError } from "./actions.ts";
import { useSnapshot } from "./hooks.ts";
import { useHost } from "./host.tsx";

/** Without a `[ui]` density: compact (UI-094). */
export const DEFAULT_DENSITY: Density = "compact";
/** Without a `[ui]` font_size: the root font size in px every rem-based token scales with (UI-094, UI-144). */
export const DEFAULT_FONT_SIZE = 17;
/** The accepted `[ui]` font_size values in px, as the daemon checks them. */
export const FONT_SIZE_MIN = 10;
export const FONT_SIZE_MAX = 28;
/** Without a `[ui]` motion: every effect of the motion catalogue (UI-145). */
export const DEFAULT_MOTION: Motion = "full";
/** Without a `[ui]` notifications mode: only while the window has no focus (UI-096). */
export const DEFAULT_NOTIFICATIONS: NotificationMode = "background";
/** Without a `[ui]` theme: Synth, omfp's own palette (UI-139, UI-510). */
export const DEFAULT_THEME: BuiltinTheme = "synth";

/** The daemon's settings (defaults until the `index` snapshot). */
export function useSettings(): Settings {
  return useSnapshot((snapshot) => snapshot.index.settings);
}

/** Sends `patch` with `settings.set`; a failure lands in the frame's error list. */
export function useSetSettings(): (patch: SetSettingsParams) => Promise<void> {
  const host = useHost();
  return useMemo(
    () => async (patch: SetSettingsParams) => {
      try {
        await host.client.call("settings.set", patch);
      } catch (error) {
        reportError(host, error);
      }
    },
    [host],
  );
}
