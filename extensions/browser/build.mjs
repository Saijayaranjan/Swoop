#!/usr/bin/env node
/**
 * Builds the extension for one or more targets (chrome, firefox, edge) into `dist/<target>/`.
 *
 * For each target this:
 *   1. bundles the TypeScript entry points with esbuild,
 *   2. merges `manifests/base.json` with `manifests/<target>.json` into `dist/<target>/manifest.json`,
 *   3. bundles the popup/options stylesheets (which `@import` the shared design tokens in
 *      src/shared-ui/tokens.css) and copies static assets (icons, HTML, `_locales`).
 *
 * Usage: `node build.mjs chrome firefox edge [--watch]`
 */

import * as esbuild from 'esbuild';
import { readFileSync, writeFileSync, mkdirSync, cpSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const VALID_TARGETS = ['chrome', 'firefox', 'edge'];

const args = process.argv.slice(2);
const watch = args.includes('--watch');
const targets = args.filter((a) => VALID_TARGETS.includes(a));
if (targets.length === 0) targets.push(...VALID_TARGETS);

function ensureIcons() {
  const iconsDir = path.join(ROOT, 'icons');
  const need = ['icon16.png', 'icon32.png', 'icon48.png', 'icon128.png'].some(
    (f) => !existsSync(path.join(iconsDir, f)),
  );
  if (need) {
    console.log('[icons] generating PNG icons...');
    execFileSync(process.execPath, [path.join(ROOT, 'scripts', 'generate-icons.mjs')], {
      stdio: 'inherit',
    });
  }
}

function mergeManifest(target) {
  const base = JSON.parse(readFileSync(path.join(ROOT, 'manifests', 'base.json'), 'utf8'));
  const overridePath = path.join(ROOT, 'manifests', `${target}.json`);
  const override = JSON.parse(readFileSync(overridePath, 'utf8'));
  return { ...base, ...override };
}

function copyStaticAssets(target) {
  const distDir = path.join(ROOT, 'dist', target);
  mkdirSync(distDir, { recursive: true });

  writeFileSync(
    path.join(distDir, 'manifest.json'),
    `${JSON.stringify(mergeManifest(target), null, 2)}\n`,
  );

  cpSync(path.join(ROOT, 'icons'), path.join(distDir, 'icons'), {
    recursive: true,
    filter: (src) => !src.endsWith('.svg'),
  });

  mkdirSync(path.join(distDir, 'popup'), { recursive: true });
  cpSync(path.join(ROOT, 'src', 'popup', 'index.html'), path.join(distDir, 'popup', 'index.html'));

  mkdirSync(path.join(distDir, 'options'), { recursive: true });
  cpSync(path.join(ROOT, 'src', 'options', 'index.html'), path.join(distDir, 'options', 'index.html'));

  cpSync(path.join(ROOT, '_locales'), path.join(distDir, '_locales'), { recursive: true });
}

/** @returns {esbuild.BuildOptions[]} */
function bundleConfigsFor(target) {
  const outDir = path.join(ROOT, 'dist', target);
  const common = {
    bundle: true,
    sourcemap: true,
    target: ['chrome116', 'firefox121'],
    logLevel: 'info',
    define: {
      __SWOOP_TARGET__: JSON.stringify(target),
    },
  };
  return [
    {
      ...common,
      entryPoints: [path.join(ROOT, 'src', 'background', 'index.ts')],
      outfile: path.join(outDir, 'background', 'index.js'),
      format: 'esm',
      platform: 'browser',
    },
    {
      ...common,
      entryPoints: [path.join(ROOT, 'src', 'content', 'index.ts')],
      outfile: path.join(outDir, 'content', 'index.js'),
      // Content scripts run as classic (non-module) scripts across both engines.
      format: 'iife',
      platform: 'browser',
    },
    {
      ...common,
      entryPoints: [path.join(ROOT, 'src', 'popup', 'index.ts')],
      outfile: path.join(outDir, 'popup', 'index.js'),
      format: 'esm',
      platform: 'browser',
    },
    {
      ...common,
      entryPoints: [path.join(ROOT, 'src', 'options', 'index.ts')],
      outfile: path.join(outDir, 'options', 'index.js'),
      format: 'esm',
      platform: 'browser',
    },
    {
      bundle: true,
      logLevel: 'info',
      target: ['chrome116', 'firefox121'],
      entryPoints: [path.join(ROOT, 'src', 'popup', 'popup.css')],
      outfile: path.join(outDir, 'popup', 'popup.css'),
    },
    {
      bundle: true,
      logLevel: 'info',
      target: ['chrome116', 'firefox121'],
      entryPoints: [path.join(ROOT, 'src', 'options', 'options.css')],
      outfile: path.join(outDir, 'options', 'options.css'),
    },
  ];
}

async function buildTarget(target) {
  copyStaticAssets(target);
  const configs = bundleConfigsFor(target);
  if (watch) {
    const contexts = await Promise.all(configs.map((c) => esbuild.context(c)));
    await Promise.all(contexts.map((ctx) => ctx.watch()));
    console.log(`[watch] ${target}: watching for changes...`);
    return contexts;
  }
  for (const config of configs) {
    await esbuild.build(config);
  }
  console.log(`[build] ${target}: dist/${target}`);
  return [];
}

async function main() {
  ensureIcons();
  const allContexts = [];
  for (const target of targets) {
    const contexts = await buildTarget(target);
    allContexts.push(...contexts);
  }
  if (!watch) {
    console.log(`Done: ${targets.map((t) => `dist/${t}`).join(', ')}`);
  } else {
    process.on('SIGINT', async () => {
      await Promise.all(allContexts.map((ctx) => ctx.dispose()));
      process.exit(0);
    });
  }
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
