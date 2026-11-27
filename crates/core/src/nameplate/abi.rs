use crate::signals::SIGNAL_COUNT;

use super::geom::{Align, BlockInput, Rect, MAX_LINES};

pub const TAG_INTS: usize = 24;
pub const TAG_ASSET: usize = 0;
pub const TAG_VARIANT: usize = 1;
pub const TAG_FLAGS: usize = 2;
pub const TAG_DETAIL: usize = 3;
pub const TAG_LINE_COUNT: usize = 4;
pub const TAG_NAME_LINE: usize = 5;
pub const TAG_ALIGN: usize = 6;
pub const TAG_NAME_X: usize = 7;
pub const TAG_NAME_WIDTH: usize = 8;
pub const TAG_PREFIX_WIDTH: usize = 9;
pub const TAG_LINE_WIDTH_0: usize = 10;
pub const TAG_LINE_WIDTH_1: usize = 11;
pub const TAG_LINE_WIDTH_2: usize = 12;
pub const TAG_BG_X0: usize = 13;
pub const TAG_BG_Y0: usize = 14;
pub const TAG_BG_X1: usize = 15;
pub const TAG_BG_Y1: usize = 16;
pub const TAG_EXTERNAL_COLOR: usize = 17;
pub const TAG_SNEAK_ALPHA: usize = 18;
pub const TAG_GLYPH_OFFSET: usize = 19;
pub const TAG_GLYPH_COUNT: usize = 20;
pub const TAG_COLOR_OFFSET: usize = 21;
pub const TAG_COLOR_COUNT: usize = 22;
pub const TAG_RESERVED: usize = 23;

pub const FLAG_DISPLAY: i32 = 1 << 0;
pub const FLAG_OWN: i32 = 1 << 1;
pub const FLAG_ANIMATED_NAMES_OFF: i32 = 1 << 2;
pub const FLAG_EFFECTS_CPU: i32 = 1 << 3;
pub const FLAG_EFFECTS_OFF: i32 = 1 << 4;
pub const FLAG_EXTERNAL_COLOR: i32 = 1 << 5;
pub const FLAG_BADGE: i32 = 1 << 6;
pub const FLAGS_KNOWN: i32 = (1 << 7) - 1;

pub const GLYPH_FLOATS: usize = 2;
pub const GLYPH_X: usize = 0;
pub const GLYPH_WIDTH: usize = 1;

pub const SIGNAL_FLOATS: usize = SIGNAL_COUNT + 1;
pub const SIGNAL_EVENTS: usize = SIGNAL_COUNT;

pub const OUT_TAGS: usize = 0;
pub const OUT_WORDS: usize = 1;
pub const OUT_STRIP_LAYOUT: usize = 2;
pub const OUT_HEADER: usize = 3;

pub const DIR_WORDS: usize = 3;
pub const DIR_OFFSET: usize = 0;
pub const DIR_LEN: usize = 1;
pub const DIR_STATUS: usize = 2;

pub const STATUS_OK: i32 = 0;
pub const STATUS_VANILLA: i32 = 1;
pub const STATUS_UNKNOWN_ASSET: i32 = 2;
pub const STATUS_BAD_ROW: i32 = 3;
pub const STATUS_DISABLED: i32 = 4;
pub const STATUS_NO_ROOM: i32 = 5;

pub const TAG_HEAD: usize = 8;
pub const HEAD_PLATE_DX: usize = 0;
pub const HEAD_PLATE_DY: usize = 1;
pub const HEAD_OWNS_BACKGROUND: usize = 2;
pub const HEAD_TEXT_MODE: usize = 3;
pub const HEAD_OPS: usize = 4;
pub const HEAD_MATERIALS: usize = 5;
pub const HEAD_MATERIAL_WORDS: usize = 6;
pub const HEAD_EVENTS: usize = 7;

pub const TEXT_MODE_VANILLA: i32 = 0;
pub const TEXT_MODE_MOTION: i32 = 1;
pub const TEXT_MODE_OWNED: i32 = 2;

