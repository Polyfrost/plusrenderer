use super::image::{Axis, ColorRef};
use super::types::{Flag, OverlayKind};

pub const CIRCLE_MIN_ROWS: u16 = 4;

pub fn fit(weights: &[u16], rows: u16) -> Option<Vec<u16>> {
    let bands = weights.len();
    let total: u64 = weights.iter().map(|w| u64::from(*w)).sum();
    if bands == 0 || usize::from(rows) < bands || total == 0 {
        return None;
    }
    let mut fitted = Vec::with_capacity(bands);
    let mut remainders = Vec::with_capacity(bands);
    for weight in weights {
        let quota = u64::from(*weight) * u64::from(rows);
        fitted.push((quota / total) as u16);
        remainders.push(Some(quota % total));
    }
    let assigned: u16 = fitted.iter().sum();
    for _ in assigned..rows {
        let mut best = 0;
        for i in 1..bands {
            if remainders[i] > remainders[best] {
                best = i;
            }
        }
        fitted[best] += 1;
        remainders[best] = None;
    }
    for i in 0..bands {
        if fitted[i] == 0 {
            let mut donor = 0;
            for j in 1..bands {
                if fitted[j] > fitted[donor] {
                    donor = j;
                }
            }
            fitted[donor] -= 1;
            fitted[i] = 1;
        }
    }
    Some(fitted)
}

pub fn fit_symmetric(weights: &[u16], rows: u16) -> Option<Vec<u16>> {
    let bands = weights.len();
    let pairs = bands / 2;
    let odd = bands % 2 == 1;
    if bands == 0 || usize::from(rows) < bands || (!odd && rows % 2 == 1) {
        return fit(weights, rows);
    }
    if pairs == 0 {
        return Some(vec![rows]);
    }
    let total: u64 = weights.iter().map(|w| u64::from(*w)).sum();
    if total == 0 {
        return None;
    }
    let mut middle = 0_u16;
    if odd {
        let share = u64::from(weights[pairs]) * u64::from(rows) / total;
        middle = (share as u16).max(1);
        if !(rows - middle).is_multiple_of(2) {
            middle += 1;
        }
        let left = |middle: u16| usize::from(rows.saturating_sub(middle));
        while left(middle) < 2 * pairs && middle >= 3 {
            middle -= 2;
        }
        if middle > rows || left(middle) < 2 * pairs {
            return fit(weights, rows);
        }
    }
    let Some(half) = fit(&weights[..pairs], (rows - middle) / 2) else {
        return fit(weights, rows);
    };
    Some(
        (0..bands)
            .map(|i| {
                if i < pairs {
                    half[i]
                } else if odd && i == pairs {
                    middle
                } else {
                    half[bands - 1 - i]
                }
            })
            .collect(),
    )
}

fn weights(flag: &Flag) -> Vec<u16> {
    flag.bands.iter().map(|b| b.weight).collect()
}

pub fn band_rows(flag: &Flag, rows: u16) -> Option<Vec<u16>> {
    if flag.axis == Axis::Y {
        let authored = flag.rows.iter().find(|o| o.rows == rows).filter(|o| {
            o.bands.len() == flag.bands.len()
                && o.bands.iter().all(|b| *b >= 1)
                && o.bands.iter().map(|b| u32::from(*b)).sum::<u32>() == u32::from(rows)
        });
        if let Some(authored) = authored {
            return Some(authored.bands.clone());
        }
    }
    let weights = weights(flag);
    if flag.symmetric {
        fit_symmetric(&weights, rows)
    } else {
        fit(&weights, rows)
    }
}

pub fn span(flag: &Flag, width: u16, height: u16) -> u16 {
    match flag.axis {
        Axis::X => width,
        Axis::Y => height,
    }
}

pub fn fits(flag: &Flag, width: u16, height: u16) -> bool {
    let circle = flag
        .overlay
        .as_ref()
        .is_some_and(|o| o.kind == OverlayKind::Circle);
    if circle && height < CIRCLE_MIN_ROWS {
        return false;
    }
    band_rows(flag, span(flag, width, height)).is_some()
}

pub fn band_at(fitted: &[u16], row: i32) -> usize {
    let mut end = 0_i32;
    for (band, rows) in fitted.iter().enumerate() {
        end += i32::from(*rows);
        if row < end {
            return band;
        }
    }
    fitted.len().saturating_sub(1)
}

pub fn chevron_stripe(column: i32, row: i32, rows: i32, width: i32, stripes: i32) -> i32 {
    let width = width.max(1);
    let depth = row.min(rows - 1 - row);
    if column < depth + stripes * width {
        (column - depth).div_euclid(width).max(0)
    } else {
        -1
    }
}

pub fn circle_part(column: i32, row: i32, columns: i32, rows: i32, width: i32) -> i32 {
    let outer = rows - 2;
    if outer < 2 {
        return -1;
    }
    let dx = 2 * column + 1 - columns;
    let dy = 2 * row + 1 - rows;
    let distance = dx * dx + dy * dy;
    let inner = (outer - 2 * width).max(0);
    if distance > outer * outer {
        -1
    } else if distance > inner * inner {
        0
    } else {
        1
    }
}

