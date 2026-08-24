// The wasi:http surface this provider uses, and the flattened
// `dwarf:fetch/client` wire format it exports. Shared WIT shapes and the
// `wit` global are in ../wit-globals.d.ts.
//
// Generate the complete, drift-proof version instead with:
//
//     dwarf --wit . --file main.ts --emit-types types/ -o fetch-provider.wasm

/** The flattened records this component exports - no resources, no streams. */
interface WireHeader {
    name: string;
    value: string;
}

interface WireRequest {
    url: string;
    method: string;
    headers: WireHeader[];
    body: number[] | null;
}

interface WireResponse {
    status: number;
    headers: WireHeader[];
    body: number[];
}

declare module "wasi:http/types@0.3.0" {
    export class Fields {
        static fromList(entries: Array<[string, number[]]>): Fields;
        copyAll(): Array<[string, number[]]>;
    }

    export interface OutgoingRequest {
        setMethod(method: WitVariant<string>): void;
        setScheme(scheme: WitVariant<string>): void;
        setAuthority(authority: string): void;
        setPathWithQuery(pathWithQuery: string): void;
    }

    export interface IncomingResponse {
        getStatusCode(): number;
        getHeaders(): Fields;
    }

    export const Request: {
        "new"(
            headers: Fields,
            body: WitReadable | null,
            trailers: WitFutureReadable<unknown>,
            options: unknown | null,
        ): [OutgoingRequest, WitFutureReadable<unknown>];
    };

    export const Response: {
        consumeBody(
            response: IncomingResponse,
            transmitted: WitFutureReadable<unknown>,
        ): [WitReadable, WitFutureReadable<unknown>];
    };
}

declare module "wasi:http/client@0.3.0" {
    import type { IncomingResponse, OutgoingRequest } from "wasi:http/types@0.3.0";
    export function send(request: OutgoingRequest): Promise<IncomingResponse>;
}

declare module "dwarf:fetch/client" {
    export function fetch(request: WireRequest): Promise<WireResponse>;
}
