package org.polyfrost.plusrenderer;

import static org.junit.Assert.*;
import static org.polyfrost.plusrenderer.RendererTest.closed;
import static org.polyfrost.plusrenderer.RendererTest.fails;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicReference;

import org.junit.Test;

public class MolangTest {
    @Test
    public void compiledExpressionsEvaluateWithQueriesAndVariablesById() {
        try (MolangEnv env = new MolangEnv(true);
             MolangRuntime pet = new MolangRuntime(env, 1)) {
            int doubled = env.compile("q.speed * 2");
            int sum = env.compile("query.speed + q.height");
            int hang = env.compile("math.die_roll(1000000000, 0, 1)");
            int spin = env.compile("v.spin = v.spin + 1; return v.spin;");
            int speed = env.queryId("speed");
            assertEquals(speed, env.queryId("Query.Speed"));
            assertEquals(-1, env.queryId("q.missing"));
            int spinVar = env.variableId("variable.spin");
            assertEquals(1, env.variableCount());

            float[] queries = new float[env.queryCount()];
            queries[speed] = 1.5f;
            queries[env.queryId("height")] = 4;
            float[] out = {-1, -1, -1, -1, -1};
            boolean[] failed = new boolean[4];
            int[] ids = {doubled, sum, hang, spin, doubled};
            assertEquals(1, pet.evaluate(ids, 4, queries, 0, out, failed));
            assertArrayEquals(new float[] {3, 5.5f, 0, 1, -1}, out, 1e-6f);
            assertArrayEquals(new boolean[] {false, false, true, false}, failed);
            assertEquals(1, pet.evaluate(new int[] {hang}, 1, queries, 0, out, null));

            pet.setVariable(spinVar, 10);
            assertEquals(10, pet.variable(spinVar), 0);
            pet.reset();
            assertEquals(0, pet.variable(spinVar), 0);
            assertEquals(0, pet.initialize(new int[] {spin}));
            assertEquals(1, pet.variable(spinVar), 0);
            pet.evaluate(new int[] {spin}, 1, null, 0, out, null);
            assertEquals("variables persist across calls", 2, out[0], 0);

            int late = env.compile("v.late * 2");
            pet.setVariable(env.variableId("late"), 5);
            pet.evaluate(new int[] {late}, 1, null, 0, out, null);
            assertEquals("a variable compiled after the runtime can be set", 10, out[0], 0);
            fails("", () -> pet.evaluate(ids, 6, queries, 0, new float[6], new boolean[6]));
            fails("", () -> pet.evaluate(ids, -1, queries, 0, out, failed));
            fails("", () -> pet.evaluate(ids, 4, queries, 0, new float[3], failed));
            fails("", () -> pet.evaluate(ids, 4, queries, 0, out, new boolean[3]));
            fails("nope", () -> env.compile("math.nope(1)"));
            try (MolangEnv lenient = new MolangEnv(false)) {
                lenient.compile("math.nope(1) + 2");
                assertEquals(1, lenient.warnings().length);
            }
        }
    }

    @Test
    public void runtimesOutliveAClosedEnv() {
        MolangEnv env = new MolangEnv(false);
        int id = env.compile("v.n = v.n + q.step; return v.n;");
        int step = env.queryId("step");
        MolangRuntime pet = new MolangRuntime(env, 1);
        env.close();
        env.close();
        float[] queries = new float[step + 1];
        queries[step] = 2;
        float[] out = new float[1];
        pet.evaluate(new int[] {id}, 1, queries, 0, out, null);
        pet.evaluate(new int[] {id}, 1, queries, 0, out, null);
        assertEquals(4, out[0], 0);
        closed(() -> env.compile("1"));
        closed(() -> new MolangRuntime(env, 1));
        pet.close();
        pet.close();
        closed(() -> pet.evaluate(new int[] {id}, 1, queries, 0, out, null));
    }

    private static Thread[] run(int threads, AtomicReference<Throwable> thrown, Runnable body) {
        Thread[] workers = new Thread[threads];
        for (int i = 0; i < threads; i++) {
            workers[i] = new Thread(() -> {
                try {
                    body.run();
                } catch (Throwable t) {
                    thrown.compareAndSet(null, t);
                }
            });
            workers[i].start();
        }
        return workers;
    }

    @Test
    public void threadsSharingARuntimeTakeTurnsAndCloseOnlyEndsTheirCalls() throws InterruptedException {
        try (MolangEnv env = new MolangEnv(false)) {
            int[] bump = {env.compile("v.n = v.n + 1; return v.n;")};
            int n = env.variableId("n");
            AtomicReference<Throwable> thrown = new AtomicReference<>();
            try (MolangRuntime pet = new MolangRuntime(env, 1)) {
                Thread[] workers = run(4, thrown, () -> {
                    for (int r = 0; r < 5000; r++) pet.evaluate(bump, 1, null, r, new float[1], null);
                });
                for (Thread worker : workers) worker.join();
                assertEquals(4 * 5000, pet.variable(n), 0);
            }
            for (int round = 0; round < 20; round++) {
                MolangRuntime pet = new MolangRuntime(env, round);
                CountDownLatch running = new CountDownLatch(4);
                Thread[] workers = run(4, thrown, () -> {
                    try {
                        for (int r = 0; ; r++) {
                            pet.evaluate(bump, 1, null, r, new float[1], null);
                            pet.setVariable(n, pet.variable(n));
                            if (r == 10) running.countDown();
                        }
                    } catch (IllegalStateException closed) {
                        running.countDown();
                    }
                });
                running.await();
                pet.close();
                for (Thread worker : workers) worker.join();
            }
            assertNull(thrown.get());
        }
    }
}
