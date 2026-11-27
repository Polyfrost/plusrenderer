package org.polyfrost.plusrenderer;

import static org.junit.Assert.*;
import static org.polyfrost.plusrenderer.RendererTest.closed;
import static org.polyfrost.plusrenderer.RendererTest.fails;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.stream.Collectors;
import java.util.stream.Stream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.junit.Test;

public class NameplatesTest {
    private static final Path KITS = Paths.get("..", "crates", "core", "src", "nameplate", "format", "testdata");

    static byte[] kit(String name, String... customizable) throws IOException {
        Path root = KITS.resolve(name);
        List<Path> files;
        try (Stream<Path> walk = Files.walk(root)) {
            files = walk.filter(Files::isRegularFile).sorted().collect(Collectors.toList());
        }
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (ZipOutputStream zip = new ZipOutputStream(bytes)) {
            for (Path file : files) {
                zip.putNextEntry(new ZipEntry(root.relativize(file).toString().replace(File.separatorChar, '/')));
                String text = new String(Files.readAllBytes(file), StandardCharsets.ISO_8859_1);
                for (String slot : customizable) {
                    text = text.replace("\"name\": \"" + slot + "\",", "\"name\": \"" + slot + "\", \"customizable\": true,");
                }
                zip.write(text.getBytes(StandardCharsets.ISO_8859_1));
            }
        }
        return bytes.toByteArray();
    }

    static NameplateAsset parse(String kit, String... customizable) throws IOException {
        return NameplateAsset.parse(kit(kit, customizable), NameplateAsset.MODE_CLIENT, 0);
    }

    static int[] rows(int... assetsAndWidths) {
        int count = assetsAndWidths.length / 2;
        int[] tags = new int[count * Nameplates.TAG_INTS];
        for (int i = 0; i < count; i++) {
            int at = i * Nameplates.TAG_INTS, width = assetsAndWidths[i * 2 + 1];
            tags[at + Nameplates.TAG_ASSET] = assetsAndWidths[i * 2];
            tags[at + Nameplates.TAG_LINE_COUNT] = 1;
            tags[at + Nameplates.TAG_ALIGN] = Nameplates.ALIGN_LEFT;
            tags[at + Nameplates.TAG_NAME_WIDTH] = tags[at + Nameplates.TAG_LINE_WIDTH_0] = width;
            tags[at + Nameplates.TAG_BG_X0] = tags[at + Nameplates.TAG_BG_Y0] = -1;
            tags[at + Nameplates.TAG_BG_X1] = width;
            tags[at + Nameplates.TAG_BG_Y1] = 9;
            tags[at + Nameplates.TAG_SNEAK_ALPHA] = 255;
        }
        return tags;
    }

    private static int frame(Nameplates plates, int[] tags, int[] out) {
        int count = tags.length / Nameplates.TAG_INTS;
        long[] wearers = new long[count];
        for (int i = 0; i < count; i++) wearers[i] = i + 1;
        return plates.frame(10.0, wearers, count, tags, new float[count * Nameplates.SIGNAL_FLOATS], null, null, out);
    }

    private static int status(int[] out, int tag) {
        return out[Nameplates.OUT_HEADER + tag * Nameplates.DIR_WORDS + Nameplates.DIR_STATUS];
    }

    private static List<Integer> ops(int[] out, int tag, int word) {
        int at = out[Nameplates.OUT_HEADER + tag * Nameplates.DIR_WORDS + Nameplates.DIR_OFFSET];
        List<Integer> values = new ArrayList<>();
        for (int k = 0; k < out[at + Nameplates.HEAD_OPS]; k++) {
            values.add(out[at + Nameplates.TAG_HEAD + k * Nameplates.OP_WORDS + word]);
        }
        return values;
    }

