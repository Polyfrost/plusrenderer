import { Renderer } from "@plusrenderer/renderer";
import { unzip } from "./zip.js";

const canvas = document.getElementById("view");
const control = (id) => document.getElementById(id);

// The player takes the lowest slot so cosmetics can hang off it.
const PLAYER = 0;
const CAPE = 1;
let nextSlot = 2;

const BODY_SLOTS = [
    "hat", "glasses", "wings", "backpack", "shoulder", "boots",
    "left_hand", "right_hand", "aura", "pet",
];

// Same clock as the rAF timestamp passed to `draw`.
const now = () => performance.now() / 1000;

const fetchOk = (url) => fetch(url).then((r) => (r.ok ? r : Promise.reject(new Error(`cannot load ${url}`))));
const fetchText = (url) => fetchOk(url).then((r) => r.text());
// createImageBitmap sniffs bytes, so `.sheet` files decode as PNGs regardless of content type.
const fetchImage = (url) => fetchOk(url).then((r) => r.blob()).then(createImageBitmap);

function showError(e) {
    console.error(e);
    control("error").textContent = String(e?.message ?? e);
}

function guessBodySlot(name) {
    const rules = [
        [/wing/, "wings"], [/boot|shoe|feet/, "boots"], [/glass|goggle|mask/, "glasses"],
        [/shoulder|pauldron/, "shoulder"], [/sword|back|bag|quiver/, "backpack"],
        [/particle|aura/, "aura"], [/glove|left_hand/, "left_hand"], [/right_hand|hand/, "right_hand"],
        [/pet/, "pet"],
    ];

    return rules.find(([re]) => re.test(name.toLowerCase()))?.[1] ?? "hat";
}

async function unpack(files) {
    const all = [];
    for (const file of files) {
        if (file.name.toLowerCase().endsWith(".zip")) all.push(...(await unzip(await file.arrayBuffer())));
        else all.push({ path: file.name, file });
    }

    return all;
}

