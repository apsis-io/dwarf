// The WIT-to-JavaScript shapes shared by every example, declared once.
//
// These are dwarf's runtime conventions, not any particular world's types:
// what a `stream<u8>` is, what a `future<T>` is, what a variant looks like,
// and the `wit` global that constructs streams and futures. A world's own
// interfaces are declared next to the example that imports them.
//
// Generate the real thing for a project instead of writing it by hand:
//
//     dwarf --wit wit --file app.ts --emit-types types/ -o app.wasm

/** WIT `stream<u8>`, readable end. A zero-length read is end-of-stream. */
interface WitReadable {
    read(count?: number): Promise<Uint8Array>;
    drop(): void;
}

/** WIT `stream<u8>`, writable end. */
interface WitWritable {
    writeAll(bytes: number[] | Uint8Array): Promise<void>;
    drop(): void;
}

/** WIT `future<T>`, readable end. */
interface WitFutureReadable<T = unknown> {
    read(): Promise<T>;
    drop(): void;
}

/** WIT `future<T>`, writable end. */
interface WitFutureWritable<T = unknown> {
    write(value: T): void;
}

/** WIT `variant`: `{ tag }`, plus `val` for cases carrying a payload. */
interface WitVariant<T = unknown> {
    tag: string;
    val?: T;
}

type WitOk<T = void> = { tag: "ok"; val: T };
type WitErr<E = unknown> = { tag: "err"; val: E };
/** WIT `result<T, E>` where it is nested rather than a top-level return. */
type WitResult<T = void, E = unknown> = WitOk<T> | WitErr<E>;

/**
 * dwarf's component-model intrinsics.
 *
 * The `Future` type constants are PER-WORLD - which exist depends on the
 * futures that world's WIT mentions - so they are typed here as an index
 * signature rather than a fixed list. `Object.keys(wit.Future.types)` lists
 * the real ones at run time, which is how to find the name you need.
 */
declare const wit: {
    Stream: {
        (type: unknown): { readable: WitReadable; writable: WitWritable };
        U8: unknown;
    };
    Future: {
        <T = unknown>(type: unknown): {
            readable: WitFutureReadable<T>;
            writable: WitFutureWritable<T>;
        };
        types: Record<string, unknown>;
        readonly [constant: string]: unknown;
    };
};
