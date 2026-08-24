// The imported interface is declared in ./wit-modules.d.ts.
import math from "local:test/math";

// Use the imported math interface
export function doubleAdd(a: number, b: number): number {
    const sum = math.add(a, b);
    return math.multiply(sum, 2);
}
