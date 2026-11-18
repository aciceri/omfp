import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { cssFiles, findRule, KIT_SRC, parseCss, stripComments } from "./support/css.ts";
import { RETIRED_TOKEN } from "./support/tokens.ts";

/** CSS named and system colors (`transparent` and `currentColor` are allowed: they carry no palette choice). */
const NAMED_COLORS = new Set(
  `aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood
  cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray
  darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen
  darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue
  firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew
  hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan
  lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray
  lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue
  mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred
  midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid
  palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple
  rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue
  slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow
  yellowgreen accentcolor accentcolortext activetext buttonborder buttonface buttontext canvas canvastext field
  fieldtext graytext highlight highlighttext linktext mark marktext selecteditem selecteditemtext visitedtext`.split(
    /\s+/,
  ),
);

const HEX_COLOR = /#[0-9a-f]{3,8}\b/gi;
const COLOR_FUNCTION = /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/gi;
const VAR_REF = /var\(\s*(--[\w-]+)/g;

/** The theme files hold the palettes (UI-139, UI-511): the only ones allowed colour literals. */
const THEME_FILES = ["tokens.css", "themes.css"];
const styleFiles = cssFiles(KIT_SRC).filter((path) => !THEME_FILES.includes(basename(path)));
const APP_SRC = join(KIT_SRC, "..", "..", "app", "src");
const tokensCss = readFileSync(join(KIT_SRC, "tokens.css"), "utf8");
const kitRules = parseCss(readFileSync(join(KIT_SRC, "kit.css"), "utf8"));
const LAYER = "@layer omfp-kit";
const FORCED_COLORS = "@media (forced-colors: active)";

/**
 * Color literals of one stylesheet: hex colors, color functions and named colors in declaration values. System colors
 * are what forced colors (UI-132) ask for, so rules under `@media (forced-colors: active)` may name them.
 */
function colorLiterals(css: string): string[] {
  const text = stripComments(css);
  const literals = [...(text.match(HEX_COLOR) ?? []), ...(text.match(COLOR_FUNCTION) ?? [])];
  for (const rule of parseCss(css).filter((rule) => !rule.at.includes(FORCED_COLORS))) {
    for (const [property, value] of rule.declarations) {
      const words = value.replace(VAR_REF, "var(").match(/-?[a-z][\w-]*/gi) ?? [];
      literals.push(...words.filter((word) => NAMED_COLORS.has(word.toLowerCase())).map((w) => `${property}: ${w}`));
    }
  }
  return literals;
}

function definedTokens(css: string): Set<string> {
  return new Set(parseCss(css).flatMap((rule) => [...rule.declarations.keys()].filter((k) => k.startsWith("--omfp-"))));
}

describe("kit styles", () => {
  test("the kit ships its component stylesheet", () => {
    expect(styleFiles.map((path) => basename(path))).toContain("kit.css");
  });

  // spec: UI-091
  test.each(styleFiles.map((path) => [basename(path), path] as const))("%s has no color literals", (_name, path) => {
    expect(colorLiterals(readFileSync(path, "utf8"))).toEqual([]);
  });

  // spec: UI-091, UI-132
  test("the scanner catches every kind of color literal, and lets forced colors name system colors", () => {
    const sample = `.a { color: #fff; background: rgb(0 0 0); border-color: hsl(1 2% 3%); fill: RebeccaPurple; }
      .b { outline-color: Canvas; color: color-mix(in srgb, var(--omfp-primary) 50%, transparent); }
      @media (forced-colors: active) { .c { outline-color: Highlight; } }`;
    expect(colorLiterals(sample)).toEqual(["#fff", "rgb(", "hsl(", "fill: RebeccaPurple", "outline-color: Canvas"]);
  });

  // spec: UI-091, UI-795
  test.each(["kit.css", "boring.css"])("%s only uses tokens that tokens.css defines", (name) => {
    const defined = definedTokens(tokensCss);
    const css = stripComments(readFileSync(join(KIT_SRC, name), "utf8"));
    const used = new Set([...css.matchAll(VAR_REF)].map((match) => match[1] ?? ""));
    expect([...used].filter((name) => name.startsWith("--omfp-") && !defined.has(name))).toEqual([]);
  });

  // spec: UI-125
  test("the app's stylesheets only use tokens that tokens.css defines", () => {
    const defined = definedTokens(tokensCss);
    const undefinedTokens = cssFiles(APP_SRC).flatMap((path) =>
      [...stripComments(readFileSync(path, "utf8")).matchAll(VAR_REF)]
        .map((match) => match[1] ?? "")
        .filter((name) => name.startsWith("--omfp-") && !defined.has(name))
        .map((name) => `${basename(path)}: ${name}`),
    );
    expect(undefinedTokens).toEqual([]);
  });

  // spec: UI-125
  test("the earlier size, radius, line-height and spacing tokens are gone everywhere, with no aliases", () => {
    const sources = [KIT_SRC, APP_SRC].flatMap((dir) =>
      readdirSync(dir, { recursive: true, encoding: "utf8" })
        .filter((name) => /\.(?:css|tsx?)$/.test(name))
        .map((name) => join(dir, name)),
    );
    expect(sources.filter((path) => RETIRED_TOKEN.test(readFileSync(path, "utf8")))).toEqual([]);
  });

  // spec: UI-125, UI-127, UI-795
  test("paddings, gaps and margins use the spacing steps or the text spacing tokens, never a length literal", () => {
    const SPACING = /(?:^|[;{\s])((?:padding|margin|gap|row-gap|column-gap)[\w-]*)\s*:\s*([^;{}]+)/g;
    // Hairlines (`1px`, `2px`, UI-125) stay literal; so do viewport-relative offsets.
    const LENGTH = /(?<![\w-])\d*\.?\d+(?:em|rem|ch|ex|px)\b/g;
    const offending = [join(KIT_SRC, "kit.css"), join(KIT_SRC, "boring.css"), ...cssFiles(APP_SRC)].flatMap((path) =>
      [...stripComments(readFileSync(path, "utf8")).matchAll(SPACING)]
        .filter((match) => (match[2] ?? "").match(LENGTH)?.some((length) => length !== "1px" && length !== "2px"))
        .map((match) => `${basename(path)}: ${match[1]}: ${(match[2] ?? "").trim()}`),
    );
    expect(offending).toEqual([]);
  });

  // spec: UI-148
  test("key chips: code family at fs-2xs, optically centred on the text baseline, tertiary on surface-container", () => {
    const chip = findRule(kitRules, [LAYER], ".omfp-kbd");
    expect(
      ["font-family", "font-size", "text-box", "color", "background"].map((name) => chip.declarations.get(name)),
    ).toEqual([
      "var(--omfp-font-code)",
      "var(--omfp-fs-2xs)",
      "trim-both ex alphabetic",
      "var(--omfp-tertiary)",
      "var(--omfp-kbd-bg)",
    ]);
    const inButton = findRule(kitRules, [LAYER], ".omfp-button .omfp-kbd");
    expect(Object.fromEntries(inButton.declarations)).toEqual({ color: "inherit", background: "transparent" });
  });

  // spec: UI-369
  test("Markdown tables: cells wrap only between words, a wide table scrolls sideways in its region", () => {
    // Prose breaks anywhere (`overflow-wrap: anywhere`), which makes every column's minimum one character wide.
    const cells = findRule(kitRules, [LAYER], ".omfp-markdown :is(th, td)");
    expect(cells.declarations.get("overflow-wrap")).toBe("break-word");
    expect(cells.declarations.get("word-break")).toBe("normal");
    const region = findRule(kitRules, [LAYER], ".omfp-markdown__table");
    expect(region.declarations.get("overflow-x")).toBe("auto");
    expect(region.declarations.get("max-inline-size")).toBe("100%");
  });

  // spec: UI-168, UI-408
  test("the `:` menu: a long command name keeps its width and pushes the description, which ellipsizes", () => {
    // A flex item shrinks below its `min-inline-size` content when allowed to: the name would overprint the description.
    const command = findRule(kitRules, [LAYER], ".omfp-cmdmenu__command");
    expect(command.declarations.get("flex")).toBe("none");
    expect(command.declarations.get("white-space")).toBe("nowrap");
    const description = findRule(kitRules, [LAYER], ".omfp-cmdmenu__description");
    expect(description.declarations.get("min-inline-size")).toBe("0");
    expect(description.declarations.get("text-overflow")).toBe("ellipsis");
  });

  // spec: UI-320
  test("the which-key panel scrolls its groups only, title and `esc` row pinned, edges with more keys fading", () => {
    const panel = findRule(kitRules, [LAYER], ".omfp-whichkey");
    expect(["display", "flex-direction", "overflow"].map((name) => panel.declarations.get(name))).toEqual([
      "flex",
      "column",
      "hidden",
    ]);
    const pinned = findRule(kitRules, [LAYER], ".omfp-whichkey__title, .omfp-whichkey__foot");
    expect(pinned.declarations.get("flex")).toBe("none");
    const groups = findRule(kitRules, [LAYER], ".omfp-whichkey__groups");
    expect(["min-block-size", "overflow-y"].map((name) => groups.declarations.get(name))).toEqual(["0", "auto"]);
    expect(groups.declarations.get("mask-image")).toBe(
      "linear-gradient( to bottom, transparent, var(--omfp-on-surface) var(--whichkey-fade-start), var(--omfp-on-surface) calc(100% - var(--whichkey-fade-end)), transparent )",
    );
    const fade = (edge: string) =>
      findRule(kitRules, [LAYER], `.omfp-whichkey__groups[data-more-${edge}]`).declarations.get(
        `--whichkey-fade-${edge}`,
      );
    expect([fade("start"), fade("end")]).toEqual(["var(--omfp-s5)", "var(--omfp-s5)"]);
  });

  // spec: UI-486
  test("room where there is room: the `:` menu up to 48rem, which-key descriptions on up to three lines, a wider left column", () => {
    expect(findRule(kitRules, [LAYER], ".omfp-cmdmenu").declarations.get("inline-size")).toBe("min(48rem, 70%)");
    const which = findRule(kitRules, [LAYER], ".omfp-whichkey__description");
    expect([which.declarations.get("-webkit-line-clamp"), which.declarations.get("white-space")]).toEqual([
      "3",
      undefined,
    ]);
    const appRules = parseCss(readFileSync(join(APP_SRC, "styles.css"), "utf8"));
    const left = findRule(appRules, ["@layer omfp-widgets"], ".omfp-frame__left");
    expect(left.declarations.get("flex")).toBe("0 0 clamp(var(--omfp-side-w), 20vw, 26rem)");
  });

  // spec: UI-485
  test("the transcript and the composer's area keep the same scrollbar gutter, so their columns line up", () => {
    const chatRules = parseCss(readFileSync(join(APP_SRC, "widgets", "chat", "chat.css"), "utf8"));
    const gutter = (selector: string) =>
      findRule(chatRules, ["@layer omfp-widgets"], selector).declarations.get("scrollbar-gutter");
    expect([gutter(".omfp-transcript"), gutter(".omfp-composer-area")]).toEqual(["stable", "stable"]);
    expect(findRule(chatRules, ["@layer omfp-widgets"], ".omfp-composer-area").declarations.get("overflow")).toBe(
      "hidden",
    );
  });

  // spec: UI-461, UI-462, UI-466
  test("the sheet covers the window, as tall as the visual viewport on a phone; its rows and close button are touch targets", () => {
    const overlay = findRule(kitRules, [LAYER], ".omfp-sheet-overlay");
    expect(
      ["position", "inset-block-start", "inset-inline", "block-size"].map((name) => overlay.declarations.get(name)),
    ).toEqual(["fixed", "0", "0", "var(--vvh, 100%)"]);
    expect(findRule(kitRules, [LAYER], ".omfp-sheet__row").declarations.get("min-block-size")).toBe(
      "var(--omfp-touch)",
    );
    const close = findRule(kitRules, [LAYER], ".omfp-sheet__close.omfp-icon-button");
    expect([close.declarations.get("min-inline-size"), close.declarations.get("min-block-size")]).toEqual([
      "var(--omfp-touch)",
      "var(--omfp-touch)",
    ]);
  });

  // spec: UI-465
  test("on a touch phone the mode line has no mode pill; a narrow window with a mouse keeps it", () => {
    const appRules = parseCss(readFileSync(join(APP_SRC, "styles.css"), "utf8"));
    const hidden = findRule(
      appRules,
      ["@layer omfp-widgets", "@media (pointer: coarse)"],
      '.omfp-app[data-layout="mobile"] .omfp-mode-pill',
    );
    expect(Object.fromEntries(hidden.declarations)).toEqual({ display: "none" });
  });

  // spec: UI-455
  test("a running boxed card's glow stays below its body's top edge, off the border line around the label", () => {
    const running = '.omfp-tool.omfp-bbox[data-status="running"] > .omfp-tool__body';
    const body = findRule(kitRules, [LAYER], `:root[data-motion="full"] ${running}`);
    expect(body.declarations.get("animation")).toBe("omfp-breathe 1.6s ease-in-out infinite");
    // Clipped at the top edge only: the glow (a 1rem blur) still shows on the sides and below.
    expect(body.declarations.get("clip-path")).toBe("inset(0 -1.5rem -1.5rem)");
  });

  // spec: UI-967
  test("visually hidden text is anchored at its containing block's corner, so it never lengthens the page", () => {
    const hidden = findRule(kitRules, [LAYER], ".omfp-visually-hidden");
    expect(hidden.declarations.get("position")).toBe("absolute");
    expect(hidden.declarations.get("inset-block-start")).toBe("0");
    expect(hidden.declarations.get("inset-inline-start")).toBe("0");
    expect(hidden.declarations.get("clip-path")).toBe("inset(50%)");
  });

  // spec: UI-118
  test("CodeMirror scrolls in a layout root sized by its container; omfp:changes' text-sized diff opts out", () => {
    const view = findRule(kitRules, [LAYER], ".omfp-code-editor__view, .omfp-code-diff__view");
    expect(view.declarations.get("contain")).toBe("strict");
    expect(view.declarations.get("overflow")).toBe("auto");
    const appRules = parseCss(readFileSync(join(APP_SRC, "styles.css"), "utf8"));
    const changes = findRule(appRules, ["@layer omfp-widgets"], ".omfp-changes__merge .omfp-code-diff__view");
    expect(changes.declarations.get("contain")).toBe("none");
  });

  // spec: UI-145, UI-124
  test("motion none stops every animation and transition and makes scrolling instant; the effective level is data-motion", () => {
    const none = findRule(
      kitRules,
      [LAYER],
      ':root[data-motion="none"], :root[data-motion="none"] *, :root[data-motion="none"] *::before, :root[data-motion="none"] *::after',
    );
    expect(Object.fromEntries(none.declarations)).toEqual({
      animation: "none !important",
      transition: "none !important",
      "scroll-behavior": "auto !important",
    });
  });

  // spec: UI-122, UI-216
  test("running marks: activity frames by default, omp's status frames on tool cards; both rest on their first frame", () => {
    const frames = (name: string) =>
      kitRules
        .filter((rule) => rule.at[1] === `@keyframes ${name}`)
        .map((rule) => [rule.selector, rule.declarations.get("content")]);
    expect(frames("omfp-braille")).toEqual([..."⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"].map((glyph, index) => [`${index * 10}%`, `"${glyph}"`]));
    expect(frames("omfp-status")).toEqual([..."⣾⣽⣻⢿⡿⣟⣯⣷"].map((glyph, index) => [`${index * 12.5}%`, `"${glyph}"`]));
    const activity = findRule(kitRules, [LAYER], '.omfp-state-mark[data-state="running"]::before');
    expect(activity.declarations.get("content")).toBe('"⠋"');
    expect(activity.declarations.get("animation")).toBe("omfp-braille 0.8s steps(1) infinite");
    const status = findRule(kitRules, [LAYER], '.omfp-state-mark[data-state="running"][data-spinner="status"]::before');
    expect(status.declarations.get("content")).toBe('"⣾"');
    expect(status.declarations.get("animation")).toBe("omfp-status 0.64s steps(1) infinite");
  });

  // spec: UI-796, UI-122, UI-187
  test("Crush's decorative glyphs are generated by kit.css, never written in the kit's markup", () => {
    const content = (selector: string) => findRule(kitRules, [LAYER], selector).declarations.get("content");
    expect(
      content(
        ".omfp-card__chevron::before, .omfp-disclosure__chevron::before, .omfp-wsection__glyph::before, .omfp-toggle-row__glyph::before, .omfp-reasoning__glyph::before",
      ),
    ).toBe('"▸"');
    expect(
      content(
        '.omfp-card[data-expanded="true"] .omfp-card__chevron::before, .omfp-disclosure[data-expanded] .omfp-disclosure__chevron::before, [aria-expanded="true"] > :is(.omfp-wsection__glyph, .omfp-toggle-row__glyph, .omfp-reasoning__glyph)::before',
      ),
    ).toBe('"▾"');
    expect(
      [".omfp-cmdmenu__title::before", ".omfp-whichkey__title::before", ".omfp-popdialog__title::before"].map(content),
    ).toEqual(['"╭─ "', '"╭─ "', '"╭─ "']);
    expect(
      [".omfp-msg > .omfp-msg__name::before", '.omfp-msg[data-author="assistant"] > .omfp-msg__name::before'].map(
        content,
      ),
    ).toEqual(['"▸ "', '"◆ "']);
    const written = readdirSync(KIT_SRC, { recursive: true, encoding: "utf8" })
      .filter((name) => name.endsWith(".tsx"))
      .filter((name) =>
        /[▸▾╭⠋⣾]/.test(stripComments(readFileSync(join(KIT_SRC, name), "utf8")).replace(/^\s*\/\/.*$|^\s*\*.*$/gm, "")),
      );
    expect(written).toEqual([]);
  });

  // spec: UI-094, UI-121
  test("the prose family only in prose; chrome in the ui family (which is the code family), code in code", () => {
    const usages = kitRules.flatMap((rule) =>
      [...rule.declarations]
        .filter(([property]) => property === "font-family" || property === "font")
        .map(([, value]) => ({ selector: rule.selector, value })),
    );
    // Code (UI-121): code blocks, inline code, logs, key chips, diffs, mono fields (paths, commands), the command
    // line, tool arguments, ask option labels and completed paths.
    const misplaced = usages.filter(({ selector, value }) => {
      if (value.includes("--omfp-font-prose")) return !selector.includes(".omfp-prose");
      if (value.includes("--omfp-font-code"))
        return !/code|log|kbd|pre|diff|mono|cmd|__arg|option-text__label|path-field__name/.test(selector);
      return !(value.includes("--omfp-font-ui") || value === "inherit");
    });
    expect(misplaced).toEqual([]);
    expect(usages.some(({ value }) => value.includes("--omfp-font-prose"))).toBe(true);
    expect(usages.some(({ value }) => value.includes("--omfp-font-code"))).toBe(true);
  });

  // spec: UI-091
  test("component styles are layered after the tokens", () => {
    const css = stripComments(readFileSync(join(KIT_SRC, "kit.css"), "utf8"));
    expect(css).toContain("@layer omfp-tokens, omfp-kit;");
    expect(kitRules.filter((rule) => rule.at[0] !== "@layer omfp-kit")).toEqual([]);
  });

  // spec: UI-123
  test("gradients only where listed", () => {
    const GRADIENT = /var\(--omfp-grad(?:-loop)?\)/;
    const ALLOWED =
      /grad-text|grad-flow|data-variant="primary"|tabs__tab\[data-selected\]::after|mode-pill|cmd__prompt|omfp-caret|progress__fill|gauge__fill|voice-meter__fill|compose\[data-mode="insert"\]|tool\[data-status="running"\]|splitter::after|areabar__item\[aria-pressed="true"\]::before|@keyframes/;
    const misplaced = kitRules
      .filter((rule) => [...rule.declarations.values()].some((value) => GRADIENT.test(value)))
      .map((rule) => rule.selector)
      .filter((selector) => !ALLOWED.test(selector));
    expect(misplaced).toEqual([]);
  });

  // spec: UI-338, UI-171
  test("thinking text is light secondary text: upright UI family at fs-sm, regular, muted, with its rule", () => {
    const thinking = findRule(kitRules, [LAYER], ".omfp-reasoning__text").declarations;
    expect(
      ["font-family", "font-size", "font-style", "font-weight", "color"].map((name) => thinking.get(name)),
    ).toEqual(["var(--omfp-font-ui)", "var(--omfp-fs-sm)", "normal", "400", "var(--omfp-on-surface-variant)"]);
    expect(thinking.get("border-inline-start")).toContain("var(--omfp-rule)");
    expect(kitRules.filter((rule) => rule.selector.includes("omfp-reasoning__label")).length).toBe(0);
  });

  // spec: UI-138
  test("diff lines tint with ok and error; syntax strings are ok, numbers warn, comments muted italic", () => {
    for (const [kind, role] of [
      ["add", "--omfp-ok"],
      ["del", "--omfp-error"],
    ]) {
      const line = findRule(kitRules, [LAYER], `.omfp-diff__line[data-kind="${kind}"]`).declarations;
      expect([line.get("color"), line.get("background")].map((value) => value?.includes(`var(${role})`))).toEqual([
        true,
        true,
      ]);
    }
    const color = (selector: string) => findRule(kitRules, [LAYER], selector).declarations.get("color");
    expect(
      color(".omfp-code :is(.hljs-string, .hljs-regexp, .hljs-subst, .hljs-template-variable, .hljs-addition)"),
    ).toBe("var(--omfp-ok)");
    expect(
      color(
        ".omfp-code :is(.hljs-number, .hljs-literal, .hljs-symbol, .hljs-bullet, .hljs-variable.constant_, .hljs-meta)",
      ),
    ).toBe("var(--omfp-warn)");
    expect(color(".omfp-code :is(.hljs-comment, .hljs-quote)")).toBe("var(--omfp-on-surface-variant)");
    expect(findRule(kitRules, [LAYER], "::selection").declarations.get("background")).toBe(
      "color-mix(in oklab, var(--omfp-primary) 28%, transparent)",
    );
  });

  // spec: UI-447, UI-163
  test("todo lists style omp's statuses, in kit.css alone", () => {
    const rule = (status: string, part = "") =>
      Object.fromEntries(findRule(kitRules, [LAYER], `.omfp-todo__item[data-status="${status}"]${part}`).declarations);
    expect(rule("completed", " .omfp-todo__mark")).toEqual({ color: "var(--omfp-ok)" });
    expect(rule("in_progress")).toEqual({ color: "var(--omfp-primary)" });
    expect(rule("in_progress", " .omfp-todo__mark")).toEqual({ color: "var(--omfp-primary)" });
    expect(rule("abandoned", " .omfp-todo__text")).toEqual({ "text-decoration": "line-through" });
    const todo = /\.omfp-todo(?:__|(?![\w-]))/;
    const elsewhere = [
      ...kitRules.filter((each) => /data-status="(?:done|active)"/.test(each.selector)),
      ...cssFiles(APP_SRC).flatMap((path) => parseCss(readFileSync(path, "utf8"))),
    ].filter((each) => todo.test(each.selector));
    expect(elsewhere.map((each) => each.selector)).toEqual([]);
  });

  // spec: UI-147, UI-146
  test("the stream caret, a StreamCaret or trailing streamed Markdown, is defined once and blinks at motion full only", () => {
    const TRAIL = [
      ".omfp-caret-trail > .omfp-markdown:last-child > :last-child:not(ul, ol)::after",
      ".omfp-caret-trail > .omfp-markdown:last-child > :is(ul, ol):last-child > li:last-child::after",
    ];
    const look = findRule(kitRules, [LAYER], [".omfp-caret", ...TRAIL].join(", "));
    expect(look.declarations.get("background")).toBe("var(--omfp-grad)");
    const blinking = kitRules.filter(
      (rule) => rule.selector.includes(".omfp-caret") && rule.declarations.has("animation"),
    );
    expect(blinking.map((rule) => [rule.selector, rule.declarations.get("animation")])).toEqual([
      [
        [".omfp-caret", ...TRAIL].map((selector) => `:root[data-motion="full"] ${selector}`).join(", "),
        "omfp-blink 0.9s steps(1) infinite",
      ],
    ]);
  });

  const RING = "var(--omfp-focus-w) solid var(--omfp-focus-color)";
  /** The shared focus rules (UI-132): items (the shadow drawing the bar), the ring, the inset ring's offset. */
  const itemRules = kitRules.filter((rule) => rule.declarations.get("box-shadow")?.includes("--omfp-focus-bar"));
  const ringRules = kitRules.filter((rule) => rule.declarations.get("outline") === RING);
  const insetRules = kitRules.filter(
    (rule) => rule.declarations.get("outline-offset") === "calc(-1 * var(--omfp-focus-w))",
  );

  // spec: UI-132
  test("focused items take a primary tint and a 2 px primary bar, both inset shadows: no outline, glow or transition", () => {
    const [item, ...more] = itemRules;
    expect(more.length).toBe(0);
    expect(Object.fromEntries(item?.declarations ?? [])).toEqual({
      outline: "var(--focus-item-outline, none)",
      "outline-offset": "-2px",
      "box-shadow": "inset 2px 0 0 var(--omfp-focus-bar), inset 0 0 0 999rem var(--omfp-focus-bg)",
    });
    const selector = item?.selector ?? "";
    for (const part of [
      ".omfp-row",
      ".omfp-menu__item",
      ".omfp-listbox__item",
      ".omfp-tool__head:not(.omfp-bbox__head)",
      ".omfp-wsection__head",
      ".omfp-modeline__notice",
      ".omfp-strip__thread",
      ":is(.omfp-row, .omfp-focus-item):focus-visible",
      ".omfp-focus-item.is-focus",
      ".omfp-focus-item:has(.omfp-focus-item__target:focus-visible, .omfp-focus-item__target :focus-visible)",
      ".omfp-tool__head.omfp-bbox__head:is([data-focus-visible], .is-focus) .omfp-tool__label",
    ]) {
      expect(selector).toContain(part);
    }
    // A selected row's own bar (inset, gradient) gives way to the focus bar at full height.
    const selected = kitRules.filter(
      (rule) => rule.selector.startsWith(".omfp-row[data-selected]:is(") && rule.selector.endsWith("::before"),
    );
    expect(selected.map((rule) => Object.fromEntries(rule.declarations))).toEqual([
      { "inset-block": "0", background: "var(--omfp-focus-bar)" },
    ]);
  });

  // spec: UI-132
  test("controls and regions keep a quiet 1 px primary ring without glow, inset ones inside; fields only their border", () => {
    const [ring, ...more] = ringRules;
    expect(more.length).toBe(0);
    expect(Object.fromEntries(ring?.declarations ?? [])).toEqual({
      outline: RING,
      "outline-offset": "var(--omfp-focus-off)",
    });
    for (const part of [
      ".omfp-button",
      ".omfp-select__button",
      ".omfp-tabs__tab",
      ".omfp-focus-ring",
      ".omfp-focus-inset",
      ".omfp-help__list",
      ":is(.omfp-checkbox, .omfp-radio):is([data-focus-visible], .is-focus) :is(.omfp-checkbox__box, .omfp-radio__dot)",
    ]) {
      expect(ring?.selector).toContain(part);
    }
    // Text fields take no ring: the primary border is their focus (the composer's box draws its own, UI-172).
    expect(ring?.selector).not.toContain("input-box");
    const field = findRule(kitRules, [LAYER], ".omfp-input-box:focus-within, .omfp-field.is-focus .omfp-input-box");
    expect(Object.fromEntries(field.declarations)).toEqual({ "border-color": "var(--omfp-focus-color)" });
    const [inset, ...others] = insetRules;
    expect(others.length).toBe(0);
    expect(inset?.selector).toContain(".omfp-focus-inset");
    expect(inset?.declarations.size).toBe(1);
  });

  // spec: UI-132, UI-146
  test("one convention: no other kit or app rule draws a focus outline or shadow, and only insert mode pulses", () => {
    const FOCUS_STATE = /focus-visible|focus-within|\.is-focus|:focus\b/;
    const shared = new Set([...itemRules, ...ringRules, ...insetRules].map((rule) => rule.selector));
    const drawing = [join(KIT_SRC, "kit.css"), ...cssFiles(APP_SRC)].flatMap((path) =>
      parseCss(readFileSync(path, "utf8"))
        .filter((rule) => FOCUS_STATE.test(rule.selector) && !shared.has(rule.selector))
        .filter((rule) =>
          ["outline", "outline-offset", "box-shadow"].some((property) => rule.declarations.has(property)),
        )
        .map((rule) => `${basename(path)}: ${rule.selector}`),
    );
    expect(drawing).toEqual([]);
    const glowing = kitRules.filter((rule) => rule.declarations.get("animation")?.startsWith("omfp-glow"));
    expect(glowing.map((rule) => rule.selector)).toEqual([
      ':root[data-motion="full"] .omfp-compose[data-mode="insert"]',
    ]);
  });

  // spec: UI-132
  test("forced colors: items take a 2 px Highlight outline inside their edge, the ring turns Highlight", () => {
    const forced = findRule(kitRules, [LAYER, FORCED_COLORS], ":root");
    expect(Object.fromEntries(forced.declarations)).toEqual({
      "--omfp-focus-color": "Highlight",
      "--focus-item-outline": "2px solid Highlight",
    });
  });
});

describe("one style of our own", () => {
  const UI_ROOT = join(KIT_SRC, "..", "..", "..");
  /** Every source file of the kit and the app. */
  function sources(dir: string): string[] {
    return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        return sources(path);
      }
      return /\.(tsx?|css)$/.test(entry.name) ? [path] : [];
    });
  }
  const STYLESHEET_IMPORT = /(?:import\s+(?:[^"']*from\s+)?|@import\s+(?:url\()?)["']([^"']+\.css)["']/g;

  // spec: UI-120
  test("no stylesheet comes from a third-party package: the kit and the app import only our own CSS", () => {
    const foreign = [...sources(KIT_SRC), ...sources(APP_SRC)].flatMap((path) =>
      [...readFileSync(path, "utf8").matchAll(STYLESHEET_IMPORT)]
        .map((match) => match[1] ?? "")
        .filter((specifier) => !/^(?:\.|\/|@omfp\/)/.test(specifier))
        .map((specifier) => `${basename(path)}: ${specifier}`),
    );
    expect(foreign).toEqual([]);
  });

  // spec: UI-120
  test("no CSS framework or styled component library is a dependency", () => {
    const FRAMEWORK =
      /^(?:tailwindcss|@tailwindcss\/.*|postcss|unocss|@unocss\/.*|bootstrap|bulma|daisyui|@mantine\/.*|@chakra-ui\/.*|@radix-ui\/themes|@heroui\/.*|@nextui-org\/.*|@mui\/.*|antd|@adobe\/react-spectrum|@react-spectrum\/.*)$/;
    const manifests = ["package.json", "packages/kit/package.json", "packages/app/package.json"];
    const found = manifests.flatMap((manifest) => {
      const json = JSON.parse(readFileSync(join(UI_ROOT, manifest), "utf8")) as Record<string, unknown>;
      return ["dependencies", "devDependencies", "peerDependencies"].flatMap((field) =>
        Object.keys((json[field] as Record<string, string> | undefined) ?? {}).filter((name) => FRAMEWORK.test(name)),
      );
    });
    expect(found).toEqual([]);
  });
});
