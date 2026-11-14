import { copyFile, cp, mkdir, readdir, rm } from "node:fs/promises";
import { basename, join } from "node:path";
import { indexHtml } from "./index-html.ts";

const root = import.meta.dir;
const repository = join(root, "../../..");
const outdir = process.env.OMFP_SITE_OUT || join(root, "dist");
const videoUrl = process.env.OMFP_SITE_VIDEO_URL ?? "";

await rm(outdir, { recursive: true, force: true });
await mkdir(outdir, { recursive: true });

const result = await Bun.build({
  entrypoints: [join(root, "src/main.tsx")],
  outdir,
  minify: true,
  sourcemap: "linked",
  external: ["/fonts/*"],
  target: "browser",
  define: { __OMFP_SITE_VIDEO_URL__: JSON.stringify(videoUrl) },
  naming: "[name]-[hash].[ext]",
});
if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}

const script = result.outputs.find((output) => output.kind === "entry-point" && output.path.endsWith(".js"));
if (script === undefined) throw new Error("website build produced no JavaScript entrypoint");
const styles = result.outputs.filter((output) => output.path.endsWith(".css"));
for (const output of styles) {
  const css = await output.text();
  await Bun.write(output.path, css.replaceAll("url(/fonts/", "url(./fonts/"));
}

await Bun.write(
  join(outdir, "index.html"),
  indexHtml({ script: basename(script.path), styles: styles.map((output) => basename(output.path)) }),
);

for (const file of ["icon.svg", "screenshot-light.png", "screenshot-dark.png"]) {
  await copyFile(join(repository, "docs/brand", file), join(outdir, file));
}
await cp(join(root, "public/carousel"), join(outdir, "carousel"), { recursive: true });

const fonts = join(root, "../kit/fonts");
await mkdir(join(outdir, "fonts"));
for (const file of await readdir(fonts)) {
  await copyFile(join(fonts, file), join(outdir, "fonts", file));
}
