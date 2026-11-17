/** L1: the `/kit` gallery's parameters (UI-173). */

import { describe, expect, test } from "bun:test";
import { builtinTheme, GALLERY_THEMES, galleryParams, gallerySearch, isGalleryPath } from "../../src/gallery/params.ts";

describe("gallery parameters", () => {
  // spec: UI-173
  test("the page is served at /kit", () => {
    expect(isGalleryPath("/kit")).toBe(true);
    expect(isGalleryPath("/kit/")).toBe(true);
    expect(isGalleryPath("/")).toBe(false);
    expect(isGalleryPath("/kits")).toBe(false);
  });

  // spec: UI-173, UI-797
  test("scheme, theme, skin, density, motion and font size come from the query, with the app's defaults", () => {
    expect(galleryParams("")).toEqual({
      scheme: "system",
      theme: "synth",
      accent: "#007aff",
      skin: "crush",
      density: "compact",
      motion: "full",
      fontSize: 17,
    });
    expect(galleryParams("?scheme=dark&theme=matugen&skin=boring&density=normal&motion=none&fs=15")).toEqual({
      scheme: "dark",
      theme: "matugen",
      accent: "#007aff",
      skin: "boring",
      density: "normal",
      motion: "none",
      fontSize: 15,
    });
  });

  // spec: UI-173
  test("unknown values fall back to the defaults", () => {
    expect(galleryParams("?scheme=sepia&theme=nope&accent=blue&skin=orca&density=huge&motion=fast&fs=abc")).toEqual(
      galleryParams(""),
    );
    expect(galleryParams("?fs=4").fontSize).toBe(17);
  });

  // spec: UI-173, UI-196
  test("the Dux preset fixture is a gallery theme", () => {
    expect(galleryParams("?theme=dux-solarized").theme).toBe("dux-solarized");
  });

  // spec: UI-514, UI-510
  test("every theme of the setting is a gallery theme, then the Dux fixture; `accent` takes the query's accent", () => {
    expect(GALLERY_THEMES).toEqual([
      "synth",
      "catppuccin",
      "gruvbox",
      "tokyo-night",
      "nord",
      "solarized",
      "rose-pine",
      "accent",
      "matugen",
      "dux-solarized",
    ]);
    expect(GALLERY_THEMES.filter((theme) => galleryParams(`?theme=${theme}`).theme !== theme)).toEqual([]);
    expect(galleryParams("?theme=accent&accent=%23FFC600")).toMatchObject({ theme: "accent", accent: "#ffc600" });
    expect([builtinTheme("matugen"), builtinTheme("dux-solarized")]).toEqual(["matugen", undefined]);
  });

  // spec: UI-514
  test("the picker's links change one parameter and keep the others", () => {
    const params = galleryParams("?scheme=dark&theme=nord&density=normal&motion=none&fs=15");
    expect(gallerySearch(params, { theme: "gruvbox" })).toBe(
      "?scheme=dark&theme=gruvbox&skin=crush&density=normal&motion=none&fs=15",
    );
    expect(galleryParams(gallerySearch(params, { scheme: "light" }))).toEqual({ ...params, scheme: "light" });
    expect(gallerySearch(params, { theme: "accent", accent: "#ff5257" })).toBe(
      "?scheme=dark&theme=accent&skin=crush&density=normal&motion=none&fs=15&accent=%23ff5257",
    );
  });

  // spec: UI-797, UI-794
  test("the picker's skin links switch between crush and boring and keep the theme and scheme", () => {
    const params = galleryParams("?scheme=light&theme=gruvbox");
    expect(gallerySearch(params, { skin: "boring" })).toBe(
      "?scheme=light&theme=gruvbox&skin=boring&density=compact&motion=full&fs=17",
    );
    expect(galleryParams(gallerySearch(params, { skin: "boring" }))).toEqual({ ...params, skin: "boring" });
  });
});
