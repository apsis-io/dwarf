// Ambient declarations for the WIT interfaces the top-level examples
// import. Shared WIT shapes and the `wit` global are in ./wit-globals.d.ts.
//
// A WIT import is not an npm package: nothing on disk declares
// `wasi:cli/stdout@0.3.0`, so TypeScript has to be told what it is. Generate
// these from the world instead of writing them by hand:
//
//     dwarf --wit wit --file app.ts --emit-types types/ -o app.wasm
//
// This file has no top-level import/export on purpose - that is what makes
// it AMBIENT. Move a `declare module` into a file that has imports and it
// becomes a module *augmentation*, which fails for a module that does not
// otherwise exist.

declare module "wasi:cli/stdin@0.3.0" {
    const stdin: {
        readViaStream(): [WitReadable, WitFutureReadable<WitResult>];
    };
    export default stdin;
}

declare module "wasi:cli/stdout@0.3.0" {
    const stdout: {
        writeViaStream(stream: WitReadable): Promise<WitResult>;
    };
    export default stdout;
}

declare module "wasi:random/random@0.3.0" {
    const random: {
        /** WIT `u64` is a JavaScript `bigint`, not a `number`. */
        getRandomU64(): bigint;
        getRandomBytes(len: bigint): Uint8Array;
    };
    export default random;
}

declare module "local:test/math" {
    const math: {
        add(a: number, b: number): number;
        multiply(a: number, b: number): number;
    };
    export default math;
}
