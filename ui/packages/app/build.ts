/**
 * Release bundle of the web UI (`just build-web`) into `dist/`, which the daemon serves (DMN-008).
 *
 * One `Bun.build` with code splitting over the app entry and one entry per module the import map gives widgets
 * (`react`, `react/jsx-runtime`, `react-dom`, `omfp/sdk`, `omfp/kit`, AGT-012), so the app and every widget share
 * one React and one SDK instance. React is CommonJS: its entries re-export its named exports, listed at build time.
 * Besides the bundle, `dist/` gets:
 * - `index.html` (index-html.ts): the favicon, the web app manifest, the import map, the bundle's CSS, then the user
 *   theme (`/theme.css`, UI-091, so its tokens win), and the app entry;
 * - the favicon files, `icon.svg` and `icon-32.png`, from docs/brand (UI-222);
 * - `manifest.webmanifest` and its icons `app-icon-192.png` (docs/brand) and `app-icon-512.png` (the desktop icon), so
 *   a phone's browser installs the app standalone (UI-425);
 * - `widget-exports.json`: every export name of each import-map module, which the daemon checks widget imports against
 *   (DMN-085);
 * - `builtin-widgets/<name>/`: the source of each built-in widget as a copyable widget folder (AGT-021, DMN-087);
 * - `fonts/`: the bundled faces of the kit's tokens with their licences (UI-141);
 * - `pdfjs/`: pdf.js's worker (its legacy build, like the kit's `PdfView`) and the data files it fetches only when a
 *   document needs them — CMaps, the standard fonts, its wasm decoders, ICC profiles — with their licences (UI-543);
 * - `katex/`: KaTeX's fonts (`.woff2`, the first format its stylesheet offers), which the stylesheet (added by the
 *   kit's math chunk) loads from `/katex/fonts/` when a formula uses them, with KaTeX's licence (UI-854).
 */
