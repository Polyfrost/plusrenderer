package org.polyfrost.plusrenderer;

public final class Nameplates implements AutoCloseable {
    public static final int SIGNAL_COUNT = Renderer.SIGNAL_COUNT;
    public static final int TAG_INTS = 24;
    public static final int TAG_ASSET = 0;
    public static final int TAG_VARIANT = 1;
    public static final int TAG_FLAGS = 2;
    public static final int TAG_DETAIL = 3;
    public static final int TAG_LINE_COUNT = 4;
    public static final int TAG_NAME_LINE = 5;
    public static final int TAG_ALIGN = 6;
    public static final int TAG_NAME_X = 7;
    public static final int TAG_NAME_WIDTH = 8;
    public static final int TAG_PREFIX_WIDTH = 9;
    public static final int TAG_LINE_WIDTH_0 = 10;
    public static final int TAG_LINE_WIDTH_1 = 11;
    public static final int TAG_LINE_WIDTH_2 = 12;
    public static final int TAG_BG_X0 = 13;
    public static final int TAG_BG_Y0 = 14;
    public static final int TAG_BG_X1 = 15;
    public static final int TAG_BG_Y1 = 16;
    public static final int TAG_EXTERNAL_COLOR = 17;
    public static final int TAG_SNEAK_ALPHA = 18;
    public static final int TAG_GLYPH_OFFSET = 19;
    public static final int TAG_GLYPH_COUNT = 20;
    public static final int TAG_COLOR_OFFSET = 21;
    public static final int TAG_COLOR_COUNT = 22;
    public static final int TAG_RESERVED = 23;
    public static final int FLAG_DISPLAY = 1 << 0;
    public static final int FLAG_OWN = 1 << 1;
    public static final int FLAG_ANIMATED_NAMES_OFF = 1 << 2;
    public static final int FLAG_EFFECTS_CPU = 1 << 3;
    public static final int FLAG_EFFECTS_OFF = 1 << 4;
    public static final int FLAG_EXTERNAL_COLOR = 1 << 5;
    public static final int FLAG_BADGE = 1 << 6;
    public static final int GLYPH_FLOATS = 2;
    public static final int GLYPH_X = 0;
    public static final int GLYPH_WIDTH = 1;
    public static final int SIGNAL_FLOATS = SIGNAL_COUNT + 1;
    public static final int SIGNAL_EVENTS = SIGNAL_COUNT;
    public static final int OUT_TAGS = 0;
    public static final int OUT_WORDS = 1;
    public static final int OUT_STRIP_LAYOUT = 2;
    public static final int OUT_HEADER = 3;
    public static final int DIR_WORDS = 3;
    public static final int DIR_OFFSET = 0;
    public static final int DIR_LEN = 1;
    public static final int DIR_STATUS = 2;
    public static final int STATUS_OK = 0;
    public static final int STATUS_VANILLA = 1;
    public static final int STATUS_UNKNOWN_ASSET = 2;
    public static final int STATUS_BAD_ROW = 3;
    public static final int STATUS_DISABLED = 4;
    public static final int STATUS_NO_ROOM = 5;
    public static final int TAG_HEAD = 8;
    public static final int HEAD_PLATE_DX = 0;
    public static final int HEAD_PLATE_DY = 1;
    public static final int HEAD_OWNS_BACKGROUND = 2;
    public static final int HEAD_TEXT_MODE = 3;
    public static final int HEAD_OPS = 4;
    public static final int HEAD_MATERIALS = 5;
    public static final int HEAD_MATERIAL_WORDS = 6;
    public static final int HEAD_EVENTS = 7;
    public static final int TEXT_MODE_VANILLA = 0;
    public static final int TEXT_MODE_MOTION = 1;
    public static final int TEXT_MODE_OWNED = 2;
    public static final int OP_WORDS = 8;
    public static final int OP_X0 = 0;
    public static final int OP_Y0 = 1;
    public static final int OP_X1 = 2;
    public static final int OP_Y1 = 3;
    public static final int OP_ARGB = 4;
    public static final int OP_BAND_LAYER_KIND = 5;
    public static final int OP_AUX0 = 6;
    public static final int OP_AUX1 = 7;
    public static final int OP_KIND_RECT = 0;
    public static final int OP_KIND_STRIP = 1;
    public static final int OP_KIND_IMAGE = 2;
    public static final int AUX_LOW = 0xffff;
    public static final int AUX_AXIS_Y = 1 << 16;
    public static final int AUX_MIRROR = 1 << 17;
    public static final int AUX_ADDITIVE = 1 << 18;
    public static final int BAND_BEHIND = 0;
    public static final int BAND_BACKGROUND = 1;
    public static final int BAND_OVER_BACKGROUND = 2;
    public static final int BAND_OVER_TEXT = 3;
    public static final int BAND_BADGE = 4;
    public static final int MAT_SHADER = 0;
    public static final int MAT_BAND_LAYER = 1;
    public static final int MAT_RUN = 2;
    public static final int MAT_AREA_X0 = 3;
    public static final int MAT_AREA_Y0 = 4;
    public static final int MAT_AREA_X1 = 5;
    public static final int MAT_AREA_Y1 = 6;
    public static final int MAT_CLIP_COUNT = 7;
    public static final int MAT_UNIFORM_WORDS = 8;
    public static final int MAT_FIXED = 9;
    public static final int CLIP_WORDS = 4;
    public static final int MAT_MIRRORED = 1 << 16;
    public static final int MAT_OPACITY_SHIFT = 24;
    public static final int TEXT_COLOR = 0;
    public static final int TEXT_SHADOW = 1;
    public static final int TEXT_SHADOW_USE = 2;
    public static final int TEXT_OUTLINE = 3;
    public static final int TEXT_LETTER_COUNT = 4;
    public static final int TEXT_FIXED = 5;
    public static final int SHADOW_LEAVE = 0;
    public static final int SHADOW_REMOVE = 1;
    public static final int SHADOW_ALWAYS = 2;
    public static final int SHADOW_WITH_DROP = 3;
    public static final int LETTER_WORDS = 7;
    public static final int LETTER_DX = 0;
    public static final int LETTER_DY = 1;
    public static final int LETTER_ARGB = 2;
    public static final int LETTER_SHADOW = 3;
    public static final int LETTER_OPACITY = 4;
    public static final int LETTER_SCALE_ROT = 5;
    public static final int LETTER_OUTLINE = 6;
    public static final int BADGE_WORDS = 5;
    public static final int BADGE_X = 0;
    public static final int BADGE_Y = 1;
    public static final int BADGE_OPACITY = 2;
    public static final int BADGE_TINT = 3;
    public static final int BADGE_DECORATION = 4;
    public static final int DECORATION_NONE = 0;
    public static final int DECORATION_SHADOW = 1;
    public static final int DECORATION_OUTLINE = 2;
    public static final int DECORATION_KIND_SHIFT = 24;
    public static final int EVENT_WORDS = 2;
    public static final int EVENT_CODE = 0;
    public static final int EVENT_LAYER = 1;
    public static final int EVENT_LAYER_DISABLED = 1;
    public static final int EVENT_MOLANG_OVER_BUDGET = 2;
    public static final int EVENT_ASSET_DISABLED = 3;
    public static final int EVENT_TARGET_TAG = -1;
    public static final int EVENT_TARGET_PALETTE = 32;
    public static final int EVENT_TARGET_LETTERS = 33;
    public static final int EVENT_TARGET_BADGE = 34;
    public static final int EVENT_TARGET_PLATE = 35;
    public static final int MAX_LAYERS = 32;
    public static final int MAX_LETTERS = 24;
    public static final int MAX_LINES = 3;
    public static final int PALETTE_TONES = 512;
    public static final int GLYPH_HEIGHT = 8;
    public static final int LINE_PITCH = 10;
    public static final int BADGE_WIDTH = 9;
    public static final int BADGE_HEIGHT = 8;
    public static final int BADGE_ADVANCE = 11;
    public static final int UNIFORM_LANES = 16;
    public static final int UNIFORM_ATLAS = 28;
    public static final int UNIFORM_TONES = 44;
    public static final int RASTER_INFO_WIDTH = 0;
    public static final int RASTER_INFO_HEIGHT = 1;
    public static final int RASTER_INFO_X = 2;
    public static final int RASTER_INFO_Y = 3;
    public static final int RASTER_INFO_PLATE_DX = 4;
    public static final int RASTER_INFO_PLATE_DY = 5;
    public static final int RASTER_INFO_BADGE_SHOWN = 6;
    public static final int RASTER_INFO_BADGE_X = 7;
    public static final int RASTER_INFO_BADGE_Y = 8;
    public static final int RASTER_INFO_BADGE_OPACITY = 9;
    public static final int RASTER_INFO_TEXT_MODE = 10;
    public static final int RASTER_INFO_LETTER_COUNT = 11;
    public static final int RASTER_INFO_LETTERS = 12;
    public static final int RASTER_INFO_WORDS = RASTER_INFO_LETTERS + MAX_LETTERS * LETTER_WORDS;
    public static final int MOTION_FULL = 0, MOTION_REDUCED = 1, MOTION_OFF = 2;
    public static final int DETAIL_FULL = 0, DETAIL_REDUCED = 1, DETAIL_MINIMAL = 2;
    public static final int ALIGN_LEFT = 0, ALIGN_RIGHT = 1, ALIGN_CENTER = 2;

