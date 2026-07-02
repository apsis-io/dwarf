import { TcpSocket } from "wasi:sockets/types@0.3.0";

export async function probe() {
    let sock;
    try {
        sock = TcpSocket.create("ipv4");
    } catch (e) {
        return `create:threw:${JSON.stringify(e)}`;
    }

    const addr = {
        tag: "ipv4",
        val: { port: 1, address: [127, 0, 0, 1] },
    };

    try {
        await sock.connect(addr);
        return "connect:ok";
    } catch (e) {
        return `connect:threw:${JSON.stringify(e)}`;
    }
}