pub fn color_at(
    flag: &Flag,
    fitted: &[u16],
    column: i32,
    row: i32,
    width: i32,
    height: i32,
) -> ColorRef {
    let along = match flag.axis {
        Axis::X => column,
        Axis::Y => row,
    };
    let band = flag
        .bands
        .get(band_at(fitted, along))
        .map_or(ColorRef::default(), |b| b.color);
    let Some(overlay) = &flag.overlay else {
        return band;
    };
    let colors = &overlay.colors;
    let stripes = colors.len() as i32;
    let width_px = i32::from(overlay.width);
    match overlay.kind {
        OverlayKind::Chevron | OverlayKind::Triangle => {
            let stripe = chevron_stripe(column, row, height, width_px, stripes);
            usize::try_from(stripe)
                .ok()
                .and_then(|s| colors.get(s))
                .copied()
                .unwrap_or(band)
        }
        OverlayKind::Circle => match circle_part(column, row, width, height, width_px) {
            0 => colors.first().copied().unwrap_or(band),
            1 => colors.get(1).copied().unwrap_or(band),
            _ => band,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nameplate::types::{Band, Overlay, RowsOverride};

    #[test]
    fn bands_fit_by_largest_remainder_mirror_when_symmetric_and_fill_every_row() {
        let cases: [(&[u16], u16, &[u16]); 6] = [(&[1; 6], 8, &[2, 2, 1, 1, 1, 1]), (&[1; 6], 13, &[3, 2, 2, 2, 2, 2]), (&[3, 1, 2], 11, &[5, 2, 4]),
            (&[1, 1, 1], 10, &[4, 3, 3]), (&[1, 1000, 1], 8, &[1, 6, 1]), (&[1000, 1000, 1, 1], 11, &[4, 5, 1, 1])];
        cases.iter().for_each(|&(weights, rows, fitted)| assert_eq!(fit(weights, rows).as_deref(), Some(fitted), "{weights:?} {rows}"));
        assert!([fit(&[1, 1, 1], 2), fit(&[], 8), fit(&[0, 0], 8), fit(&[1; 12], 11)].iter().all(Option::is_none));
        assert_eq!((fit_symmetric(&[1; 5], 12), fit_symmetric(&[1, 1000, 1], 3)), (Some(vec![3, 2, 2, 2, 3]), Some(vec![1, 1, 1])));
        for rows in 8..=40 {
            for weights in [&[1, 7, 1, 7, 1][..], &[1000, 1, 999], &[1, 2, 3, 2, 1]] {
                let (plain, mirrored) = (fit(weights, rows).unwrap(), fit_symmetric(weights, rows).unwrap());
                assert!(plain.iter().sum::<u16>() == rows && plain.iter().all(|r| *r >= 1), "{weights:?} {rows}");
                assert!(mirrored.iter().sum::<u16>() == rows && mirrored.iter().eq(mirrored.iter().rev()), "{weights:?} {rows}");
            }
        }
        let bands = (0..6).map(|i| Band { color: ColorRef::slot(i, 0), weight: 1 }).collect();
        let mut rainbow = Flag { id: "pride".into(), bands, overlay: None, rows: Vec::new(), symmetric: false, axis: Axis::Y };
        rainbow.rows.push(RowsOverride { rows: 8, bands: vec![1, 1, 1, 1, 2, 2] });
        assert_eq!((band_rows(&rainbow, 8), band_rows(&rainbow, 5)), (Some(vec![1, 1, 1, 1, 2, 2]), None));
        assert_eq!((0..7).map(|row| band_at(&[2, 3, 2], row)).collect::<Vec<_>>(), [0, 0, 1, 1, 1, 2, 2]);
    }

    #[test]
    fn overlays_paint_chevrons_triangles_and_circles_over_the_bands() {
        let bands = (0..6).map(|i| Band { color: ColorRef::slot(i, 0), weight: 1 }).collect();
        let mut flag = Flag { id: "progress".into(), bands, overlay: None, rows: Vec::new(), symmetric: false, axis: Axis::Y };
        let mut overlay = |kind, colors: &[u32], width| {
            flag.overlay = Some(Overlay { kind, colors: colors.iter().map(|&c| ColorRef::fixed(c)).collect(), width });
            let fitted = band_rows(&flag, 12).unwrap();
            [(0, 0), (2, 0), (4, 0), (8, 5), (9, 5), (7, 5)].map(|(x, y)| color_at(&flag, &fitted, x, y, 40, 12))
        };
        let (fixed, slot) = (ColorRef::fixed, |s| ColorRef::slot(s, 0));
        assert_eq!(overlay(OverlayKind::Chevron, &[0, 1], 2)[..5], [fixed(0), fixed(1), slot(0), fixed(1), slot(2)]);
        assert_eq!(overlay(OverlayKind::Triangle, &[3], 3)[3..], [slot(2), slot(2), fixed(3)]);
        assert_eq!((chevron_stripe(5, 6, 12, 2, 2), circle_part(19, 5, 30, 12, 1), circle_part(14, 5, 30, 12, 1)), (0, 0, 1));
        flag.bands.truncate(1);
        flag.overlay = Some(Overlay { kind: OverlayKind::Circle, colors: vec![fixed(7)], width: 1 });
        assert!(!fits(&flag, 30, 3) && fits(&flag, 30, 4));
        let fitted = band_rows(&flag, 12).unwrap();
        assert_eq!([(19, 5), (14, 5)].map(|(x, y)| color_at(&flag, &fitted, x, y, 30, 12)), [fixed(7), slot(0)]);
    }
}
