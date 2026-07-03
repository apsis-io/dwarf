import { delayEcho } from "test:concurrent/api";

export async function probe() {
    const [a, b] = await Promise.all([delayEcho(1, 150), delayEcho(2, 150)]);
    return [a, b];
}