pub const OP_WORDS: usize = 8;
pub const OP_X0: usize = 0;
pub const OP_Y0: usize = 1;
pub const OP_X1: usize = 2;
pub const OP_Y1: usize = 3;
pub const OP_ARGB: usize = 4;
pub const OP_BAND_LAYER_KIND: usize = 5;
pub const OP_AUX0: usize = 6;
pub const OP_AUX1: usize = 7;

pub const OP_KIND_RECT: i32 = 0;
pub const OP_KIND_STRIP: i32 = 1;
pub const OP_KIND_IMAGE: i32 = 2;

pub const AUX_LOW: i32 = 0xffff;
pub const AUX_AXIS_Y: i32 = 1 << 16;
pub const AUX_MIRROR: i32 = 1 << 17;
pub const AUX_ADDITIVE: i32 = 1 << 18;

pub const BAND_BEHIND: i32 = 0;
pub const BAND_BACKGROUND: i32 = 1;
pub const BAND_OVER_BACKGROUND: i32 = 2;
pub const BAND_OVER_TEXT: i32 = 3;
pub const BAND_BADGE: i32 = 4;

pub const MAT_SHADER: usize = 0;
pub const MAT_BAND_LAYER: usize = 1;
pub const MAT_RUN: usize = 2;
pub const MAT_AREA_X0: usize = 3;
pub const MAT_AREA_Y0: usize = 4;
pub const MAT_AREA_X1: usize = 5;
pub const MAT_AREA_Y1: usize = 6;
pub const MAT_CLIP_COUNT: usize = 7;
pub const MAT_UNIFORM_WORDS: usize = 8;
pub const MAT_FIXED: usize = 9;
pub const CLIP_WORDS: usize = 4;
pub const MAT_MIRRORED: i32 = 1 << 16;
pub const MAT_OPACITY_SHIFT: u32 = 24;

pub const TEXT_COLOR: usize = 0;
pub const TEXT_SHADOW: usize = 1;
pub const TEXT_SHADOW_USE: usize = 2;
pub const TEXT_OUTLINE: usize = 3;
pub const TEXT_LETTER_COUNT: usize = 4;
pub const TEXT_FIXED: usize = 5;

pub const SHADOW_LEAVE: i32 = 0;
pub const SHADOW_REMOVE: i32 = 1;
pub const SHADOW_ALWAYS: i32 = 2;
pub const SHADOW_WITH_DROP: i32 = 3;

pub const LETTER_WORDS: usize = 7;
pub const LETTER_DX: usize = 0;
pub const LETTER_DY: usize = 1;
pub const LETTER_ARGB: usize = 2;
pub const LETTER_SHADOW: usize = 3;
pub const LETTER_OPACITY: usize = 4;
pub const LETTER_SCALE_ROT: usize = 5;
pub const LETTER_OUTLINE: usize = 6;

pub const BADGE_WORDS: usize = 5;
pub const BADGE_X: usize = 0;
pub const BADGE_Y: usize = 1;
pub const BADGE_OPACITY: usize = 2;
pub const BADGE_TINT: usize = 3;
pub const BADGE_DECORATION: usize = 4;

pub const DECORATION_NONE: i32 = 0;
pub const DECORATION_SHADOW: i32 = 1;
pub const DECORATION_OUTLINE: i32 = 2;
pub const DECORATION_KIND_SHIFT: u32 = 24;

pub const EVENT_WORDS: usize = 2;
pub const EVENT_CODE: usize = 0;
pub const EVENT_LAYER: usize = 1;

pub const EVENT_LAYER_DISABLED: i32 = 1;
pub const EVENT_MOLANG_OVER_BUDGET: i32 = 2;
pub const EVENT_ASSET_DISABLED: i32 = 3;
pub const EVENT_CODES: [i32; 3] = [
    EVENT_LAYER_DISABLED,
    EVENT_MOLANG_OVER_BUDGET,
    EVENT_ASSET_DISABLED,
];

