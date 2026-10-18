//! C ABI over the core's `Scene`, for wasm and native hosts (JNA, FFM). `jvm` adds JNI for `java/`. Lengths are `u32`.
//! Returned pointers die at the next allocation that grows wasm memory, so read buffers right after `renderer_build`.

use std::alloc::{alloc as raw_alloc, dealloc as raw_dealloc, Layout};

use plusrenderer::build::{Camera, DrawRun};
use plusrenderer::mesh::MeshRun;
use plusrenderer::{bedrock, bundle, java, Affine, Arms, BodySlot, Scene};

#[cfg(not(target_arch = "wasm32"))]
mod jvm;

pub struct Renderer {
    scene: Scene,
    error: String,
}

#[no_mangle]
pub extern "C" fn alloc(len: u32) -> *mut u8 {
    // Checked: on wasm32 a `u32` length can exceed `isize::MAX`.
    match Layout::from_size_align(len as usize, 1) {
        // SAFETY: the layout is valid and not zero-sized.
        Ok(layout) if len != 0 => unsafe { raw_alloc(layout) },
        _ => std::ptr::null_mut(),
    }
}

/// # Safety
/// `ptr` and `len` must come from a matching `alloc`.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: u32) {
    if !ptr.is_null() && len != 0 {
        // SAFETY: `alloc` returned non-null, so this length's layout is valid.
        raw_dealloc(ptr, Layout::from_size_align_unchecked(len as usize, 1));
    }
}

#[no_mangle]
pub extern "C" fn renderer_new() -> *mut Renderer {
    Box::into_raw(Box::new(Renderer {
        scene: Scene::default(),
        error: String::new(),
    }))
}

/// # Safety
/// `r` must come from `renderer_new` and must not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn renderer_free(r: *mut Renderer) {
    drop(Box::from_raw(r));
}

/// Loads a cosmetic's first geometry. `anchor` may be empty, image size 0 if unknown. -1 on error.
/// # Safety
/// Every pointer/length pair must describe UTF-8 bytes valid for this call.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn renderer_load_cosmetic(
    r: *mut Renderer,
    slot: u32,
    ptr: *const u8,
    len: u32,
    body_ptr: *const u8,
    body_len: u32,
    anchor_ptr: *const u8,
    anchor_len: u32,
    image_width: u32,
    image_height: u32,
) -> i32 {
    let r = &mut *r;
    let anchor = str_from(anchor_ptr, anchor_len).filter(|a| !a.is_empty());
    let Some(body) = str_from(body_ptr, body_len) else {
        return fail(r, "body slot is not utf-8");
    };
    let Some(body_slot) = BodySlot::from_name(body) else {
        return fail(r, &format!("unknown body slot `{body}`"));
    };
    let Some(json) = str_from(ptr, len) else {
        return fail(r, "geometry is not utf-8");
    };
    let image = (image_width > 0 && image_height > 0).then_some([image_width, image_height]);

    let loaded = guarded(Err("the core failed on this geometry".to_string()), || {
        let model = bedrock::parse_one(json).map_err(|e| e.to_string())?;
        r.scene
            .load_cosmetic(slot as usize, model, body_slot, anchor, image);
        Ok(())
    });

    match loaded {
        Ok(()) => 0,
        Err(e) => fail(r, &e),
    }
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_load_player(r: *mut Renderer, slot: u32, slim: u32) {
    let arms = if slim == 0 { Arms::Wide } else { Arms::Slim };
    (*r).scene.load_player(slot as usize, arms);
}

/// Loads vanilla's cape and returns the frame count: above 1 bind the sheet, else the still.
/// # Safety
/// `name_ptr`/`name_len` must describe UTF-8 bytes valid for this call.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn renderer_load_cape(
    r: *mut Renderer,
    slot: u32,
    still_width: u32,
    still_height: u32,
    sheet_width: u32,
    sheet_height: u32,
    name_ptr: *const u8,
    name_len: u32,
) -> i32 {
    let name = str_from(name_ptr, name_len).unwrap_or("");
    let (frames, seconds) = if sheet_width > 0 {
        bundle::cape_sheet(
            [still_width, still_height],
            [sheet_width, sheet_height],
            name,
        )
    } else {
        (1, 0.1)
    };

    (*r).scene.load_cape(slot as usize, frames, seconds);
    frames as i32
}

