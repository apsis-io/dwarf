#!/usr/bin/env node
/**
 * Bundles an npm package (or a custom entry file) into a dwarf static
 * polyfill: writes crates/core/polyfills/<name>.js and prints a ready-to-
 * paste `Polyfill { ... }` entry for crates/core/src/polyfills.rs, with the
 * `install` line's `globalThis.<name> = { ... }` object generated from the
 * bundle's own actual export list via knitwork - not hand-transcribed, so
 * it can't drift from what's really exported (the original motivation:
 * ufo alone has 50+ exports, easy to mistype or miss one by hand).
 *
 * Usage (run from this directory, after `npm install` here):
 *   node bundle-polyfill.mjs <npm-package> [options]
 *
 * Options:
 *   --name <name>      Polyfill name (default: the package name)
 *   --entry <path>     Custom entry file instead of `export * from "<pkg>"`
 *                       (e.g. to re-export only a subset, or under aliases)
 *   --platform <p>     esbuild platform: neutral (default) | browser | node
 *                       - use browser/neutral for packages with Node-vs-
 *                       browser conditional exports (e.g. ohash's crypto
 *                       submodule) to pick the portable implementation
 *   --global <expr>    Override the install line's global expression
 *                       entirely (e.g. a single function/class export like
 *                       klona's `globalThis.klona = klona;`)
 *
 * This tool does not install packages itself - `npm install <pkg>` here
 * first. Still needs manual follow-up: write crates/core/polyfills/
 * <name>.d.ts, add a NOTICES entry, and update the docs/tests - this only
 * automates the bundle + the install-line transcription.
 */
import { build } from "esbuild";
import { genObjectFromRawEntries } from "knitwork";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const polyfillsDir = join(scriptDir, "..", "..", "crates", "core", "polyfills");

function parseArgs(argv) {
  const [pkg, ...rest] = argv;
  if (!pkg) {
    console.error(
      "Usage: node bundle-polyfill.mjs <npm-package> [--name <name>] [--entry <path>] [--platform neutral|browser|node] [--global <expr>]",
    );
    process.exit(1);
  }
  const opts = { pkg, name: pkg, entry: null, platform: "neutral", global: null };
  for (let i = 0; i < rest.length; i += 2) {
    const key = rest[i]?.replace(/^--/, "");
    const value = rest[i + 1];
    if (!(key in opts)) {
      console.error(`Unknown option --${key}`);
      process.exit(1);
    }
    opts[key] = value;
  }
  return opts;
}

/** Parses an esbuild bundle's trailing `export { a, b as c, ... };` block into [exportedName, localName] pairs. */
function parseExportBlock(code) {
  const match = code.match(/export\s*\{([\s\S]*?)\}\s*;?\s*$/);
  if (!match) {
    throw new Error("Could not find a trailing `export { ... };` block in the bundle output - is this a valid ESM bundle?");
  }
  return match[1]
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => {
      const asMatch = entry.match(/^(.+?)\s+as\s+(.+)$/);
      if (asMatch) {
        const [, local, exported] = asMatch;
        return [exported.trim(), local.trim()];
      }
      return [entry, entry];
    });
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));

  const entrySource = opts.entry ? readFileSync(opts.entry, "utf8") : `export * from "${opts.pkg}";`;

  // esbuild's bare-specifier resolution walks up from the *importing file's
  // own directory* looking for node_modules - not just "the working
  // directory" - so the scratch entry file has to live inside this
  // directory (a sibling of node_modules/, where the target package must
  // already be installed), not in the OS's shared temp dir.
  const tmpDir = mkdtempSync(join(scriptDir, ".bundle-polyfill-tmp-"));
  const entryPath = join(tmpDir, "entry.mjs");
  writeFileSync(entryPath, entrySource);

  let result;
  try {
    result = await build({
      entryPoints: [entryPath],
      bundle: true,
      format: "esm",
      target: "es2022",
      platform: opts.platform,
      legalComments: "none",
      write: false,
    });
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }

  const code = result.outputFiles[0].text;
  const outPath = join(polyfillsDir, `${opts.name}.js`);
  const header = `// ${opts.name}.js - bundled build of ${opts.pkg} (see NOTICES)\n`;
  writeFileSync(outPath, header + code);

  const installLine = opts.global
    ? `globalThis.${opts.name} = ${opts.global};`
    : `globalThis.${opts.name} = ${genObjectFromRawEntries(parseExportBlock(code))};`;

  console.log(`Wrote bundled JS to ${outPath}\n`);
  console.log(`Still needed by hand: crates/core/polyfills/${opts.name}.d.ts, a NOTICES entry, docs, and tests.\n`);
  console.log("Paste this into POLYFILLS in crates/core/src/polyfills.rs:\n");
  console.log(`    Polyfill {`);
  console.log(`        name: "${opts.name}",`);
  console.log(`        source: include_str!("../polyfills/${opts.name}.js"),`);
  console.log(`        install: ${JSON.stringify(installLine)},`);
  console.log(`        dts: include_str!("../polyfills/${opts.name}.d.ts"),`);
  console.log(`    },`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
