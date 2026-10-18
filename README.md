# plusrenderer

A universal renderer for Minecraft players, cosmetics and emotes.

The Rust core reads Bedrock `.geo.json` and `.animation.json` files, evaluates MoLang and attaches
cosmetics the same way the in-game client (PolyPlus) does. It then produces vertex buffers sorted
back to front, which a host draws one texture run at a time.

Every host shares that core. The web build draws with WebGL2. JVM hosts load the native library
(`crates/ffi`) through JNI (`java/`) or its C ABI (JNA). Rust hosts can draw with Skia
(`crates/skia`).

## Running the demo

```bash
pnpm install
pnpm dev
```

The demo opens at <http://localhost:5173>. Drag to orbit the player. Drop a cosmetic bundle (a `.zip`,
or a `.geo.json` with its `.png`), an emote, a cape or a skin anywhere on the page to load it.

| Command | What it does |
|---|---|
| `pnpm dev` | Builds the wasm, then starts the demo dev server |
| `pnpm build` | Writes a production bundle to `packages/demo/dist` |
| `pnpm preview` | Serves that bundle |
| `pnpm wasm` | Rebuilds only the wasm |
| `pnpm test` | Runs the Rust test suite |

The wasm build pins `cargo +1.97` because the nightly `rust-lld` can't link wasm here.

To build the native library, run `scripts/build-native.sh <resources-dir> [rust-target...]`.

## Using it

Releases are cut from tags. Pushing one runs `.github/workflows/release.yml`:

```bash
git tag v0.2.0 && git push --tags
```

The workflow needs the repository variable `REPOSILITE_URL` and the secrets `REPOSILITE_USERNAME`
and `REPOSILITE_TOKEN`.

### Java 8+

The jar in `java/` uses JNI and ships the native library for every platform.

```kotlin
repositories { maven("<REPOSILITE_URL>") }
dependencies { implementation("org.polyfrost:plusrenderer:0.2.0") }
```

```java
try (Renderer r = new Renderer()) {
    r.loadPlayer(0, false);
    r.pose(0);
    r.buildMesh(Renderer.JAVA_MODEL_SPACE);
}
```

To build and test it locally, run `scripts/build-native.sh target/java-resources`, then
`./gradlew test` in `java/`. Gradle itself needs JDK 17 or newer, but it compiles and tests on a
Java 8 toolchain.

### Rust

The launcher depends on the git tag directly. Cargo finds each crate inside the workspace.

```toml
plusrenderer = { git = "https://github.com/<owner>/<repo>", tag = "v0.2.0" }
# Optional, for drawing with Skia or Freya.
plusrenderer-skia = { git = "https://github.com/<owner>/<repo>", tag = "v0.2.0" }
```

If the repository is private, set `net.git-fetch-with-cli = true` in the launcher's
`.cargo/config.toml` so Cargo uses your git credentials.

### JavaScript

`@plusrenderer/renderer` combines WebGL2 and wasm. Every release attaches it as
`plusrenderer-renderer-<version>.tgz`.

```bash
pnpm add https://github.com/<owner>/<repo>/releases/download/v0.2.0/plusrenderer-renderer-0.2.0.tgz
```

```js
import { Renderer } from "@plusrenderer/renderer";

// Fetches plusrenderer.wasm from next to index.js.
const renderer = await Renderer.create(canvas);
```

## Layout

| Path | Package | Contents |
|---|---|---|
| `crates/core` | `plusrenderer` | Parsing, MoLang, animation, posing and vertex buffers |
| `crates/ffi` | `plusrenderer-ffi` | C ABI over the core for wasm and native hosts, plus JNI for `java/` |
| `crates/skia` | `plusrenderer-skia` | Skia backend on Freya's `freya-skia-safe`, plus a Freya viewer |
| `packages/renderer` | `@plusrenderer/renderer` | WebGL2 adapter |
| `packages/demo` | `@plusrenderer/demo` | Test page |
| `java` | `org.polyfrost:plusrenderer` | Java 8 JNI binding |

## Examples

Render the core straight to SVG:

```bash
cargo run -p plusrenderer --example dump_svg > out.svg
```

Draw the player in a Freya window:

```bash
cargo run -p plusrenderer-skia --example freya_viewer
```

The viewer calls `plusrenderer_skia::draw` from a rect's `RenderCallback` background, and Freya picks
Metal, Vulkan or GL. Drag to orbit, scroll to zoom and press space to play the emote.

`freya-skia-safe` must stay on the version Freya uses (0.104 for freya 0.5.0-rc.10), so that both
link the same Skia.

## Debug skin

`packages/demo/public/skin.png` is a 64x64 skin where colour marks direction. Red is the model's
right, which is the viewer's left when facing it. Blue is its left, green is up and yellow is down.
Quadrant markers on the head's top and bottom and on the right sole show any flipped face.