    @Test
    public void aKitBundleFramesThreeTagsAndReadsItsOpsBack() throws IOException {
        try (NameplateAsset asset = parse("test_plate", "Fill", "Gem"); Nameplates plates = new Nameplates()) {
            assertEquals(asset.issuesJson(), NameplateAsset.STATUS_ASSET, asset.status());
            assertTrue(asset.analyzeJson().contains("\"max_out_words\":"));
            int id = plates.addAsset(asset);
            assertEquals(1, id);
            int[] tags = rows(id, 40, id, 60, id, 24);
            int needed = plates.neededWords(tags, 3);
            int[] out = new int[needed];
            int words = frame(plates, tags, out);
            assertTrue(words > 0 && words <= needed);
            assertEquals(3, out[Nameplates.OUT_TAGS]);
            assertEquals(words, out[Nameplates.OUT_WORDS]);
            assertEquals(plates.stripLayout(), out[Nameplates.OUT_STRIP_LAYOUT]);
            for (int tag = 0; tag < 3; tag++) {
                assertEquals(Nameplates.STATUS_OK, status(out, tag));
                assertFalse(ops(out, tag, Nameplates.OP_BAND_LAYER_KIND).isEmpty());
            }
            List<Integer> designed = ops(out, 0, Nameplates.OP_ARGB), other = ops(out, 1, Nameplates.OP_ARGB);
            plates.setCustomColors(1L, id, new int[] {1, 2}, new int[] {0x3fa34d, 0xe0a020});
            frame(plates, tags, out);
            assertNotEquals(designed, ops(out, 0, Nameplates.OP_ARGB));
            assertEquals("only the recoloured wearer changes", other, ops(out, 1, Nameplates.OP_ARGB));
            fails("1 custom slots for 2 colors", () -> plates.setCustomColors(1L, id, new int[] {1}, new int[] {1, 2}));
            assertEquals(3, plates.wearerCount());
            assertTrue(plates.forgetWearer(1L));
            assertFalse(plates.forgetWearer(1L));
            plates.setVanillaTexture("glint", 1, 1, new byte[4]);
            plates.removeAsset(id);
            frame(plates, tags, out);
            assertEquals(Nameplates.STATUS_UNKNOWN_ASSET, status(out, 0));
        }
    }

    @Test
    public void materialsDrawOnTheGpuOrAsCpuImagesFromFallbacksOnAnyThread() throws Exception {
        try (NameplateAsset asset = parse("material_plate"); Nameplates plates = new Nameplates()) {
            int id = plates.addAsset(asset);
            int[] tags = rows(id, 40, id, 40);
            tags[Nameplates.TAG_INTS + Nameplates.TAG_FLAGS] |= Nameplates.FLAG_EFFECTS_CPU;
            int[] out = new int[plates.neededWords(tags, 2)];
            plates.frame(60.0, new long[] {1L, 2L}, 2, tags, new float[2 * Nameplates.SIGNAL_FLOATS], null, null, out);
            int gpu = out[Nameplates.OUT_HEADER + Nameplates.DIR_OFFSET];
            assertEquals(3, out[gpu + Nameplates.HEAD_MATERIALS]);
            assertTrue(ops(out, 1, Nameplates.OP_BAND_LAYER_KIND).isEmpty());

            assertEquals(3, plates.pendingFallbacks());
            Nameplates.Fallback fallback = plates.takeFallback();
            Thread worker = new Thread(fallback::run);
            worker.start();
            worker.join();
            assertTrue(plates.finishFallback(fallback));
            assertFalse(plates.finishFallback(fallback));
            assertEquals(0, plates.runFallbacks(Integer.MAX_VALUE));
            assertNull(plates.takeFallback());
            plates.frame(60.0, new long[] {1L, 2L}, 2, tags, new float[2 * Nameplates.SIGNAL_FLOATS], null, null, out);
            int cpu = out[Nameplates.OUT_HEADER + Nameplates.DIR_WORDS + Nameplates.DIR_OFFSET];
            int images = 0;
            for (int k = 0; k < out[cpu + Nameplates.HEAD_OPS]; k++) {
                int op = cpu + Nameplates.TAG_HEAD + k * Nameplates.OP_WORDS;
                if (((out[op + Nameplates.OP_BAND_LAYER_KIND] >> 16) & 0xff) != Nameplates.OP_KIND_IMAGE) continue;
                images++;
                int strip = out[op + Nameplates.OP_AUX0];
                int width = out[op + Nameplates.OP_AUX1] & Nameplates.AUX_LOW;
                int count = plates.stripTexels(strip, null);
                assertEquals(width * (out[op + Nameplates.OP_Y1] - out[op + Nameplates.OP_Y0]), count);
                fails("out holds " + (count - 1) + " texels", () -> plates.stripTexels(strip, new int[count - 1]));
                assertEquals(count, plates.stripTexels(strip, new int[count + 3]));
            }
            assertEquals(3, images);
            assertEquals(0, plates.stripTexels(-1, new int[1]));

        }
    }

    @Test
    public void aFallbackOutlivesItsClosedRenderer() throws IOException {
        try (NameplateAsset asset = parse("material_plate")) {
            Nameplates plates = new Nameplates();
            int[] tags = rows(plates.addAsset(asset), 40);
            tags[Nameplates.TAG_FLAGS] |= Nameplates.FLAG_EFFECTS_CPU;
            frame(plates, tags, new int[plates.neededWords(tags, 1)]);
            Nameplates.Fallback fallback = plates.takeFallback();
            plates.close();
            closed(() -> plates.finishFallback(fallback));
            fallback.run();
            fallback.close();
            closed(fallback::run);
        }
    }