pub const EVENT_TARGET_TAG: i32 = -1;
pub const EVENT_TARGET_PALETTE: i32 = 32;
pub const EVENT_TARGET_LETTERS: i32 = 33;
pub const EVENT_TARGET_BADGE: i32 = 34;
pub const EVENT_TARGET_PLATE: i32 = 35;

pub fn pack_band_layer_kind(band: i32, layer: i32, kind: i32) -> i32 {
    (band & 0xff) | ((layer & 0xff) << 8) | ((kind & 0xff) << 16)
}

pub fn unpack_band_layer_kind(word: i32) -> (i32, i32, i32) {
    (word & 0xff, (word >> 8) & 0xff, (word >> 16) & 0xff)
}

pub fn pack_band_layer(band: i32, layer: i32) -> i32 {
    (band & 0xff) | ((layer & 0xff) << 8)
}

pub fn pack_material(band: i32, layer: i32, mirrored: bool, opacity: u8) -> i32 {
    let mirror = if mirrored { MAT_MIRRORED } else { 0 };
    pack_band_layer(band, layer) | mirror | ((u32::from(opacity) << MAT_OPACITY_SHIFT) as i32)
}

pub fn unpack_material(word: i32) -> (i32, i32, bool, u8) {
    (
        word & 0xff,
        (word >> 8) & 0xff,
        word & MAT_MIRRORED != 0,
        ((word as u32) >> MAT_OPACITY_SHIFT) as u8,
    )
}

pub fn pack_scale_rotation(scale_q: u8, rotation: i16) -> i32 {
    i32::from(scale_q) | (i32::from(rotation) << 8)
}

pub fn unpack_scale_rotation(word: i32) -> (u8, i16) {
    ((word & 0xff) as u8, (word >> 8) as i16)
}

pub fn pack_decoration(kind: i32, rgb: u32) -> i32 {
    ((kind & 0xff) << DECORATION_KIND_SHIFT) | (rgb & 0x00ff_ffff) as i32
}

pub fn read_block(row: &[i32]) -> Option<BlockInput> {
    if row.len() < TAG_INTS {
        return None;
    }
    let line_count = u8::try_from(row[TAG_LINE_COUNT]).ok()?;
    let name_line = u8::try_from(row[TAG_NAME_LINE]).ok()?;
    if line_count == 0 || usize::from(line_count) > MAX_LINES || name_line >= line_count {
        return None;
    }
    let align = Align::from_value(row[TAG_ALIGN])?;
    let flags = row[TAG_FLAGS];
    let mut line_widths = [0; MAX_LINES];
    line_widths[..usize::from(line_count)]
        .copy_from_slice(&row[TAG_LINE_WIDTH_0..TAG_LINE_WIDTH_0 + usize::from(line_count)]);
    Some(BlockInput {
        display: flags & FLAG_DISPLAY != 0,
        line_widths,
        line_count,
        name_line,
        align,
        name_x: row[TAG_NAME_X],
        name_width: row[TAG_NAME_WIDTH],
        prefix_width: row[TAG_PREFIX_WIDTH],
        background: Rect::new(
            row[TAG_BG_X0],
            row[TAG_BG_Y0],
            row[TAG_BG_X1],
            row[TAG_BG_Y1],
        ),
        badge: flags & FLAG_BADGE != 0,
    })
}

