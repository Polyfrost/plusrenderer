package org.polyfrost.plusrenderer;

public final class MolangRuntime implements AutoCloseable {
    private long handle;

    public MolangRuntime(MolangEnv env, long seed) {
        synchronized (env) {
            handle = nCreate(env.handle(), seed);
        }
    }

    public synchronized int initialize(int[] scriptIds) {
        long h = handle();
        return nInitialize(h, scriptIds, scriptIds.length);
    }

    public synchronized int evaluate(int[] exprIds, int exprCount, float[] queries, double clock, float[] out, boolean[] failed) {
        long h = handle();
        if (exprCount < 0 || exprCount > exprIds.length) {
            throw new IllegalArgumentException("exprCount " + exprCount + " is outside exprIds, which holds " + exprIds.length);
        }
        if (out.length < exprCount) {
            throw new IllegalArgumentException("out holds " + out.length + " floats, " + exprCount + " are needed");
        }
        if (failed != null && failed.length < exprCount) {
            throw new IllegalArgumentException("failed holds " + failed.length + " flags, " + exprCount + " are needed");
        }
        return nEvaluate(h, exprIds, exprCount, queries, clock, out, failed);
    }

    public synchronized float variable(int id) {
        return nVariable(handle(), id);
    }

    public synchronized void setVariable(int id, float value) {
        nSetVariable(handle(), id, value);
    }

    public synchronized void reset() {
        nReset(handle());
    }

    @Override
    public synchronized void close() {
        if (handle != 0) nFree(handle);
        handle = 0;
    }

    private long handle() {
        if (handle == 0) throw new IllegalStateException("MoLang runtime is closed");
        return handle;
    }

    private static native long nCreate(long env, long seed);
    private static native void nFree(long runtime);
    private static native int nInitialize(long runtime, int[] scriptIds, int count);
    private static native int nEvaluate(long runtime, int[] exprIds, int exprCount, float[] queries, double clock, float[] out, boolean[] failed);
    private static native float nVariable(long runtime, int id);
    private static native void nSetVariable(long runtime, int id, float value);
    private static native void nReset(long runtime);
}