    @Test
    public void jsonAndTexturesComeFromTheParsedBundle() throws IOException {
        try (NameplateAsset asset = NameplateAsset.parse(kit("material_plate"), NameplateAsset.MODE_AUTHORING, 7L)) {
            assertTrue(asset.shadersJson(NameplateAsset.DIALECT_CORE_330).contains("\"shaders\":[{\"key\":"));
            fails("no dialect 6", () -> asset.shadersJson(NameplateAsset.DIALECT_COUNT));
            assertTrue(asset.metaJson().startsWith("{\"id\":\"material_plate\","));
            int[] size = new int[2];
            int bytes = asset.texture(0, size, null);
            assertEquals(size[0] * size[1] * 4, bytes);
            byte[] rgba = new byte[bytes];
            assertEquals(bytes, asset.texture(0, null, rgba));
            fails("the texture needs " + bytes, () -> asset.texture(0, size, new byte[bytes - 1]));
        }
        try (NameplateAsset bad = NameplateAsset.parse("PKgarbage".getBytes(StandardCharsets.UTF_8), NameplateAsset.MODE_AUTHORING, 0);
             Nameplates plates = new Nameplates()) {
            assertEquals(NameplateAsset.STATUS_INVALID, bad.status());
            fails("invalid", () -> plates.addAsset(bad));
        }
        assertTrue(NameplateAsset.checkMolangJson("q.name_width * 2", NameplateAsset.MOLANG_CHANNEL).startsWith("{\"ok\":true"));
        fails("no MoLang context 9", () -> NameplateAsset.checkMolangJson("1", 9));
    }

    @Test
    public void badCallsThrowAndClosingIsSafeInAnyOrder() throws IOException {
        NameplateAsset asset = parse("test_plate");
        Nameplates plates = new Nameplates();
        int id = plates.addAsset(asset);
        int[] tags = rows(id, 40, id, 40);
        float[] signals = new float[2 * Nameplates.SIGNAL_FLOATS];
        long[] wearers = {1L, 2L};
        int[] out = new int[plates.neededWords(tags, 2)];
        fails("needs at least", () -> plates.frame(1, wearers, 2, tags, signals, null, null, new int[4]));
        fails("wearers holds 1", () -> plates.frame(1, new long[1], 2, tags, signals, null, null, out));
        fails("out is null", () -> plates.frame(1, wearers, 2, tags, signals, null, null, null));
        fails("needs 16", () -> plates.setVanillaTexture("glint", 2, 2, new byte[15]));
        fails("no signal -1", () -> plates.setSignalOverride(1L, -1, 0));
        int[] small = new int[Nameplates.OUT_HEADER + 2 * Nameplates.DIR_WORDS];
        assertEquals(small.length, plates.frame(1, wearers, 2, tags, signals, null, null, small));
        assertEquals(Nameplates.STATUS_NO_ROOM, status(small, 0));

        plates.faultNextFrame();
        String panic = assertThrows(IllegalStateException.class, () -> frame(plates, tags, out)).getMessage();
        assertTrue(panic, panic.contains("core failed"));
        asset.close();
        asset.close();
        assertTrue(frame(plates, tags, out) > 0);
        assertEquals("the renderer keeps its own reference", Nameplates.STATUS_OK, status(out, 0));
        closed(asset::metaJson);
        closed(() -> plates.addAsset(asset));
        plates.close();
        plates.close();
        closed(() -> frame(plates, tags, out));
        closed(() -> plates.removeAsset(id));
    }

    @Test
    public void constantsMirrorTheCore() throws Exception {
        String json = NameplateAsset.constantsJson();
        Matcher numbers = Pattern.compile("\"(\\w+)\":(-?\\d+)[,}]").matcher(json);
        int depth = 0, scanned = 0, checked = 0;
        while (numbers.find()) {
            for (; scanned < numbers.start(); scanned++) {
                char c = json.charAt(scanned);
                depth += c == '{' || c == '[' ? 1 : c == '}' || c == ']' ? -1 : 0;
            }
            if (depth != 1) continue;
            String key = numbers.group(1);
            assertEquals(key, Integer.parseInt(numbers.group(2)), Nameplates.class.getField(key.toUpperCase(Locale.ROOT)).getInt(null));
            checked++;
        }
        assertTrue("" + checked, checked > 50);
    }
}
