package org.polyfrost.plusrenderer;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;
import java.util.zip.CRC32;

/**
 * The plusrenderer core over JNI: one instance is one scene of numbered slots (slot number = texture id).
 * Not thread-safe. Each frame: {@link #pose} then {@link #buildMesh}, or {@link #build} for 2D buffers.
 * Cosmetics hang off the nearest lower slot with the bone they name, so the player goes lowest.
 *
 * <p>The first use loads the native library for this platform from {@code /natives/plusrenderer/<os>-<arch>/}
 * on the classpath and throws {@link UnsatisfiedLinkError} where there is none. It extracts the library under
 * the {@code plusrenderer.natives.dir} system property, else the temp dir. Set it before first use (a game host
 * should point it at its own data dir, since antivirus heuristics distrust DLLs loaded from temp).
 */
public final class Renderer implements AutoCloseable {
    /** {@link #buildMesh} space: the core's own (Y up, front +Z, pixels). */
    public static final int CORE_SPACE = 0;
    /** {@link #buildMesh} space: a player render layer's (Y down, origin at the neck, blocks). */
    public static final int JAVA_MODEL_SPACE = 1;

    /** {@link #bundlePart} kinds. */
    public static final int GEOMETRY = 0, ANIMATION = 1, TEXTURE = 2, SHEET = 3;

    static {
        load();
    }

    private long handle = nCreate();

    public void loadPlayer(int slot, boolean slim) {
        nLoadPlayer(handle(), slot, slim);
    }

    /** Loads a cosmetic's first geometry. {@code anchor} may be empty, image size 0 if unknown. */
    public void loadCosmetic(int slot, String json, String bodySlot, String anchor, int imageWidth, int imageHeight) {
        check(nLoadCosmetic(handle(), slot, json, bodySlot, anchor, imageWidth, imageHeight));
    }

    /** Loads vanilla's cape and returns the frame count: above 1 bind the sheet, else the still. Sheet size 0 if none. */
    public int loadCape(int slot, int stillWidth, int stillHeight, int sheetWidth, int sheetHeight, String sheetName) {
        return nLoadCape(handle(), slot, stillWidth, stillHeight, sheetWidth, sheetHeight, sheetName);
    }

    public void loadAnimation(int slot, String json) {
        check(nLoadAnimation(handle(), slot, json));
    }

    /** Index into {@code paths} of the bundle's {@code kind} part, or -1. */
    public static int bundlePart(String[] paths, int kind) {
        StringBuilder names = new StringBuilder();
        for (String path : paths) names.append(names.length() == 0 ? "" : "\n").append(path);
        return nBundlePart(names.toString(), kind);
    }

    /** Starts the slot's animation from its first frame at scene time {@code at}. */
    public void play(int slot, float at) {
        nPlay(handle(), slot, at);
    }

    public void stop(int slot) {
        nStop(handle(), slot);
    }

    public void setVisible(int slot, boolean visible) {
        nSetVisible(handle(), slot, visible);
    }

    /** Scales the slot's model about the bone it hangs from. */
    public void setScale(int slot, float scale) {
        nSetScale(handle(), slot, scale);
    }

    public void clear(int slot) {
        nClear(handle(), slot);
    }

    /** Minecraft light level 0..15, where 15 (the default) is daylight. Glowing bones stay lit. */
    public void setLightLevel(float level) {
        nSetLightLevel(handle(), level);
    }

    /** Poses every slot at {@code seconds}. Call before {@link #buildMesh}. */
    public void pose(float seconds) {
        failed(nPose(handle(), seconds));
    }

    /** Sets a posed vanilla part: core name, rest and current pivot (pixels), rotation (radians), scale. */
    public void setGamePart(String name, float restX, float restY, float restZ, float x, float y, float z,
                            float xRot, float yRot, float zRot, float xScale, float yScale, float zScale) {
        nSetGamePart(handle(), name, restX, restY, restZ, x, y, z, xRot, yRot, zRot, xScale, yScale, zScale);
    }

    /** Answers a MoLang query for every slot. */
    public void setQuery(String name, float value) {
        nSetQuery(handle(), name, value);
    }

    public void clearGameParts() {
        nClearGameParts(handle());
    }

    /** The emote's delta for a part: offset xyz, rotation xyz in radians, scale xyz. Null if no such bone. */
    public float[] partDelta(int slot, String name) {
        return nPartDelta(handle(), slot, name);
    }

    /** Builds the 3D mesh from the last pose in {@code space} and returns the quad count. Four vertices per quad. */
    public int buildMesh(int space) {
        return failed(nBuildMesh(handle(), space));
    }

    /** xyz per vertex. */
    public float[] meshPositions() {
        return nMeshPositions(handle());
    }

    /** xyz per vertex. */
    public float[] meshNormals() {
        return nMeshNormals(handle());
    }

    /** uv per vertex. */
    public float[] meshUvs() {
        return nMeshUvs(handle());
    }

    /** Minimum light level 0..15 per vertex (0 if not glowing). Pack {@code max(scene, glow)}. */
    public byte[] meshGlow() {
        return nMeshGlow(handle());
    }

    /** Four ints per run: texture (the slot), first quad, quad count, cull (1 culls back faces). */
    public int[] meshRuns() {
        return nMeshRuns(handle());
    }

