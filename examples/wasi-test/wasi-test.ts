// The imported interface is declared in ../wit-modules.d.ts.
import random from "wasi:random/random@0.3.0";

// Test calling WASI imports
export function getRandomU64(): bigint {
    return random.getRandomU64();
}
