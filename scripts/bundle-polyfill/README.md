# bundle-polyfill

Dev-only tool for authoring dwarf's static polyfills (`crates/core/polyfills/*.js`,
registered in `crates/core/src/polyfills.rs`'s `POLYFILLS`). Not published, not
part of the `npm/` package dwarf itself ships.

Bundles an npm package via esbuild and writes the result straight into
`crates/core/polyfills/<name>.js`, then prints a ready-to-paste `Polyfill { ... }`
entry - the `install` line's `globalThis.<name> = { ... }` object is generated
from the bundle's own actual export list via [knitwork](https://github.com/unjs/knitwork),
not hand-transcribed (the original motivation: `ufo` alone has 50+ exports,
easy to mistype or silently drop one by hand).

## Usage

```sh
cd scripts/bundle-polyfill
npm install          # once
npm install <pkg>    # install whatever package you're bundling
node bundle-polyfill.mjs <pkg> [options]
```

Options:

- `--name <name>` - polyfill name, defaults to the package name
- `--entry <path>` - custom entry file instead of `export * from "<pkg>"` (e.g.
  to re-export only a subset, or under aliases)
- `--platform <p>` - esbuild platform: `neutral` (default) | `browser` | `node` -
  use `neutral`/`browser` for packages with Node-vs-browser conditional exports
  (e.g. `ohash`'s crypto submodule) to pick the portable implementation
- `--global <expr>` - override the install line's global expression entirely,
  for a polyfill that exposes a single function/class rather than a namespace
  object (e.g. `klona`'s `globalThis.klona = klona;`)

## What this does NOT automate

- `crates/core/polyfills/<name>.d.ts` - write by hand
- A [NOTICES](../../NOTICES) entry for the vendored package's license
- `docs/cli-cheatsheet.md` and `README.md`'s polyfill tables
- Tests exercising the new polyfill

## Example

```sh
node bundle-polyfill.mjs ohash --platform neutral
```

produces `crates/core/polyfills/ohash.js` and prints the matching
`Polyfill { ... }` entry to paste into `POLYFILLS`.
