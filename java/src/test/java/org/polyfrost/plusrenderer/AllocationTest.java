package org.polyfrost.plusrenderer;

import static org.junit.Assert.*;

import java.io.IOException;
import java.lang.management.ManagementFactory;
import java.util.Arrays;

import org.junit.Test;

public class AllocationTest {
    private static final int WARMUP = 4000;
    private static final int CALLS = 5000;
    private static final long SLACK = 2048;
    private static final int IS_SNEAKING = 0;

    private static final com.sun.management.ThreadMXBean THREADS =
            (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();

    private static long allocated() {
        return THREADS.getThreadAllocatedBytes(Thread.currentThread().getId());
    }

    private static void assertNoAllocation(String what, Runnable call) {
        for (int i = 0; i < WARMUP; i++) call.run();
        allocated();
        long before = allocated();
        for (int i = 0; i < CALLS; i++) call.run();
        long bytes = allocated() - before;
        assertTrue(what + " allocated " + bytes + " bytes over " + CALLS + " calls", bytes <= SLACK);
    }

    @Test
    public void theWearerFrameAndItsReadbacksAllocateNothing() {
        try (Renderer r = new Renderer()) {
            r.loadCosmetic(0, RendererTest.GLASSES, "glasses", "", 16, 16);
            RendererTest.loadTipping(r, "controller.animation.glasses.tip");
            r.play(0, 0);

            int stride = Renderer.wearerInputStride(0, 0);
            float[] input = new float[stride * 2];
            input[Renderer.wearerSignalsOffset(0, 0) + IS_SNEAKING] = 1;
            float[] out = new float[r.wearerOutputStride() * 2];
            long[] wearers = {21L, 22L};
            int[] events = new int[Renderer.EFFECT_EVENT_INTS * 4];
            float[] seconds = new float[1];
            float[] delta = new float[Renderer.PART_DELTA_VALUES];
            double[] clock = {1.0};

            assertNoAllocation("frameWearers", () -> {
                clock[0] += 0.001;
                r.frameWearers(clock[0], wearers, 2, input, 0, 0, Renderer.JAVA_MODEL_SPACE, out);
            });
            assertNoAllocation("effectEvents", () -> r.effectEvents(events));
            assertNoAllocation("controllerState", () -> assertNotNull(r.controllerState(0, 21L, 0, seconds)));
            assertNoAllocation("partDelta", () -> assertTrue(r.partDelta(0, "bb_main", delta)));
            assertNoAllocation("setQuery", () -> r.setQuery("q.test", 1f));
            assertNoAllocation("setGamePart", () -> r.setGamePart("head", 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1));
            assertNoAllocation("setLightLevel", () -> r.setLightLevel(15f));
            assertNoAllocation("pose", () -> r.pose(1.0));
        }
    }

    @Test
    public void controllerEventsAndReadbacksMatchTheAllocatingOnes() {
        try (Renderer r = new Renderer()) {
            r.loadCosmetic(0, RendererTest.GLASSES, "glasses", "", 16, 16);
            RendererTest.loadTipping(r, "controller.animation.glasses.tip");
            r.play(0, 0);
            float[] input = new float[Renderer.wearerInputStride(0, 0)];
            input[Renderer.wearerSignalsOffset(0, 0) + IS_SNEAKING] = 1;
            float[] out = new float[r.wearerOutputStride()];
            r.frameWearers(1.0, new long[] {21L}, 1, input, 0, 0, Renderer.JAVA_MODEL_SPACE, out);
            r.frameWearers(1.5, new long[] {21L}, 1, input, 0, 0, Renderer.JAVA_MODEL_SPACE, out);
            float[] seconds = new float[1];
            Renderer.ControllerState state = r.controllerState(0, 21L, 0);
            assertEquals("tipped", state.state);
            assertEquals(0.5f, state.seconds, 1e-6);
            assertEquals(state.state, r.controllerState(0, 21L, 0, seconds));
            assertEquals(state.seconds, seconds[0], 0f);
            assertNull(r.controllerState(0, 99L, 0, seconds));
            int[] events = new int[Renderer.EFFECT_EVENT_INTS * 4];
            assertEquals(0, r.effectEvents(events));
            r.frameWearers(2.0, new long[] {22L}, 1, input, 0, 0, Renderer.JAVA_MODEL_SPACE, out);
            assertEquals(2, r.effectEvents(events));
            assertArrayEquals(new int[] {0, 0, 0, 1, Renderer.EFFECT_PARTICLE, 0, 0, 0, 0, 1, Renderer.EFFECT_SOUND, 1},
                    Arrays.copyOf(events, Renderer.EFFECT_EVENT_INTS * 2));
            int[] one = new int[Renderer.EFFECT_EVENT_INTS + 1];
            Arrays.fill(one, -1);
            assertEquals(2, r.effectEvents(one));
            assertEquals("only whole events are copied", -1, one[Renderer.EFFECT_EVENT_INTS]);
            float[] delta = new float[Renderer.PART_DELTA_VALUES + 1];
            Arrays.fill(delta, -7f);
            assertTrue(r.partDelta(0, "bb_main", delta));
            assertArrayEquals(r.partDelta(0, "bb_main"), Arrays.copyOf(delta, Renderer.PART_DELTA_VALUES), 0f);
            assertEquals(-7f, delta[Renderer.PART_DELTA_VALUES], 0f);
            RendererTest.fails("", () -> r.partDelta(0, "bb_main", new float[3]));
        }
    }

    @Test
    public void theShapedFrameAndEffectReadbacksAllocateNothing() throws IOException {
        try (CosmeticAsset asset = CosmeticAssetTest.parse(); Renderer r = new Renderer()) {
            r.loadAsset(0, asset);
            r.play(0, 0.0);
            int[] sections = CosmeticAssetTest.sections(Renderer.SECTION_COLORWAY, 1);
            int[] in = CosmeticAssetTest.inputLayout(sections);
            float[] out = new float[r.outputLayout(sections, new int[Renderer.LAYOUT_INTS]) * 2];
            long[] wearers = {1L, 2L};
            float[] input = CosmeticAssetTest.rows(in, false, true);
            float[] view = new float[Renderer.VIEW_FLOATS];
            view[3] = 1;
            view[7] = 1;
            float[] quads = new float[Renderer.QUAD_FLOATS * 8];
            float[] eventFloats = new float[Renderer.EFFECT_EVENT_FLOATS * 4];
            double[] clock = {1.0};

            assertNoAllocation("frameWearersShaped", () -> {
                clock[0] += 0.001;
                r.frameWearersShaped(clock[0], wearers, 2, sections, input, Renderer.CORE_SPACE, out);
            });
            assertNoAllocation("effectGeometry", () -> r.effectGeometry(view, quads));
            assertNoAllocation("effectEventFloats", () -> r.effectEventFloats(eventFloats));
        }
    }

    @Test
    public void theNameplateFrameAllocatesNothing() throws IOException {
        try (NameplateAsset asset = NameplateAsset.parse(NameplatesTest.kit("test_plate"), NameplateAsset.MODE_CLIENT, 0);
             Nameplates plates = new Nameplates()) {
            int id = plates.addAsset(asset);
            int[] tags = NameplatesTest.rows(id, 40, id, 60, id, 24);
            int[] out = new int[plates.neededWords(tags, 3)];
            long[] wearers = {1L, 2L, 3L};
            float[] signals = new float[3 * Nameplates.SIGNAL_FLOATS];
            double[] clock = {10.0};

            assertNoAllocation("nameplate frame", () -> {
                clock[0] += 0.001;
                plates.frame(clock[0], wearers, 3, tags, signals, null, null, out);
            });
        }
    }
}
