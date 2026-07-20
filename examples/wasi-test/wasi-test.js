import random from "wasi:random/random@0.3.0";

// Test calling WASI imports
export function getRandomU64() {
    return random.getRandomU64();
}