import { copyFile, mkdir, readdir, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { basename, dirname, join } from "node:path";
import { BUILTINS } from "./builtins.ts";
import { APP_ICONS, FAVICONS, indexHtml, MANIFEST_NAME, webManifest } from "./index-html.ts";

const require = createRequire(import.meta.url);
const root = import.meta.dir;
// `OMFP_WEB_OUT` (an absolute path) builds elsewhere, e.g. the private bundle of `just test-e2e`: a rebuild of `dist/`
// by another process (it is deleted first) would otherwise break a test serving it. The generated entries are per
// process, so concurrent builds do not delete each other's.
const outdir = process.env.OMFP_WEB_OUT || join(root, "dist");
const entries = join(root, `.entries-${process.pid}`);

/** An import-map module: its specifier, the name of its bundle entry, and how its entry is generated. */
interface Shared {
  readonly specifier: string;
  readonly name: string;
  /** CommonJS package re-exported name by name, or an ES module re-exported with `export *`. */
  readonly source: { readonly kind: "commonjs" } | { readonly kind: "esm"; readonly path: string };
}

const SHARED: readonly Shared[] = [
  { specifier: "react", name: "react", source: { kind: "commonjs" } },
  { specifier: "react/jsx-runtime", name: "jsx-runtime", source: { kind: "commonjs" } },
  { specifier: "react-dom", name: "react-dom", source: { kind: "commonjs" } },
  { specifier: "omfp/sdk", name: "sdk", source: { kind: "esm", path: join(root, "src/sdk/index.ts") } },
  { specifier: "omfp/kit", name: "kit", source: { kind: "esm", path: join(root, "../kit/src/index.ts") } },
];

const IDENTIFIER = /^[A-Za-z_$][\w$]*$/;

/** Export names of a shared module, `default` included when it has one. */
async function exportNames(shared: Shared): Promise<string[]> {
  if (shared.source.kind === "commonjs") {
    const names = Object.keys(require(shared.specifier)).filter((name) => name !== "default" && IDENTIFIER.test(name));
    return [...names, "default"].sort();
  }
  // The module to inspect comes from the SHARED table.
  return Object.keys(await import(shared.source.path)).sort();
}

function entrySource(shared: Shared, names: readonly string[]): string {
  if (shared.source.kind === "esm") {
    return `export * from ${JSON.stringify(shared.source.path)};\n`;
  }
  const named = names.filter((name) => name !== "default");
  return [
    `import * as M from ${JSON.stringify(shared.specifier)};`,
    "const m = M as unknown as Record<string, unknown>;",
    `export const { ${named.join(", ")} } = m;`,
    "export default m.default ?? M;",
    "",
  ].join("\n");
}

await rm(outdir, { recursive: true, force: true });
await rm(entries, { recursive: true, force: true });
await mkdir(entries, { recursive: true });

const exportsBySpecifier: Record<string, string[]> = {};
const entrypoints = [join(root, "src/main.tsx")];
// The generated entries live in the package, where `react` resolves; they go away whatever happens.
let result: Awaited<ReturnType<typeof Bun.build>>;
try {
  for (const shared of SHARED) {
    const names = await exportNames(shared);
    exportsBySpecifier[shared.specifier] = names;
    const path = join(entries, `${shared.name}.ts`);
    await Bun.write(path, entrySource(shared, names));
    entrypoints.push(path);
  }
  result = await Bun.build({
    entrypoints,
    outdir,
    splitting: true,
    format: "esm",
    minify: true,
    publicPath: "/",
    sourcemap: "linked",
    naming: { entry: "[name]-[hash].[ext]", chunk: "chunk-[hash].[ext]", asset: "[name]-[hash].[ext]" },
    // The kit's `@font-face` rules point at `/fonts/…`, copied below as files (UI-141, UI-142): the browser fetches a
    // face only when text uses it (the bundler would otherwise inline every face into the CSS).
    external: ["/fonts/*"],
    define: { "process.env.NODE_ENV": JSON.stringify("production") },
  });
} finally {
  await rm(entries, { recursive: true, force: true });
}
if (!result.success) {
  for (const log of result.logs) {
    console.error(log);
  }
  process.exit(1);
}

/** The URL of the entry chunk built from `<name>.ts`/`.tsx`. */
function entryUrl(name: string): string {
  const output = result.outputs.find(
    (candidate) => candidate.kind === "entry-point" && basename(candidate.path).startsWith(`${name}-`),
  );
  if (output === undefined) {
    throw new Error(`no entry chunk for ${name}`);
  }
  return `/${basename(output.path)}`;
}

const importMap = { imports: Object.fromEntries(SHARED.map((shared) => [shared.specifier, entryUrl(shared.name)])) };
const styles = result.outputs
  .filter((output) => output.path.endsWith(".css"))
  .map((output) => `/${basename(output.path)}`);
await Bun.write(join(outdir, "index.html"), indexHtml({ importMap, styles, main: entryUrl("main") }));
for (const { source, name } of [...FAVICONS, ...APP_ICONS]) {
  await copyFile(join(root, "../../..", source), join(outdir, name));
}
await Bun.write(join(outdir, MANIFEST_NAME), `${JSON.stringify(webManifest(), null, 2)}\n`);
await Bun.write(join(outdir, "widget-exports.json"), `${JSON.stringify(exportsBySpecifier, null, 2)}\n`);

const fonts = join(root, "../kit/fonts");
await mkdir(join(outdir, "fonts"));
for (const file of await readdir(fonts)) {
  await copyFile(join(fonts, file), join(outdir, "fonts", file));
}

// pdf.js and KaTeX are dependencies of the kit, where they resolve. Files, not a recursive copy: those of the Nix store
// are read-only, and so would be the copied folders, which the next build could then not delete.
const kitRequire = createRequire(join(root, "../kit/package.json"));
const pdfjs = dirname(kitRequire.resolve("pdfjs-dist/package.json"));
const pdfjsOut = join(outdir, "pdfjs");
await mkdir(pdfjsOut);
await copyFile(join(pdfjs, "legacy/build/pdf.worker.min.mjs"), join(pdfjsOut, "pdf.worker.min.mjs"));
await copyFile(join(pdfjs, "LICENSE"), join(pdfjsOut, "LICENSE"));
for (const folder of ["cmaps", "standard_fonts", "wasm", "iccs"]) {
  await mkdir(join(pdfjsOut, folder));
  for (const file of await readdir(join(pdfjs, folder))) {
    await copyFile(join(pdfjs, folder, file), join(pdfjsOut, folder, file));
  }
}

const katex = dirname(kitRequire.resolve("katex/package.json"));
await mkdir(join(outdir, "katex/fonts"), { recursive: true });
await copyFile(join(katex, "LICENSE"), join(outdir, "katex/LICENSE"));
for (const file of (await readdir(join(katex, "dist/fonts"))).filter((name) => name.endsWith(".woff2"))) {
  await copyFile(join(katex, "dist/fonts", file), join(outdir, "katex/fonts", file));
}

for (const { name, folder, part } of BUILTINS) {
  const from = join(root, "src/widgets", folder);
  const to = join(outdir, "builtin-widgets", name);
  await mkdir(to, { recursive: true });
  for (const file of await readdir(from)) {
    // Widgets are TypeScript modules; the folder's CSS is part of the app's stylesheet.
    if (/\.(ts|tsx)$/.test(file)) {
      await copyFile(join(from, file), join(to, file));
    }
  }
  if (part !== undefined) {
    await Bun.write(
      join(to, "index.ts"),
      [
        `/** \`omfp:${name}\`, copied from the built-in chat folder (AGT-021). */`,
        "",
        `import { ${part.exportName} } from "./${part.file}";`,
        "",
        `export default ${part.exportName};`,
        "",
      ].join("\n"),
    );
  }
}
