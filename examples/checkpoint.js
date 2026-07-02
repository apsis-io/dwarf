// wit.Checkpoint.snapshot() demo: dump this component's WASM linear memory
// (the QuickJS heap, this counter included) as raw bytes — useful for
// diagnostics/offline inspection. See README's "Checkpoint / Restore
// (experimental)" section for why there's no restore() counterpart.

let count = 0;

export function bump() {
    count += 1;
    return count;
}

export function get() {
    return count;
}

export function snapshot() {
    return wit.Checkpoint.snapshot();
}
