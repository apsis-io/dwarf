// The wasi:http surface this example uses. The shared WIT shapes and the
// `wit` global live in ../wit-globals.d.ts.
//
// Hand-written and deliberately partial: it covers the calls in app.ts and
// nothing else. The generated equivalent, which covers the whole world and
// cannot drift:
//
//     dwarf --wit wit --file app.ts --emit-types types/ \
//           --polyfill fetch-classes -o hono.wasm

declare module "wasi:http/types" {
    /** WIT `fields` - the headers resource. */
    export class Fields {
        constructor();
        append(name: string, value: number[]): void;
        copyAll(): Array<[string, number[]]>;
    }

    /** The incoming request resource. */
    export interface IncomingRequest {
        getMethod(): { tag: string; val?: string };
        getScheme(): { tag: string; val?: string } | null;
        getPathWithQuery(): string | null;
        getAuthority(): string | null;
        getHeaders(): Fields;
    }

    export const Request: {
        /** Takes the body stream, plus the "request transmitted" future. */
        consumeBody(
            request: IncomingRequest,
            transmitted: WitFutureReadable<WitOk<void> | WitErr>,
        ): [WitReadable, WitFutureReadable<unknown>];
    };

    export const Response: {
        // Quoted, so this is a METHOD named `new` - `WasiResponse.new(...)`
        // in the source - and not a construct signature.
        "new"(
            headers: Fields,
            body: WitReadable,
            trailers: WitFutureReadable<unknown>,
        ): [OutgoingResponse, unknown];
    };

    export interface OutgoingResponse {
        setStatusCode(code: number): void;
    }
}