/// Index into newline-joined `names` by `kind`: 0 geometry, 1 animation, 2 texture, 3 sheet, or -1.
/// # Safety
/// `ptr`/`len` must describe UTF-8 bytes valid for this call.
#[no_mangle]
pub unsafe extern "C" fn bundle_part(ptr: *const u8, len: u32, kind: u32) -> i32 {
    let names: Vec<&str> = str_from(ptr, len).unwrap_or("").split('\n').collect();
    let parts = bundle::parts(&names);
    let pick = match kind {
        0 => parts.geometry,
        1 => parts.animation,
        2 => parts.texture,
        _ => parts.sheet,
    };
    pick.map_or(-1, |i| i as i32)
}

/// Returns 0, or -1 on error.
/// # Safety
/// `ptr`/`len` must describe UTF-8 bytes that stay valid for this call.
#[no_mangle]
pub unsafe extern "C" fn renderer_load_animation(
    r: *mut Renderer,
    slot: u32,
    ptr: *const u8,
    len: u32,
) -> i32 {
    let r = &mut *r;
    let Some(json) = str_from(ptr, len) else {
        return fail(r, "animation is not utf-8");
    };

    let loaded = guarded(Err("the core failed on this animation".to_string()), || {
        r.scene
            .load_animation(slot as usize, json)
            .map_err(|e| e.to_string())
    });

    match loaded {
        Ok(()) => 0,
        Err(e) => fail(r, &e),
    }
}

/// Starts the slot's animation from its first frame at scene time `at`.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_play(r: *mut Renderer, slot: u32, at: f32) {
    (*r).scene.play(slot as usize, at);
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_stop(r: *mut Renderer, slot: u32) {
    (*r).scene.stop(slot as usize);
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_set_visible(r: *mut Renderer, slot: u32, visible: u32) {
    (*r).scene.set_visible(slot as usize, visible != 0);
}

/// Scales the slot's model about the bone it hangs from.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_set_scale(r: *mut Renderer, slot: u32, scale: f32) {
    (*r).scene.set_scale(slot as usize, scale);
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_clear(r: *mut Renderer, slot: u32) {
    (*r).scene.clear(slot as usize);
}

/// Minecraft light level 0..=15, where 15 (the default) is daylight.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_set_light_level(r: *mut Renderer, level: f32) {
    (*r).scene.light.level = level;
}

/// Poses every slot at `seconds`. Call before `renderer_build_mesh`. -1 if the core failed.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_pose(r: *mut Renderer, seconds: f32) -> i32 {
    let r = &mut *r;
    guarded(-1, || {
        r.scene.pose(seconds);
        0
    })
}

/// Sets a posed vanilla part: core name, rest and current pivot (pixels), rotation (radians), scale.
/// # Safety
/// `name_ptr`/`name_len` must describe UTF-8 bytes valid for this call.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn renderer_set_game_part(
    r: *mut Renderer,
    name_ptr: *const u8,
    name_len: u32,
    rest_x: f32,
    rest_y: f32,
    rest_z: f32,
    x: f32,
    y: f32,
    z: f32,
    x_rot: f32,
    y_rot: f32,
    z_rot: f32,
    x_scale: f32,
    y_scale: f32,
    z_scale: f32,
) {
    let part = java::Part {
        offset: [x, y, z],
        rotation: [x_rot, y_rot, z_rot],
        scale: [x_scale, y_scale, z_scale],
    };
    let name = str_from(name_ptr, name_len).unwrap_or("");
    (*r).scene
        .set_game_part(name, [rest_x, rest_y, rest_z], &part);
}

