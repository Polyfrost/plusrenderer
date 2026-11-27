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

    /** {@link #setElytraPose} poses. Swimming keeps the standing wings. */
    public static final int ELYTRA_STANDING = 0, ELYTRA_CROUCHING = 1, ELYTRA_FLYING = 2;

    public static final int SLOTS = 256;
    public static final int SIGNAL_COUNT = 23;
    public static final int PART_FLOATS = 12;
    public static final int DETAIL_FULL = 0, DETAIL_REDUCED = 1, DETAIL_MINIMAL = 2;
    public static final int MOTION_FULL = 0, MOTION_REDUCED = 1, MOTION_OFF = 2;
    public static final int PHYSICS_ON = 0, PHYSICS_OWN = 1, PHYSICS_OFF = 2;
    public static final int EVENT_HURT = 1, EVENT_ATTACK_SWING = 1 << 1, EVENT_DEATH = 1 << 2, EVENT_RESPAWN = 1 << 3,
            EVENT_DIMENSION_CHANGE = 1 << 4, EVENT_SLEEP = 1 << 5, EVENT_JUMP = 1 << 6, EVENT_EQUIP = 1 << 7;
    public static final long PREVIEW_WEARER = -1L;
    public static final int EFFECT_EVENT_INTS = 6;
    public static final int EFFECT_PARTICLE = 0, EFFECT_SOUND = 1, EFFECT_VANILLA_PARTICLE = 2;
    public static final int EFFECT_EVENT_FLOATS = 5, EFFECT_NO_CONTROLLER = -1;
    public static final int SOURCE_KEYFRAME = 0, SOURCE_REACTION = 1, SOURCE_EVENT = 2;
    public static final float EVENT_EFFECT_SECONDS = 3f, EFFECT_PARTICLE_LIFE_SECONDS = 3.1f;
    public static final int RIG_PLAYER = 0, RIG_ACTOR = 2, RIG_BOAT = 3, RIG_MINECART = 4, RIG_HORSE = 5;
    public static final int ROOT_FLOATS = 12, PLAY_FLOATS = 2, ROLE_FLOATS = 5, HOLD_FLOATS = 1;
    public static final int LOCATOR_FLOATS = 12, COLORWAY_FLOATS = 1, PART_DELTA_FLOATS = 10;
    public static final int PART_DELTA_VALUES = 9;
    public static final float NO_COLORWAY = -1f;
    public static final int SECTION_PARTS = 0, SECTION_QUERIES = 1, SECTION_ROOT = 2, SECTION_VARIABLES = 3,
            SECTION_PLAY = 4, SECTION_ROLE = 5, SECTION_COLORWAY = 6, SECTION_PART_DELTAS = 7, SECTION_HOLD = 8;
    public static final int SECTION_INTS = 9;
    public static final int INPUT_SIGNALS_AT = 0, INPUT_EVENTS_AT = 1, INPUT_DETAIL_AT = 2, INPUT_ROOT_AT = 3,
            INPUT_VARIABLES_AT = 4, INPUT_PLAY_AT = 5, INPUT_ROLE_AT = 6, INPUT_STRIDE = 7, INPUT_HOLD_AT = 8;
    public static final int INPUT_LAYOUT_INTS = 9;
    public static final int LAYOUT_BONES = 0, LAYOUT_RUNS = 1, LAYOUT_LOCATORS = 2, LAYOUT_COLORWAY = 3,
            LAYOUT_PART_DELTAS = 4, LAYOUT_RUNS_AT = 5, LAYOUT_LOCATORS_AT = 6, LAYOUT_COLORWAY_AT = 7,
            LAYOUT_PART_DELTAS_AT = 8, LAYOUT_STRIDE = 9, LAYOUT_EMOTE = 10, LAYOUT_EMOTE_AT = 11;
    public static final int LAYOUT_INTS = 12;
    public static final int EMOTE_STATUS_FLOATS = 1;
    public static final int EMOTE_POSING = 1, EMOTE_ENDED = 1 << 1, EMOTE_CANCEL = 1 << 2, EMOTE_STOPPING = 1 << 3,
            EMOTE_CANCEL_WALKING = 1 << 4, EMOTE_CANCEL_SPRINTING = 1 << 5, EMOTE_CANCEL_CROUCHING = 1 << 6,
            EMOTE_CANCEL_FALLING = 1 << 7, EMOTE_CANCEL_ELYTRA_FLYING = 1 << 8, EMOTE_CANCEL_SWIMMING = 1 << 9,
            EMOTE_CANCEL_SLEEPING = 1 << 10, EMOTE_CANCEL_SWINGING = 1 << 11, EMOTE_HELD = 1 << 12;
    public static final int PLAY_CLIP = 0, PLAY_AGE = 1;
    public static final int ROLE_INDEX = 0, ROLE_DX = 1, ROLE_DY = 2, ROLE_DZ = 3, ROLE_YAW = 4;
    public static final int VIEW_FLOATS = 12;
    public static final int VIEW_CAMERA = 0, VIEW_RIGHT = 3, VIEW_UP = 6, VIEW_ORIGIN_SHIFT = 9;
    public static final int QUAD_FLOATS = 27, QUAD_TEXTURE = 0, QUAD_BLEND = 1, QUAD_COLOR = 2, QUAD_CORNERS = 6,
            CORNER_FLOATS = 5, QUAD_FLAGS = 26;
    public static final int QUAD_FLAG_EMISSIVE = 1;
    public static final float BLEND_ALPHA = 0f, BLEND_ADDITIVE = 1f;

    public static final class ControllerState {
        public final String state;
        public final float seconds;

        ControllerState(String state, float seconds) {
            this.state = state;
            this.seconds = seconds;
        }
    }

    static {
        load();
    }

    private long handle = nCreate();

    public synchronized void loadPlayer(int slot, boolean slim) {
        nLoadPlayer(handle(), slot(slot), slim);
    }

    /** Loads a cosmetic's first geometry. {@code anchor} may be empty, image size 0 if unknown. */
    public synchronized void loadCosmetic(int slot, String json, String bodySlot, String anchor, int imageWidth, int imageHeight) {
        check(nLoadCosmetic(handle(), slot(slot), json, bodySlot, anchor, imageWidth, imageHeight));
    }

    /** Loads vanilla's cape and returns the frame count: above 1 bind the sheet, else the still. Sheet size 0 if none. */
    public synchronized int loadCape(int slot, int stillWidth, int stillHeight, int sheetWidth, int sheetHeight, String sheetName) {
        return nLoadCape(handle(), slot(slot), stillWidth, stillHeight, sheetWidth, sheetHeight, sheetName);
    }

    /** As {@link #loadCape}, but vanilla's elytra, drawn with the same cape texture or sheet. */
    public synchronized int loadElytra(int slot, int stillWidth, int stillHeight, int sheetWidth, int sheetHeight, String sheetName) {
        return nLoadElytra(handle(), slot(slot), stillWidth, stillHeight, sheetWidth, sheetHeight, sheetName);
    }

    public synchronized void loadAnimation(int slot, String json) {
        check(nLoadAnimation(handle(), slot(slot), json));
    }

    public synchronized void loadControllers(int slot, String animationJson, String[] names, byte[][] files, String[] declared) {
        slot(slot);
        if (names.length != files.length) throw new IllegalArgumentException(names.length + " names for " + files.length + " files");
        int total = 0;
        int[] sizes = new int[files.length];
        for (int i = 0; i < files.length; i++) {
            sizes[i] = files[i].length;
            total += sizes[i];
        }
        byte[] data = new byte[total];
        for (int i = 0, at = 0; i < files.length; at += sizes[i], i++) {
            System.arraycopy(files[i], 0, data, at, sizes[i]);
        }
        check(nLoadControllers(handle(), slot, animationJson, joined(names), data, sizes, joined(declared)));
    }

    public static int[] bundleControllers(String[] paths) {
        return nBundleControllers(joined(paths));
    }

    public synchronized int effectEvents(int[] out) {
        return check(nEffectEvents(handle(), out));
    }

    public synchronized ControllerState controllerState(int slot, long wearer, int controller) {
        float[] seconds = new float[1];
        String state = nControllerState(handle(), slot, wearer, controller, seconds);
        return state == null ? null : new ControllerState(state, seconds[0]);
    }

    public synchronized String controllerState(int slot, long wearer, int controller, float[] seconds) {
        if (seconds == null || seconds.length < 1) throw new IllegalArgumentException("seconds must hold one float");
        return nControllerState(handle(), slot, wearer, controller, seconds);
    }

    private static String joined(String[] names) {
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < names.length; i++) out.append(i == 0 ? "" : "\n").append(names[i]);
        return out.toString();
    }

    /** Index into {@code paths} of the bundle's {@code kind} part, or -1. */
    public static int bundlePart(String[] paths, int kind) {
        StringBuilder names = new StringBuilder();
        for (String path : paths) names.append(names.length() == 0 ? "" : "\n").append(path);
        return nBundlePart(names.toString(), kind);
    }

    /** Starts the slot's animation from its first frame at scene time {@code at}. */
    public synchronized void play(int slot, double at) {
        nPlay(handle(), slot(slot), at);
    }

    public synchronized void play(int slot, float at) {
        play(slot, (double) at);
    }

    public synchronized void play(int slot, long at) {
        play(slot, (double) at);
    }

    public synchronized void stop(int slot) {
        nStop(handle(), slot(slot));
    }

    /** {@code fall} is how steeply a flying player dives, 0..1: {@code max(0, -velocity.normalize().y)}. */
    public synchronized void setElytraPose(int slot, int pose, float fall) {
        nSetElytraPose(handle(), slot(slot), pose, fall);
    }

    public synchronized void setVisible(int slot, boolean visible) {
        nSetVisible(handle(), slot(slot), visible);
    }

    /** Scales the slot's model about the bone it hangs from. */
    public synchronized void setScale(int slot, float scale) {
        nSetScale(handle(), slot(slot), scale);
    }

    public synchronized void clear(int slot) {
        nClear(handle(), slot(slot));
    }

    /** Minecraft light level 0..15, where 15 (the default) is daylight. Glowing bones stay lit. */
    public synchronized void setLightLevel(float level) {
        nSetLightLevel(handle(), level);
    }

    /** Poses every slot at {@code seconds}. Call before {@link #buildMesh}. */
    public synchronized void pose(float seconds) {
        failed(nPose(handle(), seconds));
    }

    public synchronized void pose(double seconds) {
        failed(nPoseF64(handle(), seconds));
    }

    public synchronized void pose(long seconds) {
        pose((double) seconds);
    }

    /** Sets a posed vanilla part: core name, rest and current pivot (pixels), rotation (radians), scale. */
    public synchronized void setGamePart(String name, float restX, float restY, float restZ, float x, float y, float z,
                                         float xRot, float yRot, float zRot, float xScale, float yScale, float zScale) {
        nSetGamePart(handle(), name, restX, restY, restZ, x, y, z, xRot, yRot, zRot, xScale, yScale, zScale);
    }

    /** Answers a MoLang query for every slot. */
    public synchronized void setQuery(String name, float value) {
        nSetQuery(handle(), name, value);
    }

    public synchronized void clearGameParts() {
        nClearGameParts(handle());
    }

    /** The emote's delta for a part: offset xyz, rotation xyz in radians, scale xyz. Null if no such bone. */
    public synchronized float[] partDelta(int slot, String name) {
        return nPartDelta(handle(), slot, name);
    }

    public synchronized boolean partDelta(int slot, String name, float[] out) {
        if (name == null) throw new NullPointerException("name");
        if (out == null || out.length < PART_DELTA_VALUES) {
            throw new IllegalArgumentException("out must hold " + PART_DELTA_VALUES + " floats");
        }
        return nPartDeltaInto(handle(), slot, name, out);
    }

    /** Builds the 3D mesh from the last pose in {@code space} and returns the quad count. Four vertices per quad. */
    public synchronized int buildMesh(int space) {
        return failed(nBuildMesh(handle(), space));
    }

    /** xyz per vertex. */
    public synchronized float[] meshPositions() {
        return nMeshPositions(handle());
    }

    /** xyz per vertex. */
    public synchronized float[] meshNormals() {
        return nMeshNormals(handle());
    }

    /** uv per vertex. */
    public synchronized float[] meshUvs() {
        return nMeshUvs(handle());
    }

    /** Minimum light level 0..15 per vertex (0 if not glowing). Pack {@code max(scene, glow)}. */
    public synchronized byte[] meshGlow() {
        return nMeshGlow(handle());
    }

    /** Four ints per run: texture (the slot), first quad, quad count, cull (1 culls back faces). */
    public synchronized int[] meshRuns() {
        return nMeshRuns(handle());
    }

    public synchronized int partId(String name) {
        return nPartId(handle(), name);
    }

    public synchronized int queryId(String name) {
        return nQueryId(handle(), name);
    }

    public synchronized int buildSkin() {
        return failed(nBuildSkin(handle()));
    }

    public synchronized int layout() {
        return nLayout(handle());
    }

    public synchronized int skinBoneCount() {
        return nSkinBoneCount(handle());
    }

    public synchronized float[] skinPositions() {
        return nSkinPositions(handle());
    }

    public synchronized float[] skinNormals() {
        return nSkinNormals(handle());
    }

    public synchronized float[] skinUvs() {
        return nSkinUvs(handle());
    }

    public synchronized byte[] skinGlow() {
        return nSkinGlow(handle());
    }

    public synchronized int[] skinBones() {
        return nSkinBones(handle());
    }

    public synchronized int[] skinRuns() {
        return nSkinRuns(handle());
    }

    public synchronized int frame(float seconds, float[] input, int partCount, int queryCount, int space, float[] out) {
        return check(nFrame(handle(), seconds, input, partCount, queryCount, space, out));
    }

    public synchronized int frame(double seconds, float[] input, int partCount, int queryCount, int space, float[] out) {
        return check(nFrameF64(handle(), seconds, input, partCount, queryCount, space, out));
    }

    public synchronized int frame(long seconds, float[] input, int partCount, int queryCount, int space, float[] out) {
        return frame((double) seconds, input, partCount, queryCount, space, out);
    }

    public static int wearerInputStride(int partCount, int queryCount) {
        return partCount * PART_FLOATS + queryCount + SIGNAL_COUNT + 2;
    }

    public static int wearerSignalsOffset(int partCount, int queryCount) {
        return partCount * PART_FLOATS + queryCount;
    }

    public static int wearerEventsOffset(int partCount, int queryCount) {
        return wearerSignalsOffset(partCount, queryCount) + SIGNAL_COUNT;
    }

    public static int wearerDetailOffset(int partCount, int queryCount) {
        return wearerEventsOffset(partCount, queryCount) + 1;
    }

    public synchronized int wearerOutputStride() {
        return checkFrame(nWearerOutputStride(handle()));
    }

    public synchronized int frameWearers(double clock, long[] wearers, int count, float[] input, int partCount, int queryCount,
                                         int space, float[] out) {
        return checkFrame(nFrameWearers(handle(), clock, wearers, count, input, partCount, queryCount, space, out));
    }

    public synchronized void setMotion(int motion) {
        check(nSetMotion(handle(), motion));
    }

    public synchronized void setPhysics(int mode, long own) {
        check(nSetPhysics(handle(), mode, own));
    }

    public synchronized boolean forgetWearer(long wearer) {
        return nForgetWearer(handle(), wearer);
    }

    public synchronized boolean breakRings(long wearer) {
        return nBreakRings(handle(), wearer);
    }

    public synchronized void setSignalOverride(long wearer, int signal, float value) {
        check(nSetSignalOverride(handle(), wearer, signal, value));
    }

    public synchronized void setSeedWearer(long wearer, long seedWearer) {
        nSetSeedWearer(handle(), wearer, seedWearer);
    }

    public synchronized int wearerCount() {
        return nWearerCount(handle());
    }

    public synchronized int wearerBytes() {
        return nWearerBytes(handle());
    }

    public synchronized void loadAsset(int slot, CosmeticAsset asset) {
        long r = handle();
        asset.lock.readLock().lock();
        try {
            checkFrame(nLoadAsset(r, slot(slot), asset.handle()));
        } finally {
            asset.lock.readLock().unlock();
        }
    }

    public synchronized void loadEmote(int slot, int propsSlot, CosmeticAsset asset) {
        slot(slot);
        if (propsSlot >= 0) slot(propsSlot);
        long r = handle();
        asset.lock.readLock().lock();
        try {
            checkFrame(nLoadEmote(r, slot, propsSlot, asset.handle()));
        } finally {
            asset.lock.readLock().unlock();
        }
    }

    public synchronized int frameWearersShaped(double clock, long[] wearers, int count, int[] sections, float[] input, int space,
                                               float[] out) {
        return checkFrame(nFrameWearersShaped(handle(), clock, wearers, count, sections, input, space, out));
    }

    public synchronized int outputLayout(int[] sections, int[] out) {
        return checkFrame(nOutputLayout(handle(), sections, out));
    }

    public static int inputLayout(int[] sections, int[] out) {
        return nInputLayout(sections, out);
    }

    public synchronized int hostVariableId(String name) {
        if (name == null) throw new NullPointerException("name");
        return check(nHostVariableId(handle(), name));
    }

    public synchronized int locatorId(String name) {
        if (name == null) throw new NullPointerException("name");
        int id = nLocatorId(handle(), name);
        if (id == -2) throw new IllegalStateException(nError(handle));
        return id;
    }

    public synchronized void setEffectColors(long wearer, int colorway, int[] customSlots, int[] customRgb) {
        int a = customSlots == null ? 0 : customSlots.length;
        int b = customRgb == null ? 0 : customRgb.length;
        if (a != b) throw new IllegalArgumentException(a + " custom slots for " + b + " colors");
        check(nSetEffectColors(handle(), wearer, colorway, customSlots, customRgb));
    }

    public synchronized boolean fireEffect(long wearer, String name) {
        if (name == null) throw new NullPointerException("name");
        return check(nFireEffect(handle(), wearer, name)) == 1;
    }

    public synchronized void setRig(int rig) {
        check(nSetRig(handle(), rig));
    }

    public synchronized boolean hasEffects() {
        return nHasEffects(handle());
    }

    public synchronized int effectGeometry(float[] view, float[] out) {
        return checkFrame(nEffectGeometry(handle(), view, out));
    }

    public synchronized int effectEventFloats(float[] out) {
        return check(nEffectEventFloats(handle(), out));
    }

    public synchronized int effectTextureCount() {
        return nEffectTextureCount(handle());
    }

    public synchronized String effectTexture(int id, int[] slot) {
        return nEffectTexture(handle(), id, slot);
    }

    public static long pairSeed(long a, long b, long minute) {
        return nPairSeed(a, b, minute);
    }

    public static long actorWearer(long owner, int slot) {
        if (slot < 0 || slot > 255) throw new IllegalArgumentException("no actor slot " + slot);
        return nActorWearer(owner, slot);
    }

    public static long eventWearer(long owner, int event, int seq) {
        if (event < 0 || event > 255) throw new IllegalArgumentException("no event " + event);
        return nEventWearer(owner, event, seq);
    }

    public static long eventWearer(long owner, int event, int seq, int nonce) {
        if (event < 0 || event > 255) throw new IllegalArgumentException("no event " + event);
        return nEventKey(owner, event, seq, nonce);
    }

    synchronized void faultNextFrame() {
        nFault(handle());
    }

    /** Poses at {@code seconds} and fills the 2D buffers, back to front. Returns the index count. */
    public synchronized int build(float yaw, float pitch, float distance, float targetY, float width, float height, float seconds) {
        return failed(nBuild(handle(), yaw, pitch, distance, targetY, width, height, seconds));
    }

    public synchronized int build(float yaw, float pitch, float distance, float targetY, float width, float height, double seconds) {
        return failed(nBuildF64(handle(), yaw, pitch, distance, targetY, width, height, seconds));
    }

    public synchronized int build(float yaw, float pitch, float distance, float targetY, float width, float height, long seconds) {
        return build(yaw, pitch, distance, targetY, width, height, (double) seconds);
    }

    /** Screen-space xy per vertex. */
    public synchronized float[] positions() {
        return nPositions(handle());
    }

    /** uv per vertex, 0..1 across the bound image. */
    public synchronized float[] uvs() {
        return nUvs(handle());
    }

    /** 0xAARRGGBB per vertex, lighting baked in. */
    public synchronized int[] colors() {
        return nColors(handle());
    }

    public synchronized int[] indices() {
        return nIndices(handle());
    }

    /** Three ints per run: texture (the slot), first index, index count. */
    public synchronized int[] runs() {
        return nRuns(handle());
    }

    @Override
    public synchronized void close() {
        if (handle != 0) nFree(handle);
        handle = 0;
    }

    private long handle() {
        if (handle == 0) throw new IllegalStateException("renderer is closed");
        return handle;
    }

    private static int slot(int slot) {
        if (slot < 0 || slot >= SLOTS) throw new IllegalArgumentException("no slot " + slot + ", the last is " + (SLOTS - 1));
        return slot;
    }

    private int check(int code) {
        if (code < 0) throw new IllegalArgumentException(nError(handle));
        return code;
    }

    private int checkFrame(int code) {
        if (code == -2) throw new IllegalStateException(nError(handle));
        return check(code);
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

    private static native int nPartId(long r, String name);
    private static native int nQueryId(long r, String name);
    private static native int nBuildSkin(long r);
    private static native int nLayout(long r);
    private static native int nSkinBoneCount(long r);
    private static native float[] nSkinPositions(long r);
    private static native float[] nSkinNormals(long r);
    private static native float[] nSkinUvs(long r);
    private static native byte[] nSkinGlow(long r);
    private static native int[] nSkinBones(long r);
    private static native int[] nSkinRuns(long r);
    private static native int nFrame(long r, float seconds, float[] input, int partCount, int queryCount, int space, float[] out);
    private static native int nFrameF64(long r, double seconds, float[] input, int partCount, int queryCount, int space, float[] out);
    private static native int nFrameWearers(long r, double clock, long[] wearers, int count, float[] input, int partCount,
                                            int queryCount, int space, float[] out);
    private static native int nWearerOutputStride(long r);
    private static native int nSetMotion(long r, int motion);
    private static native int nSetPhysics(long r, int mode, long own);
    private static native boolean nForgetWearer(long r, long wearer);
    private static native boolean nBreakRings(long r, long wearer);
    private static native int nSetSignalOverride(long r, long wearer, int signal, float value);
    private static native void nSetSeedWearer(long r, long wearer, long seedWearer);
    private static native int nWearerCount(long r);
    private static native int nWearerBytes(long r);
    private static native int nLoadAsset(long r, int slot, long asset);
    private static native int nLoadEmote(long r, int slot, int props, long asset);
    private static native int nFrameWearersShaped(long r, double clock, long[] wearers, int count, int[] sections,
                                                  float[] input, int space, float[] out);
    private static native int nOutputLayout(long r, int[] sections, int[] out);
    private static native int nInputLayout(int[] sections, int[] out);
    private static native int nHostVariableId(long r, String name);
    private static native int nLocatorId(long r, String name);
    private static native int nSetRig(long r, int rig);
    private static native int nFireEffect(long r, long wearer, String name);
    private static native int nSetEffectColors(long r, long wearer, int colorway, int[] slots, int[] rgb);
    private static native boolean nHasEffects(long r);
    private static native int nEffectGeometry(long r, float[] view, float[] out);
    private static native int nEffectEventFloats(long r, float[] out);
    private static native int nEffectTextureCount(long r);
    private static native String nEffectTexture(long r, int id, int[] slot);
    private static native long nPairSeed(long a, long b, long minute);
    private static native long nActorWearer(long owner, int slot);
    private static native long nEventWearer(long owner, int event, int seq);
    private static native long nEventKey(long owner, int event, int seq, int nonce);
    private static native void nFault(long r);
    private static native long nCreate();
    private static native void nFree(long r);
    private static native String nError(long r);
    private static native void nLoadPlayer(long r, int slot, boolean slim);
    private static native int nLoadCosmetic(long r, int slot, String json, String body, String anchor, int imageWidth, int imageHeight);
    private static native int nLoadCape(long r, int slot, int stillWidth, int stillHeight, int sheetWidth, int sheetHeight, String name);
    private static native int nLoadElytra(long r, int slot, int stillWidth, int stillHeight, int sheetWidth, int sheetHeight, String name);
    private static native int nBundlePart(String names, int kind);
    private static native int nLoadAnimation(long r, int slot, String json);
    private static native int nLoadControllers(long r, int slot, String json, String names, byte[] data, int[] sizes, String declared);
    private static native int[] nBundleControllers(String names);
    private static native int nEffectEvents(long r, int[] out);
    private static native String nControllerState(long r, int slot, long wearer, int controller, float[] seconds);
    private static native void nPlay(long r, int slot, double at);
    private static native void nStop(long r, int slot);
    private static native void nSetElytraPose(long r, int slot, int pose, float fall);
    private static native void nSetVisible(long r, int slot, boolean visible);
    private static native void nSetScale(long r, int slot, float scale);
    private static native void nClear(long r, int slot);
    private static native void nSetLightLevel(long r, float level);
    private static native int nPose(long r, float seconds);
    private static native int nPoseF64(long r, double seconds);
    private static native void nSetGamePart(long r, String name, float restX, float restY, float restZ, float x, float y, float z,
                                            float xRot, float yRot, float zRot, float xScale, float yScale, float zScale);
    private static native void nSetQuery(long r, String name, float value);
    private static native void nClearGameParts(long r);
    private static native float[] nPartDelta(long r, int slot, String name);
    private static native boolean nPartDeltaInto(long r, int slot, String name, float[] out);
    private static native int nBuildMesh(long r, int space);
    private static native float[] nMeshPositions(long r);
    private static native float[] nMeshNormals(long r);
    private static native float[] nMeshUvs(long r);
    private static native byte[] nMeshGlow(long r);
    private static native int[] nMeshRuns(long r);
    private static native int nBuild(long r, float yaw, float pitch, float distance, float targetY, float width, float height, float seconds);
    private static native int nBuildF64(long r, float yaw, float pitch, float distance, float targetY, float width, float height, double seconds);
    private static native float[] nPositions(long r);
    private static native float[] nUvs(long r);
    private static native int[] nColors(long r);
    private static native int[] nIndices(long r);
    private static native int[] nRuns(long r);
}
