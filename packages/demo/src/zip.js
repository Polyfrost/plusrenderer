// ponytail: stored and deflated entries only, no Zip64 or encryption. Use a library if bundles need them.

const EOCD = 0x06054b50;
const CENTRAL = 0x02014b50;

export async function unzip(buffer) {
    const bytes = new Uint8Array(buffer);
    const view = new DataView(buffer);

    // EOCD: last 22 bytes plus a comment of up to 64 KiB.
    let eocd = -1;
    for (let i = bytes.length - 22; i >= Math.max(0, bytes.length - 22 - 0xffff); i--) {
        if (view.getUint32(i, true) === EOCD) {
            eocd = i;
            break;
        }
    }
    if (eocd < 0) throw new Error("not a zip file");

    const count = view.getUint16(eocd + 10, true);
    let p = view.getUint32(eocd + 16, true);
    const files = [];

    for (let n = 0; n < count; n++) {
        if (view.getUint32(p, true) !== CENTRAL) throw new Error("corrupt zip directory");

        const method = view.getUint16(p + 10, true);
        const size = view.getUint32(p + 20, true);
        const nameLength = view.getUint16(p + 28, true);
        const skip = nameLength + view.getUint16(p + 30, true) + view.getUint16(p + 32, true);
        const local = view.getUint32(p + 42, true);
        const name = new TextDecoder().decode(bytes.subarray(p + 46, p + 46 + nameLength));

        p += 46 + skip;
        if (name.endsWith("/")) continue;

        const start = local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true);
        const data = bytes.subarray(start, start + size);
        const body =
            method === 0 ? data
            : method === 8 ? await inflate(data)
            : null;
        if (!body) throw new Error(`${name}: unsupported compression method ${method}`);
        files.push({ path: name, file: new Blob([body]) });
    }

    return files;
}

async function inflate(data) {
    const stream = new Blob([data]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
    return new Uint8Array(await new Response(stream).arrayBuffer());
}
