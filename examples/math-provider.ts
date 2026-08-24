// Implement the math interface
function add(a: number, b: number): number {
    return a + b;
}

function multiply(a: number, b: number): number {
    return a * b;
}

export const math = { add, multiply };