    /** Poses at {@code seconds} and fills the 2D buffers, back to front. Returns the index count. */
    public int build(float yaw, float pitch, float distance, float targetY, float width, float height, float seconds) {
        return failed(nBuild(handle(), yaw, pitch, distance, targetY, width, height, seconds));
    }

    /** Screen-space xy per vertex. */
    public float[] positions() {
        return nPositions(handle());
    }

    /** uv per vertex, 0..1 across the bound image. */
    public float[] uvs() {
        return nUvs(handle());
    }

    /** 0xAARRGGBB per vertex, lighting baked in. */
    public int[] colors() {
        return nColors(handle());
    }

    public int[] indices() {
        return nIndices(handle());
    }

    /** Three ints per run: texture (the slot), first index, index count. */
    public int[] runs() {
        return nRuns(handle());
    }

    @Override
    public void close() {
        if (handle != 0) nFree(handle);
        handle = 0;
    }

    private long handle() {
        if (handle == 0) throw new IllegalStateException("renderer is closed");
        return handle;
    }

    private int check(int code) {
        if (code < 0) throw new IllegalArgumentException(nError(handle));
        return code;
    }

    private static int failed(int code) {
        if (code < 0) throw new IllegalStateException("the plusrenderer core failed");
        return code;
    }

    private static void load() {
        String name = System.mapLibraryName("plusrenderer_ffi");
        String resource = "/natives/plusrenderer/" + platform() + "/" + name;

        try (InputStream in = Renderer.class.getResourceAsStream(resource)) {
            if (in == null) throw new UnsatisfiedLinkError("no plusrenderer native at " + resource);
            System.load(extract(name, readAll(in)).getAbsolutePath());
        } catch (IOException e) {
            throw (UnsatisfiedLinkError) new UnsatisfiedLinkError("could not extract " + resource).initCause(e);
        }
    }

    // Keyed by checksum so restarts reuse the extracted file.
    private static File extract(String name, byte[] bytes) throws IOException {
        CRC32 crc = new CRC32();
        crc.update(bytes);

        String root = System.getProperty("plusrenderer.natives.dir", System.getProperty("java.io.tmpdir"));
        File dir = new File(root, "plusrenderer-" + Long.toHexString(crc.getValue()));
        File file = new File(dir, name);

        if (file.length() != bytes.length) {
            Files.createDirectories(dir.toPath());
            Path partial = Files.createTempFile(dir.toPath(), name, ".part");
            Files.write(partial, bytes);
            Files.move(partial, file.toPath(), StandardCopyOption.REPLACE_EXISTING);
        }

        return file;
    }

    private static byte[] readAll(InputStream in) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buffer = new byte[1 << 16];
        for (int n; (n = in.read(buffer)) > 0; ) out.write(buffer, 0, n);
        return out.toByteArray();
    }

    /** As {@code scripts/build-native.sh} lays the natives out. */
    static String platform() {
        String os = System.getProperty("os.name").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch").toLowerCase(Locale.ROOT);

        if (arch.equals("arm64")) arch = "aarch64";
        if (arch.equals("amd64")) arch = "x86_64";

        if (os.contains("mac") || os.contains("darwin")) return "macos-" + arch;
        if (os.contains("win")) return "windows-" + arch;
        return "linux-" + arch;
    }

    private static native long nCreate();
    private static native void nFree(long r);
    private static native String nError(long r);
    private static native void nLoadPlayer(long r, int slot, boolean slim);
    private static native int nLoadCosmetic(long r, int slot, String json, String body, String anchor, int imageWidth, int imageHeight);
    private static native int nLoadCape(long r, int slot, int stillWidth, int stillHeight, int sheetWidth, int sheetHeight, String name);
    private static native int nBundlePart(String names, int kind);
    private static native int nLoadAnimation(long r, int slot, String json);
    private static native void nPlay(long r, int slot, float at);
    private static native void nStop(long r, int slot);
    private static native void nSetVisible(long r, int slot, boolean visible);
    private static native void nSetScale(long r, int slot, float scale);
    private static native void nClear(long r, int slot);
    private static native void nSetLightLevel(long r, float level);
    private static native int nPose(long r, float seconds);
    private static native void nSetGamePart(long r, String name, float restX, float restY, float restZ, float x, float y, float z,
                                            float xRot, float yRot, float zRot, float xScale, float yScale, float zScale);
    private static native void nSetQuery(long r, String name, float value);
    private static native void nClearGameParts(long r);
    private static native float[] nPartDelta(long r, int slot, String name);
    private static native int nBuildMesh(long r, int space);
    private static native float[] nMeshPositions(long r);
    private static native float[] nMeshNormals(long r);
    private static native float[] nMeshUvs(long r);
    private static native byte[] nMeshGlow(long r);
    private static native int[] nMeshRuns(long r);
    private static native int nBuild(long r, float yaw, float pitch, float distance, float targetY, float width, float height, float seconds);
    private static native float[] nPositions(long r);
    private static native float[] nUvs(long r);
    private static native int[] nColors(long r);
    private static native int[] nIndices(long r);
    private static native int[] nRuns(long r);
}
