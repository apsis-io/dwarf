// Hand-written (not vendored) minimal ReadableStream (WHATWG Streams API)
// implementation - not a full spec implementation (no BYOB readers, no
// tee(), no pipeTo/pipeThrough), just the pull-based controller pattern
// verified against real ReadableStream to match exactly what libraries like
// h3 actually use for streaming response bodies (h3's own `iterable()`
// helper: `new ReadableStream({ async pull(controller) { ... } })` with
// `controller.enqueue`/`controller.close`).
class DwarfReadableStreamController {
  constructor(stream) {
    this._stream = stream;
  }
  enqueue(chunk) {
    const s = this._stream;
    if (s._pendingReads.length > 0) {
      const { resolve } = s._pendingReads.shift();
      resolve({ value: chunk, done: false });
    } else {
      s._queue.push(chunk);
    }
  }
  close() {
    const s = this._stream;
    s._closed = true;
    while (s._pendingReads.length > 0) {
      const { resolve } = s._pendingReads.shift();
      resolve({ value: undefined, done: true });
    }
  }
  error(e) {
    const s = this._stream;
    s._errored = e;
    while (s._pendingReads.length > 0) {
      const { reject } = s._pendingReads.shift();
      reject(e);
    }
  }
  get desiredSize() {
    return this._stream._closed ? 0 : 1;
  }
}

class DwarfReadableStream {
  constructor(source = {}) {
    this._queue = [];
    this._pendingReads = [];
    this._closed = false;
    this._errored = null;
    this._source = source;
    this._locked = false;
    this._controller = new DwarfReadableStreamController(this);
    this._startPromise = Promise.resolve(source.start ? source.start(this._controller) : undefined);
  }

  async _pull() {
    if (this._source.pull) await this._source.pull(this._controller);
  }

  getReader() {
    if (this._locked) throw new TypeError("ReadableStream is already locked to a reader");
    this._locked = true;
    const stream = this;
    return {
      async read() {
        await stream._startPromise;
        if (stream._errored) throw stream._errored;
        if (stream._queue.length > 0) return { value: stream._queue.shift(), done: false };
        if (stream._closed) return { value: undefined, done: true };
        const resultPromise = new Promise((resolve, reject) => {
          stream._pendingReads.push({ resolve, reject });
        });
        await stream._pull();
        return resultPromise;
      },
      async cancel(reason) {
        if (stream._source.cancel) await stream._source.cancel(reason);
        stream._closed = true;
      },
      releaseLock() {
        stream._locked = false;
      },
    };
  }

  async cancel(reason) {
    if (this._source.cancel) await this._source.cancel(reason);
    this._closed = true;
  }
}

// Wraps a `wit.Stream()` readable end (or any object with a WASI-style
// `read(count)`/`drop()`/`cancelRead()`) as a real ReadableStream, for
// handing a WASI-backed body to code that expects the standard interface.
//
// Deliberately single-read, not a drain loop: calling a WASI stream's
// `read()` again after all data is consumed AND the writable end has
// dropped is not a catchable JS error - it's a hard host-level trap
// ("cannot read after being notified that the writable end dropped"),
// confirmed empirically. A real drain loop would need a proven way to know
// more data is coming before calling `read()` again, which hasn't been
// established - so bodies larger than one `read(65536)` call are truncated,
// matching examples/fetch-provider's own documented limitation.
function dwarfReadableStreamFromWitStream(witReadable, chunkSize = 65536) {
  let dropped = false;
  const dropOnce = () => {
    if (!dropped) {
      dropped = true;
      witReadable.drop();
    }
  };
  return new DwarfReadableStream({
    async pull(controller) {
      const chunk = await witReadable.read(chunkSize);
      dropOnce();
      if (chunk && chunk.length > 0) controller.enqueue(chunk);
      controller.close();
    },
    cancel() {
      witReadable.cancelRead?.();
      dropOnce();
    },
  });
}
