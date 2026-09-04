let waker = null;

export async function block() {
  // Built INSIDE the export, not at module scope: top-level code runs during
  // the Wizer snapshot, where there is no task context, and building a future
  // there traps the BUILD. It belongs here anyway - the read has to register
  // in this task's own waitable set.
  const f = wit.Future(wit.Future.U32);
  waker = f.writable;
  return await Promise.race([
    wedge().then(() => "WEDGE"),
    f.readable.read().then(() => "WOKEN"),
  ]);
}

export function poke() {
  waker.write(7);
  return 1;
}
