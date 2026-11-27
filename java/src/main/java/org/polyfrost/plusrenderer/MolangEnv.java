package org.polyfrost.plusrenderer;

import java.util.Objects;

public final class MolangEnv implements AutoCloseable {
    private static final String[] NO_WARNINGS = new String[0];

    static {
        try {
            Class.forName(Renderer.class.getName(), true, MolangEnv.class.getClassLoader());
        } catch (ClassNotFoundException e) {
            throw new NoClassDefFoundError(e.getMessage());
        }
    }

    private long handle;

    public MolangEnv(boolean strict) {
        handle = nCreate(strict);
    }

    public synchronized int compile(String source) {
        Objects.requireNonNull(source, "source");
        return nCompile(handle(), source);
    }

    public synchronized int queryId(String name) {
        Objects.requireNonNull(name, "name");
        return nQueryId(handle(), name);
    }

    public synchronized int variableId(String name) {
        Objects.requireNonNull(name, "name");
        return nVariableId(handle(), name);
    }

    public synchronized int queryCount() {
        return nQueryCount(handle());
    }

    public synchronized int variableCount() {
        return nVariableCount(handle());
    }

    public synchronized String[] warnings() {
        String joined = nWarnings(handle());
        return joined.isEmpty() ? NO_WARNINGS : joined.split("\n");
    }

    @Override
    public synchronized void close() {
        if (handle != 0) nFree(handle);
        handle = 0;
    }

    long handle() {
        if (handle == 0) throw new IllegalStateException("MoLang env is closed");
        return handle;
    }

    private static native long nCreate(boolean strict);
    private static native void nFree(long env);
    private static native int nCompile(long env, String source);
    private static native int nQueryId(long env, String name);
    private static native int nVariableId(long env, String name);
    private static native int nQueryCount(long env);
    private static native int nVariableCount(long env);
    private static native String nWarnings(long env);
}
