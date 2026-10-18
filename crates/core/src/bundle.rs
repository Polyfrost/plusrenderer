//! Picks which files of an uploaded bundle mean what, by name, as PolyPlus `DiskAssetReader` does.

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parts {
    pub geometry: Option<usize>,
    /// A cosmetic's animation, or an emote when there is no geometry.
    pub animation: Option<usize>,
    pub texture: Option<usize>,
    pub sheet: Option<usize>,
}

/// First of each kind in path order, skipping macOS `._` forks. A `textures/` PNG wins.
pub fn parts(names: &[&str]) -> Parts {
    let mut order: Vec<usize> = (0..names.len())
        .filter(|&i| !file_name(names[i]).starts_with("._"))
        .collect();
    order.sort_by_key(|&i| names[i]);

    let lower: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    let first = |test: &dyn Fn(&str) -> bool| order.iter().copied().find(|&i| test(&lower[i]));

    Parts {
        geometry: first(&|n| n.ends_with(".geo.json") && !n.ends_with("player.geo.json")),
        animation: first(&|n| {
            n.ends_with(".json")
                && !n.ends_with(".geo.json")
                && !n.ends_with(".emote.json")
                && !n.ends_with(".animation_controllers.json")
                && file_name(n) != "pet.json"
        }),
        texture: first(&|n| n.starts_with("textures/") && n.ends_with(".png"))
            .or_else(|| first(&|n| n.ends_with(".png"))),
        sheet: first(&|n| n.ends_with(".sheet")),
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Count of `frame`-sized frames stacked in `image`, at least `min`, else 1. The client's sheet rule.
pub(crate) fn vertical_frames(frame: [u32; 2], image: [u32; 2], min: u32) -> u32 {
    let [w, h] = frame;
    if h > 0 && image[0] == w && image[1].is_multiple_of(h) && image[1] / h >= min {
        image[1] / h
    } else {
        1
    }
}

/// Frame count and seconds per frame. The pace may be named as `cape.120ms.sheet` (50..=5000 ms).
pub fn cape_sheet(still: [u32; 2], sheet: [u32; 2], sheet_name: &str) -> (u32, f32) {
    let frames = vertical_frames(still, sheet, 2);
    let ms = sheet_name
        .to_ascii_lowercase()
        .strip_suffix("ms.sheet")
        .and_then(|rest| rest.rsplit('.').next()?.parse::<u32>().ok())
        .filter(|ms| (50..=5000).contains(ms))
        .unwrap_or(100);

    (frames, ms as f32 / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_follow_the_clients_picks() {
        let names = [
            "__MACOSX/._wings.geo.json",
            "wings.geo.json",
            "player.geo.json",
            "wings.emote.json",
            "wings.animation.json",
            "wings.png",
            "textures/wings_alt.png",
            "cape.sheet",
        ];
        let p = parts(&names);
        assert_eq!(p.geometry, Some(1));
        assert_eq!(p.animation, Some(4));
        assert_eq!(p.texture, Some(6), "textures/ wins");
        assert_eq!(p.sheet, Some(7));
        assert_eq!(parts(&["a.png", "b.png"][..]).texture, Some(0));
    }

    #[test]
    fn cape_sheets_stack_still_sized_frames() {
        assert_eq!(cape_sheet([128, 64], [128, 1024], "cape.sheet"), (16, 0.1));
        assert_eq!(
            cape_sheet([128, 64], [128, 1024], "cape.40ms.sheet").1,
            0.1,
            "too fast"
        );
        assert_eq!(
            cape_sheet([128, 64], [128, 1024], "Cape.250MS.sheet").1,
            0.25
        );
        assert_eq!(
            cape_sheet([128, 64], [128, 100], "cape.sheet").0,
            1,
            "not whole frames"
        );
        assert_eq!(
            cape_sheet([128, 64], [64, 1024], "cape.sheet").0,
            1,
            "wrong width"
        );
    }
}