pub fn write_block(input: &BlockInput, row: &mut [i32]) -> bool {
    if row.len() < TAG_INTS || usize::from(input.line_count) > MAX_LINES {
        return false;
    }
    let mut flags = row[TAG_FLAGS] & !(FLAG_DISPLAY | FLAG_BADGE);
    if input.display {
        flags |= FLAG_DISPLAY;
    }
    if input.badge {
        flags |= FLAG_BADGE;
    }
    row[TAG_FLAGS] = flags;
    row[TAG_LINE_COUNT] = i32::from(input.line_count);
    row[TAG_NAME_LINE] = i32::from(input.name_line);
    row[TAG_ALIGN] = input.align.value();
    row[TAG_NAME_X] = input.name_x;
    row[TAG_NAME_WIDTH] = input.name_width;
    row[TAG_PREFIX_WIDTH] = input.prefix_width;
    row[TAG_LINE_WIDTH_0..TAG_LINE_WIDTH_0 + MAX_LINES].copy_from_slice(&input.line_widths);
    row[TAG_BG_X0] = input.background.x0;
    row[TAG_BG_Y0] = input.background.y0;
    row[TAG_BG_X1] = input.background.x1;
    row[TAG_BG_Y1] = input.background.y1;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_rows_name_every_field_once_and_blocks_round_trip() {
        let fields = [
            TAG_ASSET, TAG_VARIANT, TAG_FLAGS, TAG_DETAIL, TAG_LINE_COUNT, TAG_NAME_LINE, TAG_ALIGN, TAG_NAME_X, TAG_NAME_WIDTH, TAG_PREFIX_WIDTH,
            TAG_LINE_WIDTH_0, TAG_LINE_WIDTH_1, TAG_LINE_WIDTH_2, TAG_BG_X0, TAG_BG_Y0, TAG_BG_X1, TAG_BG_Y1, TAG_EXTERNAL_COLOR, TAG_SNEAK_ALPHA,
            TAG_GLYPH_OFFSET, TAG_GLYPH_COUNT, TAG_COLOR_OFFSET, TAG_COLOR_COUNT, TAG_RESERVED,
        ];
        assert_eq!(fields.to_vec(), (0..TAG_INTS).collect::<Vec<_>>());
        assert_eq!((SIGNAL_FLOATS, FLAGS_KNOWN, LETTER_WORDS, LETTER_OUTLINE, MAT_FIXED, TAG_HEAD), (24, 0x7f, 7, 6, MAT_UNIFORM_WORDS + 1, HEAD_EVENTS + 1));
        let (line_widths, background) = ([40, 62, 30], Rect::new(-1, -1, 62, 29));
        let input = BlockInput { display: true, line_widths, line_count: 3, name_line: 1, align: Align::Center, name_x: 20, name_width: 30, prefix_width: 20, background, badge: true };
        let mut row = [0; TAG_INTS];
        row[TAG_FLAGS] = FLAG_OWN;
        assert!(write_block(&input, &mut row));
        assert_eq!((row[TAG_FLAGS], read_block(&row)), (FLAG_OWN | FLAG_DISPLAY | FLAG_BADGE, Some(input)));
        for (field, value) in [(TAG_NAME_LINE, 3), (TAG_LINE_COUNT, 4), (TAG_LINE_COUNT, 0), (TAG_ALIGN, 9)] {
            let mut bad = row;
            bad[field] = value;
            assert_eq!(read_block(&bad), None, "{field} = {value}");
        }
        assert_eq!(read_block(&row[..TAG_INTS - 1]), None);
    }

    #[test]
    fn packed_words_round_trip() {
        assert_eq!(unpack_band_layer_kind(pack_band_layer_kind(BAND_OVER_TEXT, 31, OP_KIND_STRIP)), (3, 31, 1));
        assert_eq!(pack_band_layer(BAND_BADGE, 2), 4 | (2 << 8));
        for (band, layer, mirrored, opacity) in [(0, 0, false, 0), (3, 31, true, 255), (4, 7, false, 128)] {
            assert_eq!(unpack_material(pack_material(band, layer, mirrored, opacity)), (band, layer, mirrored, opacity));
        }
        for (scale, rotation) in [(4, 0), (12, -15), (1, 359), (3, -180)] {
            assert_eq!(unpack_scale_rotation(pack_scale_rotation(scale, rotation)), (scale, rotation));
        }
        let decoration = pack_decoration(DECORATION_OUTLINE, 0xff12_3456);
        assert_eq!((decoration >> DECORATION_KIND_SHIFT, decoration & 0x00ff_ffff), (DECORATION_OUTLINE, 0x12_3456));
    }
}
