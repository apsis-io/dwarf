// The WIT type mappings, written out as TypeScript.
//
// These declarations are the point of the example: each one says what a WIT
// type actually is on the JavaScript side, in a form the compiler checks
// against the code below. `dwarf --emit-types <dir>` generates the same
// shapes from the world itself.

/** WIT `record point { x: f64, y: f64 }` */
interface Point {
    x: number;
    y: number;
}

/** WIT `enum color` - a case-name string, not a numeric enum. */
type Color = "red" | "green" | "blue";

/** WIT `flags permissions` - an object of booleans, one per flag. */
interface Permissions {
    read: boolean;
    write: boolean;
    execute: boolean;
}

/** WIT `variant shape` - `{ tag, val }`, discriminated by `tag`. */
type Shape = { tag: "circle"; val: number } | { tag: "rectangle"; val: Point };

// Numeric types (using camelCase - runtime converts from WIT kebab-case)
export function addU32(a: number, b: number): number {
    return (a + b) >>> 0; // unsigned 32-bit
}

export function addS32(a: number, b: number): number {
    return (a + b) | 0; // signed 32-bit
}

export function addF64(a: number, b: number): number {
    return a + b;
}

export function negate(b: boolean): boolean {
    return !b;
}

export function toUpper(c: string): string {
    return c.toUpperCase();
}

// Record
export function addPoints(a: Point, b: Point): Point {
    return { x: a.x + b.x, y: a.y + b.y };
}

// List
export function sumList(nums: number[]): number {
    return nums.reduce((acc, n) => acc + n, 0);
}

// Option - `T | null` is what dwarf passes and expects; `undefined` is
// accepted on the way in, never produced on the way out.
export function maybeDouble(n: number | null | undefined): number | null {
    if (n === null || n === undefined) {
        return null;
    }
    return n * 2;
}

// Top-level result returns use the JS exception convention:
// return the ok payload, or throw the err payload.
export function safeDivide(a: number, b: number): number {
    if (b === 0) {
        throw "division by zero";
    }
    return Math.floor(a / b);
}

// Enum - represented as its case-name string
export function colorName(c: Color): string {
    return c;
}

// Flags - represented as a { name: boolean } object
export function checkRead(p: Permissions): boolean {
    return p.read === true; // read flag
}

// Variant - { tag: case-name, val: payload }
export function shapeArea(s: Shape): number {
    if (s.tag === "circle") {
        // circle - val is radius
        const r = s.val;
        return Math.PI * r * r;
    } else {
        // rectangle - val is point {x, y} representing width/height
        return s.val.x * s.val.y;
    }
}
