use crate::limits::{NAMEPLATE_IMAGE_OVERHEAD_BYTES, NAMEPLATE_RECT_BYTES, NAMEPLATE_STRIP_BYTES};

use super::geom::{LayoutResult, Placement, Rect};
use super::types::NameplateAsset;

pub const REF_KIND_SHIFT: u32 = 28;
pub const REF_SLOT: u32 = 0;
pub const REF_FIXED: u32 = 1;
pub const REF_BLEND: u32 = 2;
pub const REF_PAYLOAD: u32 = (1 << REF_KIND_SHIFT) - 1;
pub const TONES_PER_SLOT: u32 = 16;
pub const BLEND_ROWS_BITS: u32 = 11;
pub const BLEND_ROW_BITS: u32 = 10;
pub const BLEND_SLOT_SHIFT: u32 = BLEND_ROWS_BITS + BLEND_ROW_BITS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    Slot,
    Fixed,
    Blend,
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColorRef(pub u32);

impl ColorRef {
    pub const fn slot(slot: u8, tone: u8) -> ColorRef {
        ColorRef((REF_SLOT << REF_KIND_SHIFT) | (slot as u32 * TONES_PER_SLOT + tone as u32))
    }

    pub const fn fixed(index: u32) -> ColorRef {
        ColorRef((REF_FIXED << REF_KIND_SHIFT) | (index & REF_PAYLOAD))
    }

    pub const fn blend(slot: u8, row: u16, rows: u16) -> ColorRef {
        ColorRef(
            (REF_BLEND << REF_KIND_SHIFT)
                | ((slot as u32) << BLEND_SLOT_SHIFT)
                | ((row as u32) << BLEND_ROWS_BITS)
                | rows as u32,
        )
    }

    pub const fn kind(self) -> RefKind {
        match self.0 >> REF_KIND_SHIFT {
            REF_SLOT => RefKind::Slot,
            REF_FIXED => RefKind::Fixed,
            _ => RefKind::Blend,
        }
    }

    pub const fn payload(self) -> u32 {
        self.0 & REF_PAYLOAD
    }

    pub const fn tone_ref(self) -> u32 {
        self.payload()
    }

    pub const fn slot_index(self) -> u8 {
        match self.kind() {
            RefKind::Slot => (self.payload() / TONES_PER_SLOT) as u8,
            RefKind::Blend => ((self.payload() >> BLEND_SLOT_SHIFT) & 0x1f) as u8,
            RefKind::Fixed => 0,
        }
    }

    pub const fn tone(self) -> u8 {
        (self.payload() % TONES_PER_SLOT) as u8
    }

    pub const fn fixed_index(self) -> u32 {
        self.payload()
    }

    pub const fn blend_row(self) -> u16 {
        ((self.payload() >> BLEND_ROWS_BITS) & ((1 << BLEND_ROW_BITS) - 1)) as u16
    }

    pub const fn blend_rows(self) -> u16 {
        (self.payload() & ((1 << BLEND_ROWS_BITS) - 1)) as u16
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Axis {
    #[default]
    X,
    Y,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PaintRect {
    pub x0: i16,
    pub y0: i16,
    pub x1: i16,
    pub y1: i16,
    pub color: ColorRef,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StripRun {
    pub y0: i16,
    pub y1: i16,
    pub x0: i16,
    pub x1: i16,
    pub slot: u8,
    pub axis: Axis,
    pub phase: i16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerImage {
    pub rects: Vec<PaintRect>,
    pub strips: Vec<StripRun>,
    pub bounds: Rect,
    pub bytes: u32,
}

impl LayerImage {
    pub fn new(rects: Vec<PaintRect>, strips: Vec<StripRun>) -> LayerImage {
        let bounds = rects
            .iter()
            .map(|r| {
                Rect::new(
                    i32::from(r.x0),
                    i32::from(r.y0),
                    i32::from(r.x1),
                    i32::from(r.y1),
                )
            })
            .chain(strips.iter().map(|s| {
                Rect::new(
                    i32::from(s.x0),
                    i32::from(s.y0),
                    i32::from(s.x1),
                    i32::from(s.y1),
                )
            }))
            .fold(Rect::EMPTY, |all, r| all.union(r));
        let bytes = image_bytes(rects.len(), strips.len());
        LayerImage {
            rects,
            strips,
            bounds,
            bytes,
        }
    }

    pub fn quads(&self) -> u32 {
        (self.rects.len() + self.strips.len()) as u32
    }
}

pub fn image_bytes(rects: usize, strips: usize) -> u32 {
    let rects = u32::try_from(rects).unwrap_or(u32::MAX);
    let strips = u32::try_from(strips).unwrap_or(u32::MAX);
    NAMEPLATE_IMAGE_OVERHEAD_BYTES
        .saturating_add(rects.saturating_mul(NAMEPLATE_RECT_BYTES))
        .saturating_add(strips.saturating_mul(NAMEPLATE_STRIP_BYTES))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageKey {
    pub asset: u32,
    pub layer: u8,
    pub state: u16,
    pub inputs: u64,
}

pub trait Painter {
    fn paint(
        &self,
        a: &NameplateAsset,
        layer: u8,
        state: u16,
        p: &Placement,
        g: &LayoutResult,
    ) -> LayerImage;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_refs_pack_their_kind_and_images_count_their_bytes() {
        let (slot, blend) = (ColorRef::slot(3, 7), ColorRef::blend(31, 1023, 1024));
        assert_eq!((slot.0, slot.kind(), slot.slot_index(), slot.tone()), (55, RefKind::Slot, 3, 7));
        assert_eq!((ColorRef::fixed(1234).kind(), ColorRef::fixed(1234).fixed_index()), (RefKind::Fixed, 1234));
        assert_eq!((blend.kind(), blend.slot_index(), blend.blend_row(), blend.blend_rows()), (RefKind::Blend, 31, 1023, 1024));
        let rect = PaintRect { x0: -2, y0: 0, x1: 4, y1: 1, color: slot };
        let strip = StripRun { y0: 3, y1: 5, x0: 1, x1: 12, slot: 2, axis: Axis::X, phase: 0 };
        let image = LayerImage::new(vec![rect, rect], vec![strip]);
        assert_eq!((image.bounds, image.bytes, image.quads()), (Rect::new(-2, 0, 12, 5), image_bytes(2, 1), 3));
        assert_eq!(image_bytes(10, 2), 64 + 200 + 48);
    }
}
