import { Thing, consume } from "test:resarg/api";

export async function probe() {
    const t = new Thing();
    return await consume(t);
}
