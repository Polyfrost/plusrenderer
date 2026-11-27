package org.polyfrost.plusrenderer;

import static org.junit.Assert.*;
import static org.polyfrost.plusrenderer.RendererTest.closed;
import static org.polyfrost.plusrenderer.RendererTest.fails;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Arrays;
import java.util.List;
import java.util.stream.Collectors;
import java.util.stream.Stream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.junit.Test;

public class CosmeticAssetTest {
    private static final Path DATA = Paths.get("..", "crates", "core", "tests", "data");
    private static final String[] EFFECT_CAPS = {"effects", "particles", "sounds", "trails", "events"};
    private static final String[] CAPS = {"controllers", "reactions", "palettes", "custom_colors"};
    private static final int IS_IN_RAIN = 4, IS_ON_GROUND = 11, TIME_OF_DAY = 19;

    static byte[] zip(Path dir) throws IOException {
        List<Path> files;
        try (Stream<Path> walk = Files.walk(dir)) {
            files = walk.filter(Files::isRegularFile).sorted().collect(Collectors.toList());
        }
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (ZipOutputStream zip = new ZipOutputStream(bytes)) {
            for (Path file : files) {
                zip.putNextEntry(new ZipEntry(dir.relativize(file).toString().replace(File.separatorChar, '/')));
                zip.write(Files.readAllBytes(file));
            }
        }
        return bytes.toByteArray();
    }

    static CosmeticAsset parse() throws IOException {
        return CosmeticAsset.parse(zip(DATA.resolve("sample_wings")), CosmeticAsset.MODE_CLIENT, 42L, CAPS);
    }

    private static CosmeticAsset spark() throws IOException {
        return CosmeticAsset.parse(zip(DATA.resolve("sample_spark")), CosmeticAsset.MODE_CLIENT, 7L, EFFECT_CAPS);
    }

    static int[] sections(int... pairs) {
        int[] sections = new int[Renderer.SECTION_INTS];
        for (int i = 0; i < pairs.length; i += 2) sections[pairs[i]] = pairs[i + 1];
        return sections;
    }

    static int[] inputLayout(int[] sections) {
        int[] layout = new int[Renderer.INPUT_LAYOUT_INTS];
        assertEquals(Renderer.inputLayout(sections, layout), layout[Renderer.INPUT_STRIDE]);
        return layout;
    }

    static float[] rows(int[] layout, boolean... raining) {
        int stride = layout[Renderer.INPUT_STRIDE];
        float[] input = new float[stride * raining.length];
        for (int i = 0; i < raining.length; i++) {
            int signals = i * stride + layout[Renderer.INPUT_SIGNALS_AT];
            input[signals + IS_ON_GROUND] = 1;
            input[signals + TIME_OF_DAY] = 0.25f;
            input[signals + IS_IN_RAIN] = raining[i] ? 1 : 0;
        }
        return input;
    }

    @Test
    public void bundlesParseAndReadBackAsJson() throws IOException {
        try (CosmeticAsset asset = parse()) {
            assertEquals(asset.issuesJson(), CosmeticAsset.STATUS_ASSET, asset.status());
            String meta = asset.metaJson();
            assertTrue(meta, meta.startsWith("{\"id\":\"sample_wings\",\"identity\":\"42\",\"kind\":\"item\""));
            assertTrue(asset.analyzeJson().contains("\"quads\":6"));
        }
        try (CosmeticAsset skipped = CosmeticAsset.parse(zip(DATA.resolve("sample_wings")), CosmeticAsset.MODE_CLIENT, 1L);
             Renderer renderer = new Renderer()) {
            assertEquals(CosmeticAsset.STATUS_SKIPPED, skipped.status());
            fails("skipped", skipped::metaJson);
            fails("the bundle did not load: it is skipped", () -> renderer.loadAsset(0, skipped));
        }
        fails("no mode 7", () -> CosmeticAsset.parse(new byte[0], 7, 1L));
    }