    static {
        try {
            Class.forName(Renderer.class.getName(), true, Nameplates.class.getClassLoader());
        } catch (ClassNotFoundException e) {
            throw new NoClassDefFoundError(e.getMessage());
        }
    }

    private long handle = nCreate();

    public synchronized int addAsset(NameplateAsset asset) {
        long h = handle();
        asset.lock.readLock().lock();
        try {
            return check(nAddAsset(h, asset.handle()));
        } finally {
            asset.lock.readLock().unlock();
        }
    }

    public synchronized void removeAsset(int id) {
        nRemoveAsset(handle(), id);
    }

    public synchronized void setMotion(int motion) {
        check(nSetMotion(handle(), motion));
    }

    public synchronized void setVanillaTexture(String id, int width, int height, byte[] rgba) {
        if (id == null) throw new NullPointerException("id");
        check(nSetVanillaTexture(handle(), id, width, height, rgba));
    }

    public synchronized int neededWords(int[] tags, int count) {
        return check(nNeededWords(handle(), tags, count));
    }

    public synchronized int frame(double clock, long[] wearers, int count, int[] tags, float[] signals, float[] glyphs, int[] colors,
                                  int[] out) {
        return check(nFrame(handle(), clock, wearers, count, tags, signals, glyphs, colors, out));
    }

