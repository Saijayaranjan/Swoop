#!/usr/bin/env node
// Dev server: esbuild watch + serve, with an unhashed index.html for a stable URL while editing.

import { context } from "esbuild";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const outdir = join(root, "dist-dev");
const port = 5173;

mkdirSync(outdir, { recursive: true });
writeFileSync(
  join(outdir, "index.html"),
  `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
<meta name="color-scheme" content="light dark" />
<title>Swoop (dev)</title>
<link rel="stylesheet" href="/main.css" />
</head>
<body>
<div id="app"></div>
<script type="module" src="/main.js"></script>
</body>
</html>
`,
);

const ctx = await context({
  entryPoints: [join(root, "src", "main.tsx")],
  bundle: true,
  outdir,
  entryNames: "[name]",
  format: "esm",
  target: ["es2022"],
  sourcemap: true,
  jsx: "automatic",
  jsxImportSource: "preact",
  logLevel: "info",
});

await ctx.watch();
const { host, port: boundPort } = await ctx.serve({ servedir: outdir, port });
console.log(`\nSwoop remote UI dev server: http://${host === "0.0.0.0" ? "localhost" : host}:${boundPort}/\n`);
console.log("Note: API calls are same-origin; point this at a build that also proxies /api and /api/v1/events to a running swoop daemon, or open the built dist/ from the daemon itself for full functionality.\n");
