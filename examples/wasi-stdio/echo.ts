// WASI 0.3 stdio: a `stream<u8>` handed straight from stdin to stdout, and
// the `future<result<_, error-code>>` that reports how it ended.
//
// The imported interfaces are declared in ../wit-modules.d.ts.
import stdin from "wasi:cli/stdin@0.3.0";
import stdout from "wasi:cli/stdout@0.3.0";

export const run = {
    async run(): Promise<void> {
        const [input, status] = stdin.readViaStream();
        const written = await stdout.writeViaStream(input);
        if (written.tag === "err") {
            throw written.val;
        }
        const statusResult = await status.read();
        if (statusResult.tag === "err") {
            throw statusResult.val;
        }
    },
};
