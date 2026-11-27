use std::sync::OnceLock;

use crate::palette::lerp;

pub const BACKDROP_GRAY: u32 = 0xff80_8080;

const SRGB_KNEE: f64 = 0.04045;
const SRGB_SLOPE: f64 = 12.92;
const GAMUT_EPSILON: f64 = 1.0e-4;
const GAMUT_STEPS: u32 = 16;

fn linear_table() -> &'static [f64; 256] {
    static TABLE: OnceLock<[f64; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0; 256];
        for (i, value) in table.iter_mut().enumerate() {
            let c = i as f64 / 255.0;
            *value = if c <= SRGB_KNEE {
                c / SRGB_SLOPE
            } else {
                libm::pow((c + 0.055) / 1.055, 2.4)
            };
        }
        table
    })
}

pub fn srgb_to_linear(channel: u8) -> f64 {
    linear_table()[usize::from(channel)]
}

pub fn linear_to_srgb(linear: f64) -> u8 {
    if linear.is_nan() || linear <= 0.0 {
        return 0;
    }
    let encoded = if linear <= SRGB_KNEE / SRGB_SLOPE {
        linear * SRGB_SLOPE
    } else {
        1.055 * libm::pow(linear, 1.0 / 2.4) - 0.055
    };
    libm::round(encoded * 255.0).clamp(0.0, 255.0) as u8
}

fn channel(argb: u32, shift: u32) -> u8 {
    (argb >> shift) as u8
}

fn linear_rgb(argb: u32) -> [f64; 3] {
    [
        srgb_to_linear(channel(argb, 16)),
        srgb_to_linear(channel(argb, 8)),
        srgb_to_linear(channel(argb, 0)),
    ]
}

pub fn relative_luminance(argb: u32) -> f64 {
    let [r, g, b] = linear_rgb(argb);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

pub fn contrast_ratio(a: u32, b: u32) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (light, dark) = if la >= lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}

pub fn flatten(argb: u32, backdrop: u32) -> u32 {
    let alpha = argb >> 24;
    let inverse = 255 - alpha;
    let mix = |shift: u32| {
        let source = (argb >> shift) & 0xff;
        let under = (backdrop >> shift) & 0xff;
        ((source * alpha + under * inverse + 127) / 255) << shift
    };
    0xff00_0000 | mix(16) | mix(8) | mix(0)
}

pub fn lerp_argb(from: u32, to: u32, num: i64, den: i64) -> u32 {
    lerp(from, to, num, den)
}

pub fn to_oklab(argb: u32) -> [f64; 3] {
    let [r, g, b] = linear_rgb(argb);
    let l = libm::cbrt(0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b);
    let m = libm::cbrt(0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b);
    let s = libm::cbrt(0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b);
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

fn oklab_linear(lab: [f64; 3]) -> [f64; 3] {
    let [lightness, a, b] = lab;
    let l = lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m = lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s = lightness - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

pub fn from_oklab(lab: [f64; 3], alpha: u8) -> u32 {
    let [r, g, b] = oklab_linear(lab);
    (u32::from(alpha) << 24)
        | (u32::from(linear_to_srgb(r)) << 16)
        | (u32::from(linear_to_srgb(g)) << 8)
        | u32::from(linear_to_srgb(b))
}

pub fn to_oklch(argb: u32) -> [f64; 3] {
    let [lightness, a, b] = to_oklab(argb);
    let chroma = libm::sqrt(a * a + b * b);
    let mut hue = libm::atan2(b, a).to_degrees();
    if hue < 0.0 {
        hue += 360.0;
    }
    if hue >= 360.0 {
        hue -= 360.0;
    }
    [lightness, chroma, hue]
}

fn in_gamut(lab: [f64; 3]) -> bool {
    oklab_linear(lab)
        .iter()
        .all(|c| (-GAMUT_EPSILON..=1.0 + GAMUT_EPSILON).contains(c))
}

pub fn from_oklch(lch: [f64; 3], alpha: u8) -> u32 {
    let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
    let lightness = finite(lch[0]).clamp(0.0, 1.0);
    let chroma = finite(lch[1]).max(0.0);
    let hue = finite(lch[2]).to_radians();
    let (sin, cos) = (libm::sin(hue), libm::cos(hue));
    let lab = |c: f64| [lightness, c * cos, c * sin];
    if in_gamut(lab(chroma)) {
        return from_oklab(lab(chroma), alpha);
    }
    let mut low = 0.0;
    let mut high = chroma;
    for _ in 0..GAMUT_STEPS {
        let mid = (low + high) * 0.5;
        if in_gamut(lab(mid)) {
            low = mid;
        } else {
            high = mid;
        }
    }
    from_oklab(lab(low), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wcag_ratios_flattening_and_lerps_match_their_references() {
        let pairs = [(0xff00_0000, 0xffff_ffff, 21.0), (0xff76_7676, 0xffff_ffff, 4.54), (0xff00_00ff, 0xffff_ffff, 8.59), (0xff80_8080, 0xff00_0000, 5.32)];
        pairs.iter().for_each(|&(a, b, ratio)| assert!((contrast_ratio(a, b) - ratio).abs() <= 0.01, "{a:08x} on {b:08x}"));
        assert_eq!(relative_luminance(0x0080_8080), relative_luminance(0xff80_8080));
        assert_eq!([flatten(0x0012_3456, BACKDROP_GRAY), flatten(0x80ff_ffff, BACKDROP_GRAY), flatten(0x40ff_0000, 0)], [0xff80_8080, 0xffc0_c0c0, 0xff40_0000]);
        assert_eq!((lerp_argb(0xff60_4020, 0xff20_1008, 1, 11), lerp_argb(0x00ff_ff00, 0xffff_0000, 1, 2)), (0xff5a_3b1d, 0x7fff_7f00));
        assert!((1..=64i64).all(|den| (0..=den).all(|num| lerp_argb(0, 0xffff_ffff, num, den) == ((255 * num / den) as u32) * 0x0101_0101)));
    }

    #[test]
    fn oklab_and_oklch_round_trip_every_sampled_color_and_clip_into_gamut() {
        let channel = (0..=255_u32).step_by(5);
        let grid = channel.clone().flat_map(|r| channel.clone().flat_map(move |g| (0..=255_u32).step_by(5).map(move |b| 0xff00_0000 | (r << 16) | (g << 8) | b)));
        for argb in grid {
            for back in [from_oklab(to_oklab(argb), 0xff), from_oklch(to_oklch(argb), 0xff)] {
                assert!([0, 8, 16].iter().all(|s| (((back >> s) & 0xff) as i32 - ((argb >> s) & 0xff) as i32).abs() <= 1), "{argb:08x} -> {back:08x}");
            }
        }
        let red = to_oklch(0xffff_0000);
        assert!((red[0] - 0.628).abs() < 1e-3 && (red[1] - 0.2577).abs() < 1e-3 && (red[2] - 29.23).abs() < 0.05, "{red:?}");
        assert!((to_oklch(from_oklch([0.7, 0.5, 30.0], 0xff))[2] - 30.0).abs() < 3.0);
        assert_eq!([from_oklch([2.0, 0.0, 0.0], 0xff), from_oklch([f64::NAN; 3], 0xff), from_oklch(to_oklch(0xff3a_7bd5), 0x40) >> 24], [0xffff_ffff, 0xff00_0000, 0x40]);
    }
}
