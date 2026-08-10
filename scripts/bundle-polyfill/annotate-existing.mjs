/* Re-apply the .d.ts -> JSDoc pass to the bundles already checked in.
 *
 * `bundle-polyfill.mjs` does this for anything it bundles from now on;
 * this is for the ones bundled before it did, and for after a .d.ts is
 * edited. Idempotent: the annotator only inserts where no JSDoc-derived
 * types exist, because re-running it over annotated output finds the same
 * declarations and writes the same blocks.
 *
 *   node annotate-existing.mjs [--check]
 */
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { annotateBundle, signaturesFromDts } from "./annotate.mjs";

const polyfillsDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "crates", "core", "polyfills");
const check = process.argv.includes("--check");
let changed = 0;

for (const file of readdirSync(polyfillsDir).filter((n) => n.endsWith(".js")).sort()) {
  const name = file.slice(0, -3);
  const dts = join(polyfillsDir, `${name}.d.ts`);
  if (!existsSync(dts)) continue;
  const jsPath = join(polyfillsDir, file);
  const before = readFileSync(jsPath, "utf8");
  const { code, annotated } = annotateBundle(before, signaturesFromDts(readFileSync(dts, "utf8"), dts));
  if (annotated === 0 || code === before) continue;
  changed++;
  console.log(`${name}: typed ${annotated} export(s)`);
  if (!check) writeFileSync(jsPath, code);
}

if (check && changed > 0) {
  console.error(`\n${changed} bundle(s) are missing types their .d.ts already declares — run without --check.`);
  process.exit(1);
}
console.log(changed === 0 ? "All bundles carry the types their .d.ts declares." : `\nUpdated ${changed} bundle(s).`);