/// Answers a MoLang query for every slot.
/// # Safety
/// `name_ptr`/`name_len` must describe UTF-8 bytes valid for this call.
#[no_mangle]
pub unsafe extern "C" fn renderer_set_query(
    r: *mut Renderer,
    name_ptr: *const u8,
    name_len: u32,
    value: f32,
) {
    let name = str_from(name_ptr, name_len).unwrap_or("");
    (*r).scene.set_query(name, value);
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_clear_game_parts(r: *mut Renderer) {
    (*r).scene.clear_game_parts();
}

/// Writes the emote's part delta (offset, rotation in radians, scale factor) to `out`. -1 if no such bone.
/// # Safety
/// `name_ptr`/`name_len` must be UTF-8 valid for this call. `out` must hold nine writable floats.
#[no_mangle]
pub unsafe extern "C" fn renderer_part_delta(
    r: *const Renderer,
    slot: u32,
    name_ptr: *const u8,
    name_len: u32,
    out: *mut f32,
) -> i32 {
    let r = &*r;
    let name = str_from(name_ptr, name_len).unwrap_or("");
    let Some(pose) = r.scene.bone_pose(slot as usize, name) else {
        return -1;
    };

    let d = java::part_delta(&pose);
    let values = [d.offset, d.rotation, d.scale];
    out.copy_from_nonoverlapping(values.as_ptr().cast(), 9);
    0
}

/// Builds the 3D mesh from the last pose in `space` (1 is `java::MODEL`). Quad count, or -1.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_build_mesh(r: *mut Renderer, space: u32) -> i32 {
    let r = &mut *r;
    let to_host = if space == 1 {
        java::MODEL
    } else {
        Affine::IDENTITY
    };
    guarded(-1, || (r.scene.build_mesh(&to_host).glow.len() / 4) as i32)
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_mesh_run_count(r: *const Renderer) -> u32 {
    let r = &*r;
    r.scene.mesh.runs.len() as u32
}

/// Poses at `seconds` and fills the 2D buffers. Returns the index count, or -1.
/// # Safety
/// `r` must come from `renderer_new`.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn renderer_build(
    r: *mut Renderer,
    yaw: f32,
    pitch: f32,
    distance: f32,
    target_y: f32,
    width: f32,
    height: f32,
    seconds: f32,
) -> i32 {
    let camera = Camera::orbit(yaw, pitch, distance, [0.0, target_y, 0.0]);
    let r = &mut *r;
    guarded(-1, || {
        r.scene.pose(seconds);
        r.scene.build(&camera, [width, height]).indices.len() as i32
    })
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_vertex_count(r: *const Renderer) -> u32 {
    let r = &*r;
    (r.scene.buffers.positions.len() / 2) as u32
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_run_count(r: *const Renderer) -> u32 {
    let r = &*r;
    r.scene.buffers.runs.len() as u32
}

macro_rules! field_ptr {
    ($name:ident, $src:ident . $field:ident, $ty:ty) => {
        /// # Safety
        /// Valid only until the next call that allocates.
        #[no_mangle]
        pub unsafe extern "C" fn $name(r: *const Renderer) -> *const $ty {
            let r = &*r;
            r.scene.$src.$field.as_ptr()
        }
    };
}

field_ptr!(renderer_positions, buffers.positions, f32);
field_ptr!(renderer_uvs, buffers.uvs, f32);
field_ptr!(renderer_colors, buffers.colors, u32);
field_ptr!(renderer_indices, buffers.indices, u16);
// Three u32 per run: texture (the slot), first index, index count.
field_ptr!(renderer_runs, buffers.runs, DrawRun);

// Four vertices per quad: xyz, xyz normal, uv, glow level.
field_ptr!(renderer_mesh_positions, mesh.positions, f32);
field_ptr!(renderer_mesh_normals, mesh.normals, f32);
field_ptr!(renderer_mesh_uvs, mesh.uvs, f32);
field_ptr!(renderer_mesh_glow, mesh.glow, u8);
// Four u32 per run: texture (the slot), first quad, quad count, cull.
field_ptr!(renderer_mesh_runs, mesh.runs, MeshRun);

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_error(r: *const Renderer) -> *const u8 {
    let r = &*r;
    r.error.as_ptr()
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_error_len(r: *const Renderer) -> u32 {
    let r = &*r;
    r.error.len() as u32
}

unsafe fn str_from<'a>(ptr: *const u8, len: u32) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len as usize)).ok()
}

/// Turns a core panic into `failed` so it can't take the host down, but wasm aborts regardless.
fn guarded<T>(failed: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(failed)
}

fn fail(r: &mut Renderer, message: &str) -> i32 {
    r.error.clear();
    r.error.push_str(message);
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: &str = "";

    fn n(s: &str) -> u32 {
        s.len() as u32
    }

    const GLASSES: &str = r#"{"format_version":"1.12.0","minecraft:geometry":[{
        "description":{"identifier":"geometry.glasses","texture_width":16,"texture_height":16},
        "bones":[{"name":"bb_main","pivot":[0,0,0],
          "cubes":[{"origin":[-4,27,-5],"size":[8,2,1],"uv":[0,0]}]}]}]}"#;

    #[test]
    fn a_cosmetic_round_trips_through_the_abi() {
        unsafe {
            let r = renderer_new();
            renderer_load_player(r, 0, 0);
            let body = "glasses";
            let code = renderer_load_cosmetic(
                r,
                1,
                GLASSES.as_ptr(),
                n(GLASSES),
                body.as_ptr(),
                n(body),
                NONE.as_ptr(),
                0,
                32,
                32,
            );
            assert_eq!(code, 0);
            assert!(renderer_build(r, 0.0, 10.0, 62.0, 18.0, 512.0, 512.0, 0.0) > 0);
            let runs = std::slice::from_raw_parts(
                renderer_runs(r) as *const u32,
                renderer_run_count(r) as usize * 3,
            );
            assert!(
                runs.chunks(3).any(|run| run[0] == 1),
                "slot 1 draws with texture 1"
            );
            renderer_free(r);
        }
    }

    #[test]
    fn errors_name_the_problem() {
        unsafe {
            let r = renderer_new();
            let (bad, tail, hat) = ("{", "tail", "hat");
            assert_eq!(
                renderer_load_cosmetic(
                    r,
                    3,
                    bad.as_ptr(),
                    n(bad),
                    hat.as_ptr(),
                    n(hat),
                    NONE.as_ptr(),
                    0,
                    0,
                    0
                ),
                -1
            );
            assert!((&*r).error.contains("malformed geometry"));
            assert_eq!(
                renderer_load_cosmetic(
                    r,
                    1,
                    GLASSES.as_ptr(),
                    n(GLASSES),
                    tail.as_ptr(),
                    n(tail),
                    NONE.as_ptr(),
                    0,
                    0,
                    0
                ),
                -1
            );
            assert!((&*r).error.contains("unknown body slot"));
            renderer_free(r);
        }
    }

    #[test]
    fn a_game_host_round_trips_parts_and_builds_a_mesh() {
        let nod = r#"{"animations":{"nod":{"bones":{"head":{"rotation":[30,0,0]}}}}}"#;
        let (head, hat) = ("head", "glasses");
        unsafe {
            let r = renderer_new();
            renderer_load_player(r, 0, 0);
            renderer_set_visible(r, 0, 0);
            renderer_load_animation(r, 0, nod.as_ptr(), n(nod));
            renderer_play(r, 0, 0.0);
            renderer_load_cosmetic(
                r,
                1,
                GLASSES.as_ptr(),
                n(GLASSES),
                hat.as_ptr(),
                n(hat),
                NONE.as_ptr(),
                0,
                16,
                16,
            );
            renderer_pose(r, 0.0);

            let mut d = [0.0f32; 9];
            assert_eq!(
                renderer_part_delta(r, 0, head.as_ptr(), n(head), d.as_mut_ptr()),
                0
            );
            assert!(
                (d[3] - 30f32.to_radians()).abs() < 1e-6,
                "x rotation in radians: {d:?}"
            );
            renderer_set_game_part(
                r,
                head.as_ptr(),
                n(head),
                0.0,
                0.0,
                0.0,
                d[0],
                d[1],
                d[2],
                d[3],
                d[4],
                d[5],
                d[6],
                d[7],
                d[8],
            );

            let quads = renderer_build_mesh(r, 1);
            assert_eq!(
                quads, 6,
                "only the glasses: the hidden player is the game's to draw"
            );
            let runs = std::slice::from_raw_parts(
                renderer_mesh_runs(r) as *const u32,
                renderer_mesh_run_count(r) as usize * 4,
            );
            assert_eq!(runs, [1, 0, 6, 0]);
            renderer_free(r);
        }
    }

    #[test]
    fn bundle_parts_cross_as_newline_joined_names() {
        let names = "wings.png\nwings.geo.json";
        unsafe {
            assert_eq!(bundle_part(names.as_ptr(), n(names), 0), 1);
            assert_eq!(bundle_part(names.as_ptr(), n(names), 2), 0);
            assert_eq!(bundle_part(names.as_ptr(), n(names), 3), -1);
        }
    }
}
