import { offset } from "./config";
import { add } from "./lib/math.js";
import { prefix } from "local-greeter";

export function addWithOffset(a: number, b: number): number {
    return add(a, b) + offset;
}

export function greet(name: string): string {
    return `${prefix}, ${name}!`;
}
