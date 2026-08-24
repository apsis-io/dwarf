// DX layer over the fetch-provider component: a standard `fetch(input, init)`
// that accepts a URL string or a `Request` and returns a real `Response`,
// hiding the flattened wire format (strings/lists/records) `dwarf:fetch/client`
// itself uses. Requires the `fetch-classes` polyfill (`--polyfill
// fetch-classes`) for `Request`/`Response`/`Headers`, and this component's own
// interface composed in via `wac plug` (see README.md).
import { fetch as wireFetch } from "dwarf:fetch/client";

export async function fetch(
  input: string | Request,
  init: RequestInit = {},
): Promise<Response> {
  const request = input instanceof Request ? input : new Request(input, init);

  const headers: WireHeader[] = [];
  for (const [name, value] of request.headers.entries()) {
    headers.push({ name, value });
  }

  const buf = await request.arrayBuffer();
  const body = buf.byteLength > 0 ? Array.from(new Uint8Array(buf)) : null;

  const wireResponse = await wireFetch({
    url: request.url,
    method: request.method,
    headers,
    body,
  });

  const responseHeaders = new Headers();
  for (const h of wireResponse.headers) {
    responseHeaders.append(h.name, h.value);
  }

  return new Response(Uint8Array.from(wireResponse.body), {
    status: wireResponse.status,
    headers: responseHeaders,
  });
}
