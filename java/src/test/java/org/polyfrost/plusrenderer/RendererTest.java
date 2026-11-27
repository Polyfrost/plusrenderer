package org.polyfrost.plusrenderer;

import static org.junit.Assert.*;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;

import org.junit.Test;
import org.junit.function.ThrowingRunnable;

// Needs the host native: scripts/build-native.sh target/java-resources
public class RendererTest {
    static final String GLASSES = "{\"format_version\":\"1.12.0\",\"minecraft:geometry\":[{"
            + "\"description\":{\"identifier\":\"geometry.glasses\",\"texture_width\":16,\"texture_height\":16},"
            + "\"bones\":[{\"name\":\"bb_main\",\"pivot\":[0,0,0],"
            + "\"cubes\":[{\"origin\":[-4,27,-5],\"size\":[8,2,1],\"uv\":[0,0]}]}]}]}";
    static final String TIPPING = "{\"animations\":{"
            + "\"animation.glasses.rest\":{\"loop\":true,\"bones\":{\"bb_main\":{\"rotation\":[0,0,0]}}},"
            + "\"animation.glasses.tip\":{\"loop\":true,\"bones\":{\"bb_main\":{\"rotation\":[20,0,0]}}}}}";
    static final String TIP = "{\"animation_controllers\":{"
            + "\"controller.animation.glasses.tip\":{\"states\":{"
            + "\"default\":{\"animations\":[\"rest\"],\"transitions\":[{\"tipped\":\"q.is_sneaking\"}]},"
            + "\"tipped\":{\"animations\":[\"tip\"],"
            + "\"particle_effects\":[{\"effect\":\"puff\"}],\"sound_effects\":[{\"effect\":\"ding\"}],"
            + "\"transitions\":[{\"default\":\"!q.is_sneaking\"}]}}}}}";
    private static final String TURN = "{\"animations\":{\"turn\":{\"animation_length\":10,\"bones\":{\"head\":{"
            + "\"rotation\":{\"0\":[0,0,0],\"10\":[100,0,0]}}}}}}";
    private static final String SNEAK = "{\"animations\":{\"a\":{\"loop\":true,"
            + "\"bones\":{\"bb_main\":{\"rotation\":[\"q.is_sneaking * 20\",0,0]}}}}}";

    static void fails(String message, ThrowingRunnable call) {
        String got = assertThrows(IllegalArgumentException.class, call).getMessage();
        assertTrue(got, got.contains(message));
    }

    static void closed(ThrowingRunnable call) {
        assertThrows(IllegalStateException.class, call);
    }

    static void loadTipping(Renderer r, String declared) {
        r.loadControllers(0, TIPPING, new String[] {"animation_controllers/glasses.json"},
                new byte[][] {TIP.getBytes(StandardCharsets.UTF_8)}, new String[] {declared});
    }

