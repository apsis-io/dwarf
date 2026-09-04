// The NEGATIVE control: identical in shape, except the thing being raced is a
// plain JavaScript promise instead of a WIT future. A promise is not a
// waitable, so nothing can ever deliver an event for it to the suspended
// task, and the continuation is scheduled and never drained.
let resolveIt = null;

export async function block() {
  const p = new Promise((resolve) => {
    resolveIt = resolve;
  });
  return await Promise.race([
    wedge().then(() => "WEDGE"),
    p.then(() => "WOKEN"),
  ]);
}

export function poke() {
  resolveIt();
  return 1;
}