    public synchronized boolean forgetWearer(long wearer) {
        return nForgetWearer(handle(), wearer);
    }

    public synchronized void setSignalOverride(long wearer, int signal, float value) {
        check(nSetSignalOverride(handle(), wearer, signal, value));
    }

    public synchronized void setCustomColors(long wearer, int asset, int[] slots, int[] rgb) {
        if (slots == null) throw new NullPointerException("slots");
        if (rgb == null) throw new NullPointerException("rgb");
        check(nSetCustomColors(handle(), wearer, asset, slots, rgb));
    }

    public synchronized boolean clearCustomColors(long wearer, int asset) {
        return nClearCustomColors(handle(), wearer, asset);
    }

    public synchronized boolean fire(long wearer, int asset, int oneShot) {
        return check(nFire(handle(), wearer, asset, oneShot)) != 0;
    }

    public synchronized int stripLayout() {
        return nStripLayout(handle());
    }

    public synchronized int stripTexels(int id, int[] out) {
        return check(nStripTexels(handle(), id, out));
    }

    public synchronized int wearerCount() {
        return nWearerCount(handle());
    }

    public synchronized int wearerBytes() {
        return nWearerBytes(handle());
    }

    public synchronized int pendingFallbacks() {
        return nPendingFallbacks(handle());
    }

    public synchronized int runFallbacks(int limit) {
        return check(nRunFallbacks(handle(), limit));
    }

    public synchronized Fallback takeFallback() {
        long job = nTakeFallback(handle());
        return job == 0 ? null : new Fallback(job);
    }

    public synchronized boolean finishFallback(Fallback fallback) {
        long n = handle();
        long job = fallback.release();
        return job != 0 && check(nFinishFallback(n, job)) == 1;
    }

    synchronized void faultNextFrame() {
        nFault(handle());
    }

    @Override
    public synchronized void close() {
        if (handle != 0) nFree(handle);
        handle = 0;
    }

    private long handle() {
        if (handle == 0) throw new IllegalStateException("nameplates are closed");
        return handle;
    }

    private int check(int code) {
        if (code == -2) throw new IllegalStateException(nError(handle));
        if (code < 0) throw new IllegalArgumentException(nError(handle));
        return code;
    }

    private static native long nCreate();
    private static native void nFree(long n);
    private static native String nError(long n);
    private static native int nAddAsset(long n, long asset);
    private static native void nRemoveAsset(long n, int id);
    private static native int nSetMotion(long n, int motion);
    private static native int nSetVanillaTexture(long n, String id, int width, int height, byte[] rgba);
    private static native int nNeededWords(long n, int[] tags, int count);
    private static native int nFrame(long n, double clock, long[] wearers, int count, int[] tags, float[] signals,
                                     float[] glyphs, int[] colors, int[] out);
    private static native void nFault(long n);
    private static native boolean nForgetWearer(long n, long wearer);
    private static native int nSetSignalOverride(long n, long wearer, int signal, float value);
    private static native int nSetCustomColors(long n, long wearer, int asset, int[] slots, int[] rgb);
    private static native boolean nClearCustomColors(long n, long wearer, int asset);
    private static native int nFire(long n, long wearer, int asset, int oneShot);
    private static native int nStripLayout(long n);
    private static native int nStripTexels(long n, int id, int[] out);
    private static native int nWearerCount(long n);
    private static native int nWearerBytes(long n);
    private static native int nPendingFallbacks(long n);
    private static native int nRunFallbacks(long n, int limit);
    private static native long nTakeFallback(long n);
    private static native boolean nRunFallback(long job);
    private static native int nFinishFallback(long n, long job);
    private static native void nFreeFallback(long job);

    public static final class Fallback implements AutoCloseable {
        private long handle;

        private Fallback(long handle) {
            this.handle = handle;
        }

        public synchronized void run() {
            if (handle == 0) throw new IllegalStateException("fallback already finished or closed");
            nRunFallback(handle);
        }

        private synchronized long release() {
            long job = handle;
            handle = 0;
            return job;
        }

        @Override
        public void close() {
            long job = release();
            if (job != 0) nFreeFallback(job);
        }
    }
}