    @Test
    public void aShapedFrameCarriesPosesColorwaysLocatorsAndPartDeltas() throws IOException {
        try (CosmeticAsset asset = parse(); Renderer renderer = new Renderer()) {
            renderer.loadAsset(0, asset);
            renderer.play(0, 0.0);
            assertEquals(0, renderer.locatorId("head"));
            int[] sections = sections(Renderer.SECTION_COLORWAY, 1);
            int[] in = inputLayout(sections);
            int[] layout = new int[Renderer.LAYOUT_INTS];
            int stride = renderer.outputLayout(sections, layout);
            assertEquals(stride, layout[Renderer.LAYOUT_COLORWAY_AT] + Renderer.COLORWAY_FLOATS);

            long[] wearers = {1L, 2L};
            float[] out = new float[stride * 2];
            renderer.frameWearersShaped(1.0, wearers, 2, sections, rows(in, false, true), Renderer.CORE_SPACE, out);
            int bones = renderer.frameWearersShaped(3.0, wearers, 2, sections, rows(in, false, true), Renderer.CORE_SPACE, out);
            assertEquals(layout[Renderer.LAYOUT_BONES], bones);
            int colorway = layout[Renderer.LAYOUT_COLORWAY_AT];
            assertEquals(Renderer.NO_COLORWAY, out[colorway], 0f);
            assertEquals("storm while it rains", 0f, out[stride + colorway], 0f);

            fails("out holds", () -> renderer.frameWearersShaped(4.0, wearers, 2, sections, rows(in, false, true),
                    Renderer.CORE_SPACE, new float[stride]));
            renderer.setRig(Renderer.RIG_ACTOR);
            fails("no rig 6", () -> renderer.setRig(6));
        }
    }

    @Test
    public void texturesResolveThroughColorwaysAndCustomColours() throws IOException {
        try (CosmeticAsset asset = parse()) {
            int[] size = new int[2];
            assertEquals(1024, asset.resolveTexture(0, CosmeticAsset.NO_COLORWAY, null, null, size, null));
            assertArrayEquals(new int[] {16, 16}, size);
            byte[] plain = new byte[1024], storm = new byte[1024], custom = new byte[1024];
            asset.resolveTexture(0, CosmeticAsset.NO_COLORWAY, null, null, null, plain);
            asset.resolveTexture(0, 0, null, null, null, storm);
            asset.resolveTexture(0, CosmeticAsset.NO_COLORWAY, new int[] {0}, new int[] {0xff0000}, null, custom);
            assertFalse(Arrays.equals(plain, storm) || Arrays.equals(plain, custom) || Arrays.equals(storm, custom));
            long key = asset.textureKey(0, CosmeticAsset.NO_COLORWAY, null, null);
            assertNotEquals(key, asset.textureKey(0, CosmeticAsset.NO_COLORWAY, new int[] {0}, new int[] {0xff0000}));

            fails("rgba holds 4 bytes", () -> asset.resolveTexture(0, -1, null, null, null, new byte[4]));
        }
    }

    @Test
    public void effectPixelsAndSoundFilesReadBackAndFireByName() throws IOException {
        try (CosmeticAsset asset = spark(); Renderer renderer = new Renderer()) {
            byte[] rgba = new byte[16];
            assertEquals(16, asset.effectTexture(0, rgba));
            assertArrayEquals(new byte[] {(byte) 255, 0, 0, (byte) 255}, Arrays.copyOf(rgba, 4));
            assertArrayEquals(Files.readAllBytes(DATA.resolve("sample_spark/sounds/pop.ogg")), asset.sound(0));
            assertFalse(renderer.fireEffect(3L, "join"));
            renderer.loadAsset(0, asset);
            assertTrue(renderer.fireEffect(3L, "join"));
            assertThrows(NullPointerException.class, () -> renderer.fireEffect(3L, null));
            renderer.setEffectColors(3L, 0, new int[] {0}, new int[] {0xff0000});
            fails("no colorway -2", () -> renderer.setEffectColors(3L, -2, null, null));
        }
    }

    @Test
    public void panicsAndClosedHandlesThrowWhileTheSceneKeepsItsOwnCopy() throws IOException {
        Renderer renderer = new Renderer();
        CosmeticAsset asset = parse();
        renderer.loadAsset(0, asset);
        asset.close();
        asset.close();
        closed(asset::metaJson);
        closed(() -> renderer.loadAsset(1, asset));
        int[] sections = sections(Renderer.SECTION_COLORWAY, 1);
        float[] input = rows(inputLayout(sections), true);
        float[] out = new float[renderer.outputLayout(sections, null)];
        renderer.faultNextFrame();
        String panic = assertThrows(IllegalStateException.class,
                () -> renderer.frameWearersShaped(1.0, new long[] {1L}, 1, sections, input, Renderer.CORE_SPACE, out)).getMessage();
        assertEquals("the core failed on this frame", panic);
        assertTrue(renderer.frameWearersShaped(1.1, new long[] {1L}, 1, sections, input, Renderer.CORE_SPACE, out) > 0);
        renderer.close();
        renderer.close();
        closed(renderer::hasEffects);
    }
}