try {
    const [renderer, skin, emote] = await Promise.all([
        Renderer.create(canvas),
        fetchImage("./skin.png"),
        fetchText("./emote.json"),
    ]);

    renderer.loadPlayer(PLAYER, false);
    renderer.setTexture(PLAYER, skin);
    control("slim").onchange = (e) => {
        renderer.loadPlayer(PLAYER, e.target.checked);
        if (control("emote").checked) renderer.play(PLAYER, now());
    };

    function setEmote(json, name) {
        renderer.loadAnimation(PLAYER, json);
        control("emote-name").textContent = name;
        if (control("emote").checked) renderer.play(PLAYER, now());
    }
    setEmote(emote, "wave");
    control("emote").onchange = (e) => (e.target.checked ? renderer.play(PLAYER, now()) : renderer.stop(PLAYER));

    function setCape(still, sheet, name) {
        renderer.loadCape(CAPE, still, sheet, name);
        renderer.setVisible(CAPE, control("cape").checked);
        control("cape-name").textContent = name;
    }
    control("cape").onchange = (e) => renderer.setVisible(CAPE, e.target.checked);

    // Changing the body slot reloads, since binding happens at load.
    async function addCosmetic({ name, body, on, source }) {
        const slot = nextSlot++;
        const row = document.createElement("div");
        row.className = "cosmetic";
        row.innerHTML = `
            <input type="checkbox"${on ? " checked" : ""}>
            <span class="name"></span>
            <select>${BODY_SLOTS.map((b) => `<option${b === body ? " selected" : ""}>${b}</option>`).join("")}</select>
            <button title="remove">✕</button>`;
        const [toggle, label, select, remove] = row.children;

        // Uploaded names are text, never markup.
        label.textContent = name.replaceAll("_", " ");
        label.title = name;
        control("cosmetics").append(row);

        const parts = await source();
        const load = () => {
            renderer.loadCosmetic(slot, parts.geometry, select.value, parts.image);
            if (parts.animation) {
                renderer.loadAnimation(slot, parts.animation);
                renderer.play(slot, now());
            }
            renderer.setVisible(slot, toggle.checked);
        };
        load();

        toggle.onchange = () => renderer.setVisible(slot, toggle.checked);
        select.onchange = () => {
            try {
                load();
            } catch (e) {
                showError(e);
            }
        };
        remove.onclick = () => {
            renderer.clear(slot);
            row.remove();
        };
    }

    // Without a kind: geometry is a cosmetic, animation an emote, sheet a cape, square image a skin, else a cape.
    async function upload(files, kind) {
        control("error").textContent = "";
        try {
            const entries = await unpack(files);
            const label = entries.map((e) => e.path).join(", ");
            const [geometry, animation, texture, sheet] =
                renderer.bundleParts(entries.map((e) => e.path)).map((i) => entries[i]);

            kind ??= geometry ? "bundle" : animation ? "emote" : sheet ? "cape" : undefined;

            if (kind === "bundle") {
                if (!geometry || !texture) throw new Error(`${label}: a bundle needs a .geo.json and a .png`);
                const name = geometry.path.split("/").pop().replace(/\.geo\.json$/i, "");
                return await addCosmetic({
                    name,
                    body: guessBodySlot(name),
                    on: true,
                    source: async () => ({
                        geometry: await geometry.file.text(),
                        image: await createImageBitmap(texture.file),
                        animation: animation ? await animation.file.text() : null,
                    }),
                });
            }
            if (kind === "emote") {
                if (!animation) throw new Error(`${label}: no animation (.json)`);
                return setEmote(await animation.file.text(), animation.path);
            }

            if (!texture) throw new Error(`${label}: no image (.png)`);
            const image = await createImageBitmap(texture.file);
            if (kind === "skin" || (!kind && image.width === image.height)) {
                return renderer.setTexture(PLAYER, image);
            }

            control("cape").checked = true;
            setCape(image, sheet && (await createImageBitmap(sheet.file)), sheet?.path ?? texture.path);
        } catch (e) {
            showError(e);
        }
    }

    for (const [id, kind] of [["skin-file", "skin"], ["emote-file", "emote"], ["cape-file", "cape"], ["bundle-file", "bundle"]]) {
        control(id).onchange = async (e) => {
            await upload(e.target.files, kind);
            e.target.value = "";
        };
    }

    document.ondragover = (e) => {
        e.preventDefault();
        document.body.classList.add("dragging");
    };
    document.ondragleave = () => document.body.classList.remove("dragging");
    document.ondrop = (e) => {
        e.preventDefault();
        document.body.classList.remove("dragging");
        upload(e.dataTransfer.files);
    };

    control("light").oninput = (e) => renderer.setLightLevel(Number(e.target.value));

    const wrap = (deg) => ((deg + 540) % 360) - 180;
    let drag = null;
    canvas.onpointerdown = (e) => {
        drag = { x: e.clientX, y: e.clientY };
        canvas.setPointerCapture(e.pointerId);
    };
    canvas.onpointerup = () => (drag = null);
    canvas.onpointermove = (e) => {
        if (!drag) return;

        const yaw = control("yaw");
        const pitch = control("pitch");
        yaw.value = wrap(Number(yaw.value) + (e.clientX - drag.x) * 0.6);
        pitch.value = Number(pitch.value) - (e.clientY - drag.y) * 0.4;
        drag = { x: e.clientX, y: e.clientY };
        control("spin").checked = false;
    };

    let frames = 0;
    let since = performance.now();
    let fps = 0;
    const frame = (ms) => {
        const seconds = ms / 1000;
        if (control("spin").checked) control("yaw").value = wrap(seconds * 40);

        const { triangles, draws } = renderer.draw(
            {
                yaw: Number(control("yaw").value),
                pitch: Number(control("pitch").value),
                distance: Number(control("distance").value),
                targetY: 18,
            },
            seconds,
        );

        frames += 1;
        if (ms - since >= 500) {
            fps = Math.round((frames * 1000) / (ms - since));
            frames = 0;
            since = ms;
        }

        control("stats").textContent = `${fps} fps · ${triangles} triangles · ${draws} draws · ${canvas.width}x${canvas.height}`;
        requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
} catch (e) {
    showError(e);
}
