# Module resolution example

This example shows imports resolved during Wizer initialization:

- `./lib/math.js` is a relative import — and resolves `lib/math.ts`, which is
  TypeScript's own convention: the specifier names the *emitted* module.
- `./config` demonstrates extension inference (it finds `config.ts`).
- `local-greeter` is resolved from the local `node_modules` fixture.

Build it from the repository root:

```bash
dwarf \
  --wit examples/module-resolution/package.wit \
  --file examples/module-resolution/main.ts \
  --module-root examples/module-resolution \
  --output module-resolution.wasm
```

The `--module-root` directory is exposed read-only during Wizer so imported
files can be read and baked into the generated component.

Without the flag the root is the **entry file's own directory** — not the
current working directory, which would make the build depend on where it was
invoked from (see "Reproducible builds" in the repository README). This
example passes it explicitly because `main.ts` imports from `./lib` and
`./node_modules`, which its own directory already covers; an entry importing
from ABOVE its directory needs the flag to name that wider root.

The loader expects ES modules — `.ts`/`.mts`/`.cts` (types stripped as they
are read) or `.js`/`.mjs`. It resolves package metadata with `oxc_resolver`,
but it does not transform CommonJS packages.
