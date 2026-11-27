use super::glsl::reads_screen;
use super::ir::{
    MaterialFrame, MaterialIr, LANE_COUNT, TONES_PER_SLOT, UNIFORM_AREA, UNIFORM_ATLAS,
    UNIFORM_BODY, UNIFORM_CLOCK, UNIFORM_LANES, UNIFORM_TONES, WORDS_PER_TONE,
};

pub fn write(ir: &MaterialIr, frame: &MaterialFrame<'_>, out: &mut [i32]) -> usize {
    let words = ir.uniform_words() as usize;
    let Some(out) = out.get_mut(..words) else {
        return 0;
    };
    out.fill(0);
    let (area, body) = (frame.area, frame.body);
    floats(
        out,
        UNIFORM_AREA,
        [
            area.x0 - body.x0,
            area.y0 - body.y0,
            area.x1 - body.x0,
            area.y1 - body.y0,
        ]
        .map(|v| v as f32),
    );
    floats(
        out,
        UNIFORM_BODY,
        [body.x0, body.y0, body.x1, body.y1].map(|v| v as f32),
    );
    let clock = UNIFORM_CLOCK as usize;
    out[clock..clock + 4].copy_from_slice(&frame.clock);
    let lanes = ir.lanes.len().min(LANE_COUNT);
    for (i, value) in frame.lanes[..lanes].iter().enumerate() {
        out[UNIFORM_LANES as usize + i] = bits(*value as f32);
    }
    for (k, rect) in frame.atlas.iter().enumerate() {
        floats(out, UNIFORM_ATLAS + k as u32 * 4, *rect);
    }
    for (s, slot) in ir.tone_slots.iter().enumerate() {
        for tone in 0..TONES_PER_SLOT {
            let index = u32::from(*slot) * TONES_PER_SLOT + tone;
            let argb = frame.tones.get(index as usize).copied().unwrap_or(0);
            let at = UNIFORM_TONES + (s as u32 * TONES_PER_SLOT + tone) * WORDS_PER_TONE;
            floats(out, at, [16, 8, 0, 24].map(|shift| channel(argb, shift)));
        }
    }
    words
}

pub const SKSL_COLOR_WORDS: u32 = 4;
pub const SKSL_SCREEN_WORDS: u32 = 9;

pub fn sksl_words(ir: &MaterialIr) -> u32 {
    let screen = if reads_screen(ir) {
        SKSL_SCREEN_WORDS
    } else {
        0
    };
    ir.uniform_words() + SKSL_COLOR_WORDS + screen
}

pub fn write_sksl(
    ir: &MaterialIr,
    frame: &MaterialFrame<'_>,
    color: [f32; 4],
    screen: [f32; 9],
    out: &mut [i32],
) -> usize {
    let words = sksl_words(ir) as usize;
    if out.len() < words {
        return 0;
    }
    let at = write(ir, frame, out);
    for (word, value) in out[at..at + 4].iter_mut().zip(color) {
        *word = bits(value);
    }
    if reads_screen(ir) {
        let at = at + SKSL_COLOR_WORDS as usize;
        for (word, value) in out[at..at + 9].iter_mut().zip(screen) {
            *word = bits(value);
        }
    }
    words
}

fn bits(value: f32) -> i32 {
    value.to_bits() as i32
}

fn channel(argb: u32, shift: u32) -> f32 {
    ((argb >> shift) & 0xff) as f32 / 255.0
}

