package org.polyfrost.plusrenderer;

import java.util.Objects;
import java.util.concurrent.locks.ReentrantReadWriteLock;

public final class CosmeticAsset implements AutoCloseable {
    public static final int MODE_CLIENT = 0, MODE_AUTHORING = 1;
    public static final int STATUS_ASSET = 0, STATUS_DRAFT = 1, STATUS_SKIPPED = 2, STATUS_INVALID = 3,
            STATUS_LEGACY = 4;
    public static final int NO_COLORWAY = -1;
    public static final int TEXTURE_INFO_INTS = 3;
    public static final int TEXTURE_WIDTH = 0, TEXTURE_HEIGHT = 1, TEXTURE_INDEXED = 2;

    static {
        try {
            Class.forName(Renderer.class.getName(), true, CosmeticAsset.class.getClassLoader());
        } catch (ClassNotFoundException e) {
            throw new NoClassDefFoundError(e.getMessage());
        }
    }

    final ReentrantReadWriteLock lock = new ReentrantReadWriteLock();
    private final int status;
    private long handle;

    private CosmeticAsset(long handle) {
        this.handle = handle;
        this.status = nStatus(handle);
    }

    public static CosmeticAsset parse(byte[] bytes, int mode, long identity) {
        return parse(bytes, mode, identity, null);
    }

    public static CosmeticAsset parse(byte[] bytes, int mode, long identity, String[] capabilities) {
        Objects.requireNonNull(bytes, "bytes");
        String joined = null;
        if (capabilities != null) {
            StringBuilder names = new StringBuilder();
            for (String name : capabilities) names.append(names.length() == 0 ? "" : "\n").append(name);
            joined = names.toString();
        }
        return new CosmeticAsset(nParse(bytes, mode, identity, joined));
    }

    public int status() {
        return status;
    }

    public boolean loaded() {
        return status == STATUS_ASSET || status == STATUS_DRAFT;
    }

    public String issuesJson() {
        lock.readLock().lock();
        try {
            return nIssuesJson(handle());
        } finally {
            lock.readLock().unlock();
        }
    }

    public String analyzeJson() {
        lock.readLock().lock();
        try {
            return nAnalyzeJson(handle());
        } finally {
            lock.readLock().unlock();
        }
    }

    public String metaJson() {
        lock.readLock().lock();
        try {
            return nMetaJson(handle());
        } finally {
            lock.readLock().unlock();
        }
    }

    public int textureCount() {
        lock.readLock().lock();
        try {
            return nTextureCount(handle());
        } finally {
            lock.readLock().unlock();
        }
    }

    public int textureInfo(int index, int[] info) {
        lock.readLock().lock();
        try {
            return nTextureInfo(handle(), index, info);
        } finally {
            lock.readLock().unlock();
        }
    }

    public int resolveTexture(int index, int colorway, int[] customSlots, int[] customRgb, int[] size, byte[] rgba) {
        checkCustom(customSlots, customRgb);
        lock.readLock().lock();
        try {
            return nResolveTexture(handle(), index, colorway, customSlots, customRgb, size, rgba);
        } finally {
            lock.readLock().unlock();
        }
    }

    public long textureKey(int index, int colorway, int[] customSlots, int[] customRgb) {
        checkCustom(customSlots, customRgb);
        lock.readLock().lock();
        try {
            return nTextureKey(handle(), index, colorway, customSlots, customRgb);
        } finally {
            lock.readLock().unlock();
        }
    }

    public int effectTextureCount() {
        lock.readLock().lock();
        try {
            return nEffectTextureCount(handle());
        } finally {
            lock.readLock().unlock();
        }
    }

    public int effectTextureInfo(int index, int[] info) {
        lock.readLock().lock();
        try {
            return nEffectTextureInfo(handle(), index, info);
        } finally {
            lock.readLock().unlock();
        }
    }

    public int effectTexture(int index, byte[] rgba) {
        lock.readLock().lock();
        try {
            return nEffectTexture(handle(), index, rgba);
        } finally {
            lock.readLock().unlock();
        }
    }

    public int soundCount() {
        lock.readLock().lock();
        try {
            return nSoundCount(handle());
        } finally {
            lock.readLock().unlock();
        }
    }

    public byte[] sound(int index) {
        lock.readLock().lock();
        try {
            return nSound(handle(), index);
        } finally {
            lock.readLock().unlock();
        }
    }

    @Override
    public void close() {
        lock.writeLock().lock();
        try {
            if (handle != 0) nFree(handle);
            handle = 0;
        } finally {
            lock.writeLock().unlock();
        }
    }

    long handle() {
        if (handle == 0) throw new IllegalStateException("cosmetic asset is closed");
        return handle;
    }

    private static void checkCustom(int[] slots, int[] rgb) {
        int a = slots == null ? 0 : slots.length;
        int b = rgb == null ? 0 : rgb.length;
        if (a != b) throw new IllegalArgumentException(a + " custom slots for " + b + " colors");
    }

    private static native long nParse(byte[] bytes, int mode, long identity, String capabilities);
    private static native void nFree(long h);
    private static native int nStatus(long h);
    private static native String nIssuesJson(long h);
    private static native String nAnalyzeJson(long h);
    private static native String nMetaJson(long h);
    private static native int nTextureCount(long h);
    private static native int nTextureInfo(long h, int index, int[] info);
    private static native int nResolveTexture(long h, int index, int colorway, int[] slots, int[] rgb, int[] size,
                                              byte[] rgba);
    private static native long nTextureKey(long h, int index, int colorway, int[] slots, int[] rgb);
    private static native int nEffectTextureCount(long h);
    private static native int nEffectTextureInfo(long h, int index, int[] info);
    private static native int nEffectTexture(long h, int index, byte[] rgba);
    private static native int nSoundCount(long h);
    private static native byte[] nSound(long h, int index);
}