    @Test
    public void aGameHostRoundTripsPartsAndBuildsAMesh() {
        try (Renderer r = new Renderer()) {
            r.loadPlayer(0, false);
            r.setVisible(0, false);
            r.loadAnimation(0, "{\"animations\":{\"nod\":{\"bones\":{\"head\":{\"rotation\":[30,0,0]}}}}}");
            r.play(0, 0);
            r.loadCosmetic(1, GLASSES, "glasses", "", 16, 16);
            r.pose(0);

            float[] d = r.partDelta(0, "head");
            assertEquals(Math.toRadians(30), d[3], 1e-6);
            r.setGamePart("head", 0, 0, 0, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7], d[8]);
            assertNull(r.partDelta(0, "tail"));

            assertEquals("only the glasses: the hidden player is the game's to draw", 6, r.buildMesh(Renderer.JAVA_MODEL_SPACE));
            assertArrayEquals(new int[] {1, 0, 6, 0}, r.meshRuns());
            assertEquals(6 * 4 * 3, r.meshPositions().length);
            assertEquals(6 * 4, r.meshGlow().length);
            assertTrue(r.build(0, 10, 62, 18, 512, 512, 0) > 0);
            assertEquals(r.positions().length / 2, r.colors().length);
        }
    }

    @Test
    public void startTimesKeepTheirPrecisionThroughPoseFrameAndBuild() {
        try (Renderer r = new Renderer()) {
            r.loadPlayer(0, false);
            r.loadAnimation(0, TURN);
            double start = 1_800_000_000.25;
            r.play(0, start);
            r.pose(start + 0.5);
            assertEquals(Math.toRadians(5), r.partDelta(0, "head")[3], 1e-4);
            assertTrue(r.frame(start + 1.0, new float[0], 0, 0, Renderer.JAVA_MODEL_SPACE, new float[256]) >= 0);
            assertEquals(Math.toRadians(10), r.partDelta(0, "head")[3], 1e-4);
            r.play(0, 1_800_000_001L);
            assertTrue(r.frame(1_800_000_003L, new float[0], 0, 0, Renderer.JAVA_MODEL_SPACE, new float[256]) >= 0);
            assertEquals(Math.toRadians(20), r.partDelta(0, "head")[3], 1e-4);
        }
    }

    @Test
    public void errorsBecomeExceptions() {
        try (Renderer r = new Renderer()) {
            r.loadCosmetic(0, GLASSES, "glasses", "", 16, 16);
            r.loadAnimation(0, SNEAK);
            fails("unknown body slot", () -> r.loadCosmetic(1, GLASSES, "tail", "", 0, 0));
            int stride = Renderer.wearerInputStride(0, 0);
            int outStride = r.wearerOutputStride();
            long[] wearers = {1L, 2L};
            float[] input = new float[stride * 2];
            float[] out = new float[outStride * 2];
            int space = Renderer.JAVA_MODEL_SPACE;
            fails("input holds " + (stride * 2 - 1), () -> r.frameWearers(1, wearers, 2, new float[stride * 2 - 1], 0, 0, space, out));
            fails("out holds " + (outStride * 2 - 1), () -> r.frameWearers(1, wearers, 2, input, 0, 0, space, new float[outStride * 2 - 1]));
            fails("wearers holds 1", () -> r.frameWearers(1, new long[1], 2, input, 0, 0, space, out));
            fails("no motion 3", () -> r.setMotion(3));
            fails("", () -> r.setSignalOverride(1L, Renderer.SIGNAL_COUNT, 1));
            fails("controller_missing", () -> loadTipping(r, "controller.animation.glasses.gone"));
            assertEquals(0, r.wearerCount());
            assertTrue(r.frameWearers(0, null, 0, null, 0, 0, 0, null) >= 0);
            assertTrue(r.frameWearers(2, new long[] {5L}, 0, null, 1, 0, space, new float[0]) >= 0);
            assertEquals(0, r.wearerCount());
            fails("out holds 1", () -> r.frame(0, input, 0, 1, space, new float[1]));

            String nod = "{\"animations\":{\"nod\":{\"bones\":{\"head\":{\"rotation\":[30,0,0]}}}}}";
            for (int slot : new int[] {-1, Renderer.SLOTS, Integer.MIN_VALUE}) {
                ThrowingRunnable[] calls = {
                    () -> r.loadPlayer(slot, false), () -> r.loadCosmetic(slot, GLASSES, "glasses", "", 16, 16),
                    () -> r.loadCape(slot, 64, 32, 0, 0, ""), () -> r.loadElytra(slot, 64, 32, 0, 0, ""),
                    () -> r.loadAnimation(slot, nod), () -> r.loadControllers(slot, nod, new String[0], new byte[0][], new String[0]),
                    () -> r.play(slot, 0.0), () -> r.stop(slot), () -> r.setElytraPose(slot, Renderer.ELYTRA_FLYING, 0.5f),
                    () -> r.setVisible(slot, false), () -> r.setScale(slot, 2f), () -> r.clear(slot),
                };
                for (ThrowingRunnable call : calls) fails("no slot " + slot + ", the last is 255", call);
            }
            r.close();
            closed(() -> r.pose(0));
        }
    }

    @Test
    public void bundlePartsPickByKind() {
        String[] names = {"wings.png", "wings.geo.json"};
        assertEquals(1, Renderer.bundlePart(names, Renderer.GEOMETRY));
        assertEquals(0, Renderer.bundlePart(names, Renderer.TEXTURE));
        assertEquals(-1, Renderer.bundlePart(names, Renderer.SHEET));
        assertArrayEquals(new int[] {2, 1}, Renderer.bundleControllers(
                new String[] {"a.png", "x.animation_controllers.json", "animation_controllers/y.json"}));
    }

    @Test
    public void aSkinPosedByFrameIsTheMesh() {
        try (Renderer r = new Renderer()) {
            r.loadPlayer(0, false);
            r.setVisible(0, false);
            r.loadCosmetic(1, GLASSES, "glasses", "", 16, 16);
            int head = r.partId("head");
            int moving = r.queryId("is_moving");
            float[] input = new float[12 + 1];
            input[head * 12 + 6] = 0.5f;
            input[head * 12 + 9] = input[head * 12 + 10] = input[head * 12 + 11] = 1;
            input[12 + moving] = 1;

            r.setGamePart("head", 0, 0, 0, 0, 0, 0, 0.5f, 0, 0, 1, 1, 1);
            r.pose(0);
            int quads = r.buildMesh(Renderer.JAVA_MODEL_SPACE);
            float[] mesh = r.meshPositions();
            int layout = r.layout();
            assertEquals(quads, r.buildSkin());
            assertArrayEquals(r.meshRuns(), r.skinRuns());
            float[] skin = r.skinPositions();
            int[] bones = r.skinBones();

            float[] out = new float[64];
            int boneCount = r.frame(0, input, 1, 1, Renderer.JAVA_MODEL_SPACE, out);
            assertEquals(r.skinBoneCount(), boneCount);
            assertEquals(layout, r.layout());
            for (int v = 0; v < bones.length; v++) {
                int m = bones[v] * 12;
                for (int row = 0; row < 3; row++) {
                    float p = out[m + row * 4] * skin[v * 3] + out[m + row * 4 + 1] * skin[v * 3 + 1]
                            + out[m + row * 4 + 2] * skin[v * 3 + 2] + out[m + row * 4 + 3];
                    assertEquals(mesh[v * 3 + row], p, 1e-5);
                }
            }
            assertEquals("one run, on its first frame", 0, out[boneCount * 12], 0);
        }
    }
}
