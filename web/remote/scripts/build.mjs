#!/usr/bin/env node
// Production build: bundles src/main.tsx (+ its imported CSS) with esbuild into a single
// hashed app.js / app.css, writes dist/index.html referencing them, and enforces the < 150 KB
// gzipped budget for the whole bundle (docs: this UI is embedded into the server binary).

import { build } from "esbuild";
import { gzipSync } from "node:zlib";
import { mkdirSync, readFileSync, rmSync, writeFileSync, statSync } from "node:fs";
import { join, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const outdir = join(root, "dist");
const BUDGET_BYTES = 150 * 1024;

rmSync(outdir, { recursive: true, force: true });
mkdirSync(outdir, { recursive: true });

const result = await build({
  entryPoints: [join(root, "src", "main.tsx")],
  bundle: true,
  outdir,
  entryNames: "[name]-[hash]",
  assetNames: "[name]-[hash]",
  format: "esm",
  target: ["es2022"],
  minify: true,
  sourcemap: false,
  metafile: true,
  jsx: "automatic",
  jsxImportSource: "preact",
  legalComments: "none",
  logLevel: "info",
});

const outputs = Object.keys(result.metafile.outputs);
const jsFile = outputs.find((f) => f.endsWith(".js"));
const cssFile = outputs.find((f) => f.endsWith(".css"));

if (!jsFile) {
  console.error("Build produced no JS output");
  process.exit(1);
}

const jsName = basename(jsFile);
const cssName = cssFile ? basename(cssFile) : null;

const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
<meta name="color-scheme" content="light dark" />
<title>Osprey</title>
${cssName ? `<link rel="stylesheet" href="/${cssName}" />` : ""}
</head>
<body>
<div id="app"></div>
<script type="module" src="/${jsName}"></script>
</body>
</html>
`;

writeFileSync(join(outdir, "index.html"), html);

function gzipSize(path) {
  return gzipSync(readFileSync(path)).length;
}

const htmlPath = join(outdir, "index.html");
const jsPath = join(outdir, jsName);
const cssPath = cssName ? join(outdir, cssName) : null;

const sizes = {
  "index.html": { raw: statSync(htmlPath).size, gzip: gzipSize(htmlPath) },
  [jsName]: { raw: statSync(jsPath).size, gzip: gzipSize(jsPath) },
};
if (cssPath) {
  sizes[cssName] = { raw: statSync(cssPath).size, gzip: gzipSize(cssPath) };
}

let totalGzip = 0;
console.log("\nBundle sizes:");
for (const [name, { raw, gzip } ] of Object.entries(sizes)) {
  totalGzip += gzip;
  console.log(`  ${name.padEnd(28)} ${String(raw).padStart(8)} B raw   ${String(gzip).padStart(8)} B gzip`);
}
console.log(`  ${"TOTAL (gzip)".padEnd(28)} ${" ".padStart(8)}      ${String(totalGzip).padStart(8)} B`);

if (totalGzip > BUDGET_BYTES) {
  console.error(`\nBundle exceeds the ${BUDGET_BYTES} B (150 KB) gzipped budget by ${totalGzip - BUDGET_BYTES} B.`);
  process.exit(1);
}
console.log(`\nWithin budget: ${totalGzip} B / ${BUDGET_BYTES} B gzipped.`);
