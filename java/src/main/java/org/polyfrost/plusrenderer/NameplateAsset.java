package org.polyfrost.plusrenderer;

import java.util.Objects;
import java.util.concurrent.locks.ReentrantReadWriteLock;

public final class NameplateAsset implements AutoCloseable {
    public static final int MODE_CLIENT = 0, MODE_AUTHORING = 1;
    public static final int STATUS_ASSET = 0, STATUS_DRAFT = 1, STATUS_SKIPPED = 2, STATUS_INVALID = 3;
    public static final int DIALECT_CORE_330 = 0, DIALECT_CORE_330_UBO = 1, DIALECT_LEGACY_150 = 2,
            DIALECT_COMPAT_130 = 3, DIALECT_ES_300 = 4, DIALECT_SKSL = 5;
    public static final int DIALECT_COUNT = 6;
    public static final int MOLANG_CHANNEL = 0, MOLANG_SLOT_COLOR = 1, MOLANG_FIELD = 2, MOLANG_TEXT = 3,
            MOLANG_MATERIAL = 4;

    static {
        try {
            Class.forName(Renderer.class.getName(), true, NameplateAsset.class.getClassLoader());
        } catch (ClassNotFoundException e) {
            throw new NoClassDefFoundError(e.getMessage());
        }
    }

    final ReentrantReadWriteLock lock = new ReentrantReadWriteLock();
    private final int status;
    private long handle;

    private NameplateAsset(long handle) {
        this.handle = handle;
        this.status = nStatus(handle);
    }

    public static NameplateAsset parse(byte[] bytes, int mode, long identity) {
        return parse(bytes, mode, identity, null);
    }

    public static NameplateAsset parse(byte[] bytes, int mode, long identity, String[] capabilities) {
        Objects.requireNonNull(bytes, "bytes");
        String joined = null;
        if (capabilities != null) {
            StringBuilder names = new StringBuilder();
            for (String name : capabilities) names.append(names.length() == 0 ? "" : "\n").append(name);
            joined = names.toString();
        }
        return new NameplateAsset(nParse(bytes, mode, identity, joined));
    }

    public static String constantsJson() {
        return nConstantsJson();
    }

    public static String checkMolangJson(String source, int context) {
        Objects.requireNonNull(source, "source");
        return nCheckMolangJson(source, context);
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

    public String shadersJson(int dialect) {
        lock.readLock().lock();
        try {
            return nShadersJson(handle(), dialect);
        } finally {
            lock.readLock().unlock();
        }
    }

    public String dialectIssuesJson(int dialect) {
        lock.readLock().lock();
        try {
            return nDialectIssuesJson(handle(), dialect);
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

    public int texture(int index, int[] size, byte[] rgba) {
        lock.readLock().lock();
        try {
            return nTexture(handle(), index, size, rgba);
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
        if (handle == 0) throw new IllegalStateException("nameplate asset is closed");
        return handle;
    }

    private static native long nParse(byte[] bytes, int mode, long identity, String capabilities);
    private static native void nFree(long h);
    private static native int nStatus(long h);
    private static native String nIssuesJson(long h);
    private static native String nShadersJson(long h, int dialect);
    private static native String nDialectIssuesJson(long h, int dialect);
    private static native String nAnalyzeJson(long h);
    private static native String nMetaJson(long h);
    private static native int nTextureCount(long h);
    private static native int nTexture(long h, int index, int[] size, byte[] rgba);
    private static native String nConstantsJson();
    private static native String nCheckMolangJson(String source, int context);
}