fn floats(out: &mut [i32], at: u32, values: [f32; 4]) {
    let at = at as usize;
    for (word, value) in out[at..at + 4].iter_mut().zip(values) {
        *word = bits(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::geom::Rect;
    use crate::nameplate::material::ir::{
        Lane, Op, OpKind, Reg, Ty, LANE_COUNT, UNIFORM_AREA, UNIFORM_ATLAS, UNIFORM_BODY,
        UNIFORM_CLOCK, UNIFORM_LANES, UNIFORM_TONES,
    };
    use crate::nameplate::types::PALETTE_TONES;

    fn floats(words: &[i32]) -> Vec<f32> {
        words.iter().map(|w| f32::from_bits(*w as u32)).collect()
    }

    fn ir(slots: &[u8], lanes: u16) -> MaterialIr {
        MaterialIr {
            margins: [8, 5, 8, 5],
            tone_slots: slots.to_vec(),
            lanes: (0..lanes).map(Lane::Param).collect(),
            ..MaterialIr::default()
        }
    }

    #[test]
    fn the_block_follows_the_std140_word_table() {
        assert_eq!(
            [
                UNIFORM_AREA,
                UNIFORM_BODY,
                UNIFORM_CLOCK,
                UNIFORM_LANES,
                UNIFORM_ATLAS,
                UNIFORM_TONES
            ],
            [0, 4, 8, 12, 28, 44]
        );
        let tones: Vec<u32> = (0..PALETTE_TONES as u32)
            .map(|i| 0x8000_0000 | (i & 0xff) << 16 | (255 - (i & 0xff)) << 8 | (i >> 4))
            .collect();
        let mut lanes = [0.0; LANE_COUNT];
        lanes
            .iter_mut()
            .enumerate()
            .for_each(|(i, lane)| *lane = i as f64 + 0.25);
        let atlas = [
            [1.0, 2.0, 16.0, 16.0],
            [3.0, 4.0, 8.0, 8.0],
            [0.0; 4],
            [5.0, 6.0, 7.0, 8.0],
        ];
        let frame = MaterialFrame {
            area: Rect::new(-5, -7, 51, 11),
            body: Rect::new(3, -2, 43, 6),
            clock: [123, 600, 20, 2],
            lanes,
            tones: &tones,
            atlas,
        };
        let mut out = [7; 60];
        assert_eq!(write(&ir(&[], 3), &frame, &mut out), 44);
        let actual = &floats(&out[0..8]);
        assert_eq!(*actual, [-8.0, -5.0, 48.0, 13.0, 3.0, -2.0, 43.0, 6.0]);
        assert_eq!(&out[8..12], &[123, 600, 20, 2]);
        assert_eq!(floats(&out[12..16]), [0.25, 1.25, 2.25, 0.0]);
        assert!(floats(&out[16..28]).iter().all(|v| *v == 0.0));
        assert_eq!(
            floats(&out[28..44]),
            atlas.concat().iter().map(|v| *v as f32).collect::<Vec<_>>()
        );
        assert_eq!(&out[44..], &[7; 16]);

        let toned = ir(&[2, 5], 16);
        assert_eq!(toned.uniform_words(), 44 + 2 * 16 * 4);
        let mut out = vec![5; toned.uniform_words() as usize];
        assert_eq!(write(&toned, &frame, &mut out), out.len());
        for (s, slot) in [2u32, 5].into_iter().enumerate() {
            for tone in 0..16u32 {
                let argb = tones[(slot * 16 + tone) as usize];
                let at = 44 + (s * 16 + tone as usize) * 4;
                let expected = [16, 8, 0, 24].map(|shift| ((argb >> shift) & 0xff) as f32 / 255.0);
                assert_eq!(
                    floats(&out[at..at + 4]),
                    expected,
                    "slot {slot} tone {tone}"
                );
            }
        }
        let mut short = vec![9; 100];
        assert_eq!(write(&ir(&[0], 0), &frame, &mut short), 0);
        assert!(short.iter().all(|w| *w == 9));
    }

    #[test]
    fn sksl_buffers_append_colour_and_screen_after_the_block() {
        let tones = [0u32; PALETTE_TONES];
        let (area, body) = (Rect::new(-8, -5, 48, 13), Rect::new(0, 0, 40, 8));
        let frame = MaterialFrame {
            area,
            body,
            clock: [1, 12, 12, 1],
            lanes: [0.0; LANE_COUNT],
            tones: &tones,
            atlas: [[0.0; 4]; 4],
        };
        let (color, screen) = (
            [1.0, 0.5, 0.25, 0.75],
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
        );
        let plain = ir(&[2], 2);
        assert_eq!(sksl_words(&plain), plain.uniform_words() + 4);
        let mut block = vec![0; plain.uniform_words() as usize];
        write(&plain, &frame, &mut block);
        let mut out = vec![-1; 200];
        assert_eq!(write_sksl(&plain, &frame, color, screen, &mut out), 112);
        let actual = &(&out[..108], floats(&out[108..112]));
        assert_eq!(*actual, (&block[..], color.to_vec()));
        assert!(out[112..].iter().all(|w| *w == -1));
        let mut screened = ir(&[], 0);
        screened.ops.push(Op {
            kind: OpKind::ScreenPos,
            ty: Ty::Vec2,
            dst: Reg(0),
            args: [Reg::NONE; 4],
            imm: [0.0; 4],
        });
        assert_eq!(sksl_words(&screened), 44 + 4 + 9);
        let mut out = vec![0; 57];
        assert_eq!(write_sksl(&screened, &frame, color, screen, &mut out), 57);
        assert_eq!(floats(&out[48..]), screen);
        let actual = &write_sksl(&screened, &frame, color, screen, &mut out[..56]);
        assert_eq!(*actual, 0);
    }
}
