/**
 * The `/kit` gallery (UI-173): every kit component of 09-design in every state it lists, side by side, with sizes and
 * both densities, the glyph vocabulary and a token sheet with measured contrast. Hover, pressed and focus are forced
 * with the kit's state classes (`is-hover`, `is-pressed`, `is-focus`), so the page is deterministic; the query picks
 * the scheme, theme, skin, density, motion and root size (params.ts), and the header's picker links every theme, scheme
 * and skin (UI-514, UI-797). The screenshot test (UI-174) waits for `[data-ready=true]` and reads the contrast rows.
 */
import "./gallery.css";
import { useEffect, useState } from "react";
import { SKINS } from "../frame/set.ts";
import type { Adjustment } from "../theme/contrast.ts";
import { applyContrastGuard, cssColorReader } from "../theme/guard.ts";
import { applyAppearance, applyUserTheme, SYSTEM_ACCENT } from "../theme/theme.ts";
import { BorderBoxes, Cards, CodeDemo, Diffs, Interactions, ToolCards } from "./cards.tsx";
import { Composers, LiveVoice, Transcript } from "./chat.tsx";
import { Buttons, Choices, Dialogs, Fields, MenuDemo, PathFields, Selects, TabsDemo } from "./controls.tsx";
import duxTheme from "./dux-solarized.css" with { type: "text" };
import { Empties, Logs, Marks, ProgressDemo, Toasts } from "./feedback.tsx";
import { FocusStates } from "./focus.tsx";
import forestTheme from "./matugen-forest.css" with { type: "text" };
import {
  builtinTheme,
  GALLERY_THEMES,
  type GalleryParams,
  type GalleryTheme,
  galleryParams,
  gallerySearch,
} from "./params.ts";
import { HelpPanels, LeftColumn, ModeLines, ResponsiveParts } from "./shell.tsx";
import { DensityStrip, Glyphs, Logo, TokenSheet } from "./tokens.tsx";

/** The fixture user themes of `?theme=` (UI-173, UI-514) over Synth: matugen's output as `matugen`'s, the Dux preset. */
const FIXTURE_CSS: ReadonlyMap<GalleryTheme, string> = new Map<GalleryTheme, string>([
  ["matugen", forestTheme as string],
  ["dux-solarized", duxTheme as string],
]);

/**
 * Puts the query's appearance and theme on the document — a built-in theme (`accent` from the query's accent, as the
 * desktop window gives the system's, UI-512), or Synth under a fixture user theme (`matugen`'s or the Dux preset),
 * which replaces the user theme — and runs the contrast guard (UI-196), before the first render, so what the page
 * measures is in its final colors. Returns what the guard overrode.
 */
function applyGalleryParams(params: GalleryParams): Adjustment[] {
  const root = document.documentElement;
  const builtin = builtinTheme(params.theme);
  root.style.setProperty(SYSTEM_ACCENT, params.accent);
  applyAppearance(root, {
    scheme: params.scheme,
    theme: builtin ?? "synth",
    skin: params.skin,
    density: params.density,
    fontSize: params.fontSize,
    motion: params.motion,
    fonts: null,
  });
  applyUserTheme(document, FIXTURE_CSS.get(params.theme) ?? null);
  document.title = "omfp kit";
  return applyContrastGuard(root, cssColorReader(document));
}

/**
 * The header's picker (UI-514, UI-797): every theme, then both schemes, then both skins, as links to this page with the
 * other parameters kept.
 */
function ThemePicker({ params }: { params: GalleryParams }) {
  const link = (key: string, label: string, change: Partial<GalleryParams>, current: boolean) => (
    <a
      key={key}
      className="omfp-gallery__pick"
      href={gallerySearch(params, change)}
      aria-current={current ? "page" : undefined}
      data-testid="theme-pick"
    >
      {label}
    </a>
  );
  return (
    <nav className="omfp-gallery__picker" aria-label="Themes, schemes and skins">
      {GALLERY_THEMES.map((theme) => link(theme, theme, { theme }, theme === params.theme))}
      {(["light", "dark"] as const).map((scheme) => link(scheme, scheme, { scheme }, scheme === params.scheme))}
      {SKINS.map((skin) => link(`skin-${skin}`, skin, { skin }, skin === params.skin))}
    </nav>
  );
}

/** Two frames: what the last state change shows has been laid out and painted. */
function afterPaint(): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  return promise;
}

/** The `/kit` page. */
export function Gallery({ search }: { search: string }) {
  const [{ params, guard }] = useState(() => {
    const parsed = galleryParams(search);
    return { params: parsed, guard: applyGalleryParams(parsed) };
  });
  const [measured, setMeasured] = useState(false);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    if (!measured) {
      return;
    }
    let live = true;
    void (async () => {
      await document.fonts.ready;
      await afterPaint();
      await document.fonts.ready;
      if (live) {
        setReady(true);
      }
    })();
    return () => {
      live = false;
    };
  }, [measured]);
  return (
    <main className="omfp-gallery" data-testid="kit-gallery" data-ready={ready}>
      <header className="omfp-gallery__top">
        <h1 className="omfp-gallery__page-title omfp-grad-text omfp-grad-flow">◆ omfp kit</h1>
        <span className="omfp-gallery__note">
          {params.skin === "boring" ? "Boring" : "Crush"} · {params.theme}
          {params.theme === "accent" ? ` ${params.accent}` : ""} · {params.scheme} · {params.density} · motion{" "}
          {params.motion} · {params.fontSize}px
        </span>
        <ThemePicker params={params} />
      </header>
      <div className="omfp-gallery__sections">
        <Buttons />
        <Logo />
        <Glyphs />
        <Fields />
        <PathFields />
        <Selects />
        <Choices />
        <TabsDemo />
        <MenuDemo />
        <Marks />
        <Dialogs />
        <ProgressDemo />
        <Logs />
        <Toasts />
        <ModeLines />
        <HelpPanels />
        <Empties />
        <Cards />
        <ToolCards />
        <BorderBoxes />
        <Interactions />
        <Diffs />
        <CodeDemo />
        <LeftColumn />
        <FocusStates />
        <ResponsiveParts />
        <Transcript />
        <Composers />
        <LiveVoice />
        <DensityStrip />
        <TokenSheet guard={guard} onMeasured={() => setMeasured(true)} />
      </div>
    </main>
  );
}
