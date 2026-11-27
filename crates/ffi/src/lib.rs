//! C ABI over the core's `Scene`, for wasm and native hosts (JNA, FFM). `jvm` adds JNI for `java/`. Lengths are `u32`.
//! Returned pointers die at the next allocation that grows wasm memory, so read buffers right after `renderer_build`.

use std::alloc::{alloc as raw_alloc, dealloc as raw_dealloc, Layout};

use plusrenderer::build::{Camera, DrawRun};
use plusrenderer::cost::Motion;
use plusrenderer::mesh::MeshRun;
use plusrenderer::scene::{FrameShape, EFFECT_EVENT_INTS};
use plusrenderer::{bedrock, bundle, java, Affine, Arms, BodySlot, ElytraPose, Scene};

mod cosmetic;
#[cfg(not(target_arch = "wasm32"))]
mod cosmetic_jvm;
#[cfg(not(target_arch = "wasm32"))]
mod jvm;
#[cfg(not(target_arch = "wasm32"))]
mod molang_jvm;
mod nameplate;
#[cfg(not(target_arch = "wasm32"))]
mod nameplate_jvm;

pub const SLOTS: u32 = 256;

pub struct Renderer {
    scene: Scene,
    error: String,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    input: Vec<f32>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ids: Vec<i64>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    output: Vec<f32>,
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
        input: Vec::new(),
        ids: Vec::new(),
        output: Vec::new(),
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
    let Some(slot) = slot_in(r, slot) else {
        return -1;
    };
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
        r.scene.load_cosmetic(slot, model, body_slot, anchor, image);
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
    let r = &mut *r;
    let Some(slot) = slot_in(r, slot) else {
        return;
    };
    let arms = if slim == 0 { Arms::Wide } else { Arms::Slim };
    r.scene.load_player(slot, arms);
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
    let r = &mut *r;
    let Some(slot) = slot_in(r, slot) else {
        return -1;
    };
    let (frames, seconds) = sheet_frames(
        [still_width, still_height],
        [sheet_width, sheet_height],
        name_ptr,
        name_len,
    );

    r.scene.load_cape(slot, frames, seconds);
    frames as i32
}

/// As `renderer_load_cape`, but vanilla's elytra, drawn with the same cape texture or sheet.
/// # Safety
/// `name_ptr`/`name_len` must describe UTF-8 bytes valid for this call.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn renderer_load_elytra(
    r: *mut Renderer,
    slot: u32,
    still_width: u32,
    still_height: u32,
    sheet_width: u32,
    sheet_height: u32,
    name_ptr: *const u8,
    name_len: u32,
) -> i32 {
    let r = &mut *r;
    let Some(slot) = slot_in(r, slot) else {
        return -1;
    };
    let (frames, seconds) = sheet_frames(
        [still_width, still_height],
        [sheet_width, sheet_height],
        name_ptr,
        name_len,
    );

    r.scene.load_elytra(slot, frames, seconds);
    frames as i32
}

/// Frame count and seconds per frame. One still frame if there is no sheet.
unsafe fn sheet_frames(
    still: [u32; 2],
    sheet: [u32; 2],
    name_ptr: *const u8,
    name_len: u32,
) -> (u32, f32) {
    if sheet[0] == 0 {
        return (1, 0.1);
    }

    let name = str_from(name_ptr, name_len).unwrap_or("");
    bundle::cape_sheet(still, sheet, name)
}

/// `pose` 0 standing (and swimming), 1 crouching, 2 flying. `fall` is the flying dive, 0..=1.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_set_elytra_pose(
    r: *mut Renderer,
    slot: u32,
    pose: u32,
    fall: f32,
) {
    let r = &mut *r;
    let Some(slot) = slot_in(r, slot) else {
        return;
    };
    let pose = match pose {
        1 => ElytraPose::Crouching,
        2 => ElytraPose::Flying { fall },
        _ => ElytraPose::Standing,
    };

    r.scene.set_elytra_pose(slot, pose);
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
    let Some(slot) = slot_in(r, slot) else {
        return -1;
    };
    let Some(json) = str_from(ptr, len) else {
        return fail(r, "animation is not utf-8");
    };

    let loaded = guarded(Err("the core failed on this animation".to_string()), || {
        r.scene
            .load_animation(slot, json)
            .map_err(|e| e.to_string())
    });

    match loaded {
        Ok(()) => 0,
        Err(e) => fail(r, &e),
    }
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_load_controllers(
    r: *mut Renderer,
    slot: u32,
    ptr: *const u8,
    len: u32,
    names_ptr: *const u8,
    names_len: u32,
    data_ptr: *const u8,
    data_len: u32,
    sizes_ptr: *const u32,
    file_count: u32,
    declared_ptr: *const u8,
    declared_len: u32,
) -> i32 {
    let r = &mut *r;
    let Some(slot) = slot_in(r, slot) else {
        return -1;
    };
    let Some(json) = str_from(ptr, len) else {
        return fail(r, "animation is not utf-8");
    };
    let names: Vec<&str> = match (file_count, str_from(names_ptr, names_len)) {
        (0, _) => Vec::new(),
        (_, Some(names)) => names.split('\n').collect(),
        (_, None) => return fail(r, "controller file names are not utf-8"),
    };
    if names.len() != file_count as usize {
        return fail(
            r,
            &format!(
                "{} controller file names for {file_count} files",
                names.len()
            ),
        );
    }
    let pairs = checked(sizes_ptr, file_count as usize, "sizes")
        .and_then(|sizes| Ok((sizes, checked(data_ptr, data_len as usize, "data")?)));
    let (sizes, data) = match pairs {
        Ok(pairs) => pairs,
        Err(e) => return fail(r, &e),
    };
    let total: u64 = sizes.iter().map(|&size| size as u64).sum();
    if sizes.len() != names.len() || total != data.len() as u64 {
        return fail(
            r,
            &format!(
                "controller files hold {} bytes, their sizes add up to {total}",
                data.len()
            ),
        );
    }
    let mut files = Vec::with_capacity(names.len());
    let mut rest = data;
    for (name, &size) in names.iter().zip(sizes) {
        let (file, tail) = rest.split_at(size as usize);
        files.push((*name, file));
        rest = tail;
    }
    let declared: Vec<&str> = str_from(declared_ptr, declared_len)
        .unwrap_or("")
        .split('\n')
        .filter(|name| !name.is_empty())
        .collect();

    let loaded = guarded(
        Err("the core failed on these controllers".to_string()),
        || {
            r.scene
                .load_controllers(slot, json, &files, &declared)
                .map_err(|e| e.to_string())
        },
    );

    match loaded {
        Ok(()) => 0,
        Err(e) => fail(r, &e),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn bundle_controllers(
    ptr: *const u8,
    len: u32,
    out: *mut u32,
    out_len: u32,
) -> u32 {
    let names: Vec<&str> = str_from(ptr, len).unwrap_or("").split('\n').collect();
    let controllers = bundle::parts(&names).controllers;
    if !out.is_null() {
        for (i, &index) in controllers.iter().take(out_len as usize).enumerate() {
            *out.add(i) = index as u32;
        }
    }
    controllers.len() as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_effect_events(
    r: *const Renderer,
    out: *mut u32,
    out_len: u32,
) -> u32 {
    let events = (*r).scene.effect_events();
    if !out.is_null() {
        let whole = out_len as usize / EFFECT_EVENT_INTS * EFFECT_EVENT_INTS;
        out.copy_from_nonoverlapping(events.as_ptr(), whole.min(events.len()));
    }
    (events.len() / EFFECT_EVENT_INTS) as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_controller_state(
    r: *mut Renderer,
    slot: u32,
    wearer: u64,
    controller: u32,
    name_out: *mut u8,
    name_len: u32,
    seconds: *mut f32,
) -> i32 {
    let state = (*r)
        .scene
        .controller_state(slot as usize, wearer, controller as usize);
    let Some((name, time)) = state else {
        return -1;
    };
    if !name_out.is_null() {
        let n = name.len().min(name_len as usize);
        name_out.copy_from_nonoverlapping(name.as_ptr(), n);
    }
    if !seconds.is_null() {
        *seconds = time;
    }
    name.len() as i32
}

/// Starts the slot's animation from its first frame at scene time `at`.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_play(r: *mut Renderer, slot: u32, at: f32) {
    renderer_play_f64(r, slot, f64::from(at));
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_play_f64(r: *mut Renderer, slot: u32, at: f64) {
    if let Some(slot) = slot_in(&mut *r, slot) {
        (*r).scene.play(slot, at);
    }
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_stop(r: *mut Renderer, slot: u32) {
    if let Some(slot) = slot_in(&mut *r, slot) {
        (*r).scene.stop(slot);
    }
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_set_visible(r: *mut Renderer, slot: u32, visible: u32) {
    if let Some(slot) = slot_in(&mut *r, slot) {
        (*r).scene.set_visible(slot, visible != 0);
    }
}

/// Scales the slot's model about the bone it hangs from.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_set_scale(r: *mut Renderer, slot: u32, scale: f32) {
    if let Some(slot) = slot_in(&mut *r, slot) {
        (*r).scene.set_scale(slot, scale);
    }
}

/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_clear(r: *mut Renderer, slot: u32) {
    if let Some(slot) = slot_in(&mut *r, slot) {
        (*r).scene.clear(slot);
    }
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
    renderer_pose_f64(r, f64::from(seconds))
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_pose_f64(r: *mut Renderer, seconds: f64) -> i32 {
    let r = &mut *r;
    guarded(-1, || {
        r.scene.pose_at(seconds);
        0
    })
}

/// 1 if a later pose could differ from the last one. While 0, hosts can skip unchanged frames.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_is_animating(r: *const Renderer) -> u32 {
    let r = &*r;
    r.scene.is_animating() as u32
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

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_part_id(
    r: *mut Renderer,
    name_ptr: *const u8,
    name_len: u32,
) -> u32 {
    let name = str_from(name_ptr, name_len).unwrap_or("");
    (*r).scene.part_id(name) as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_query_id(
    r: *mut Renderer,
    name_ptr: *const u8,
    name_len: u32,
) -> u32 {
    let name = str_from(name_ptr, name_len).unwrap_or("");
    (*r).scene.query_id(name) as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_build_skin(r: *mut Renderer) -> i32 {
    let r = &mut *r;
    guarded(-1, || (r.scene.build_skin().glow.len() / 4) as i32)
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_layout(r: *const Renderer) -> u32 {
    (*r).scene.layout()
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_skin_bone_count(r: *const Renderer) -> u32 {
    (*r).scene.skin.bone_count
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_skin_run_count(r: *const Renderer) -> u32 {
    (*r).scene.skin.runs.len() as u32
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_frame(
    r: *mut Renderer,
    seconds: f32,
    parts: *const f32,
    part_count: u32,
    queries: *const f32,
    query_count: u32,
    space: u32,
) -> i32 {
    renderer_frame_f64(
        r,
        f64::from(seconds),
        parts,
        part_count,
        queries,
        query_count,
        space,
    )
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_frame_f64(
    r: *mut Renderer,
    seconds: f64,
    parts: *const f32,
    part_count: u32,
    queries: *const f32,
    query_count: u32,
    space: u32,
) -> i32 {
    let r = &mut *r;
    let views = checked(parts, (part_count as usize).saturating_mul(12), "parts")
        .and_then(|parts| Ok((parts, checked(queries, query_count as usize, "queries")?)));
    let (parts, queries) = match views {
        Ok(views) => views,
        Err(e) => return fail(r, &e),
    };
    let to_host = space_transform(space);

    guarded(-1, || {
        r.scene.set_game_parts_flat(parts);
        for (id, &value) in queries.iter().enumerate() {
            r.scene.set_query_at(id, value);
        }
        r.scene.pose_at(seconds);
        r.scene.skin_frame(&to_host);
        r.scene.skin.bone_count as i32
    })
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_frame_wearers(
    r: *mut Renderer,
    clock: f64,
    wearers: *const u64,
    wearer_count: u32,
    input: *const f32,
    input_len: u32,
    part_count: u32,
    query_count: u32,
    space: u32,
    out: *mut f32,
    out_len: u32,
) -> i32 {
    let r = &mut *r;
    let views = checked(wearers, wearer_count as usize, "wearers").and_then(|wearers| {
        let input = checked(input, input_len as usize, "input")?;
        Ok((wearers, input, checked_mut(out, out_len as usize, "out")?))
    });
    let (wearers, input, out) = match views {
        Ok(views) => views,
        Err(e) => return fail(r, &e),
    };
    let shape = FrameShape {
        parts: part_count as usize,
        queries: query_count as usize,
    };
    let to_host = space_transform(space);

    let framed = guarded(None, || {
        Some(
            r.scene
                .frame_wearers(clock, wearers, shape, input, &to_host, out),
        )
    });
    match framed {
        Some(Ok(bones)) => bones as i32,
        Some(Err(e)) => fail(r, &e.to_string()),
        None => core_failed(r, "the core failed on this frame"),
    }
}

#[no_mangle]
pub extern "C" fn renderer_wearer_input_stride(part_count: u32, query_count: u32) -> u32 {
    FrameShape {
        parts: part_count as usize,
        queries: query_count as usize,
    }
    .stride() as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_wearer_output_stride(r: *mut Renderer) -> i32 {
    let r = &mut *r;
    match guarded(None, || Some(r.scene.wearer_output_stride() as i32)) {
        Some(stride) => stride,
        None => core_failed(r, "the core failed building the skin"),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_set_motion(r: *mut Renderer, motion: u32) -> i32 {
    let r = &mut *r;
    match u8::try_from(motion).ok().and_then(Motion::from_value) {
        Some(motion) => {
            r.scene.set_motion(motion);
            0
        }
        None => fail(r, &format!("no motion {motion}")),
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_forget_wearer(r: *mut Renderer, wearer: u64) -> u32 {
    (*r).scene.forget_wearer(wearer) as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_break_rings(r: *mut Renderer, wearer: u64) -> u32 {
    (*r).scene.break_rings(wearer) as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_set_signal_override(
    r: *mut Renderer,
    wearer: u64,
    signal: u32,
    value: f32,
) -> i32 {
    let r = &mut *r;
    if r.scene.set_signal_override(wearer, signal as usize, value) {
        0
    } else {
        fail(r, &format!("no signal {signal}"))
    }
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_set_seed_wearer(r: *mut Renderer, wearer: u64, seed: u64) {
    (*r).scene.set_seed_wearer(wearer, seed)
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_wearer_count(r: *const Renderer) -> u32 {
    (*r).scene.wearer_count() as u32
}

#[allow(clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_wearer_bytes(r: *const Renderer) -> u32 {
    (*r).scene.wearer_bytes() as u32
}

/// Builds the 3D mesh from the last pose in `space` (1 is `java::MODEL`). Quad count, or -1.
/// # Safety
/// `r` must come from `renderer_new`.
#[no_mangle]
pub unsafe extern "C" fn renderer_build_mesh(r: *mut Renderer, space: u32) -> i32 {
    let r = &mut *r;
    let to_host = space_transform(space);
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
    renderer_build_f64(
        r,
        yaw,
        pitch,
        distance,
        target_y,
        width,
        height,
        f64::from(seconds),
    )
}

#[allow(clippy::too_many_arguments, clippy::missing_safety_doc)]
#[no_mangle]
pub unsafe extern "C" fn renderer_build_f64(
    r: *mut Renderer,
    yaw: f32,
    pitch: f32,
    distance: f32,
    target_y: f32,
    width: f32,
    height: f32,
    seconds: f64,
) -> i32 {
    let camera = Camera::orbit(yaw, pitch, distance, [0.0, target_y, 0.0]);
    let r = &mut *r;
    guarded(-1, || {
        r.scene.pose_at(seconds);
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
    ($name:ident, $field:ident, $ty:ty) => {
        #[allow(clippy::missing_safety_doc)]
        #[no_mangle]
        pub unsafe extern "C" fn $name(r: *const Renderer) -> *const $ty {
            let r = &*r;
            r.scene.$field.as_ptr()
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

field_ptr!(renderer_skin_positions, skin.positions, f32);
field_ptr!(renderer_skin_normals, skin.normals, f32);
field_ptr!(renderer_skin_uvs, skin.uvs, f32);
field_ptr!(renderer_skin_glow, skin.glow, u8);
field_ptr!(renderer_skin_bones, skin.bones, u32);
field_ptr!(renderer_skin_runs, skin.runs, MeshRun);
field_ptr!(renderer_skin_transforms, skin_transforms, f32);
field_ptr!(renderer_skin_offsets, skin_offsets, f32);

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

unsafe fn checked<'a, T>(ptr: *const T, len: usize, name: &str) -> Result<&'a [T], String> {
    match (len, ptr.is_null()) {
        (0, _) => Ok(&[]),
        (_, true) => Err(format!("{name} is null with a length of {len}")),
        _ => Ok(std::slice::from_raw_parts(ptr, len)),
    }
}

unsafe fn checked_mut<'a, T>(ptr: *mut T, len: usize, name: &str) -> Result<&'a mut [T], String> {
    match (len, ptr.is_null()) {
        (0, _) => Ok(&mut []),
        (_, true) => Err(format!("{name} is null with a length of {len}")),
        _ => Ok(std::slice::from_raw_parts_mut(ptr, len)),
    }
}

fn slot_index(slot: i64) -> Result<usize, String> {
    match usize::try_from(slot) {
        Ok(index) if slot < i64::from(SLOTS) => Ok(index),
        _ => Err(format!("no slot {slot}, the last is {}", SLOTS - 1)),
    }
}

fn slot_in(r: &mut Renderer, slot: u32) -> Option<usize> {
    slot_index(i64::from(slot)).map_err(|e| fail(r, &e)).ok()
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

fn core_failed(r: &mut Renderer, message: &str) -> i32 {
    fail(r, message);
    -2
}

fn space_transform(space: u32) -> Affine {
    if space == 1 {
        java::MODEL
    } else {
        Affine::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    pub(crate) fn file(path: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../core")
                .join(path),
        )
        .unwrap()
    }

    pub(crate) fn tree(dir: &str) -> Vec<(String, Vec<u8>)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../core")
            .join(dir);
        let (mut files, mut dirs) = (Vec::new(), vec![root.clone()]);
        while let Some(at) = dirs.pop() {
            for path in std::fs::read_dir(at).unwrap().map(|e| e.unwrap().path()) {
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    let name = path.strip_prefix(&root).unwrap().to_string_lossy();
                    files.push((name.replace('\\', "/"), std::fs::read(&path).unwrap()));
                }
            }
        }
        files.sort();
        files
    }

    pub(crate) fn bundle(files: &[(String, Vec<u8>)]) -> Vec<u8> {
        let files: Vec<_> = files
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        zip(&files)
    }

    pub(crate) fn n(s: &str) -> u32 {
        s.len() as u32
    }

    pub(crate) fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in files {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    const NONE: &str = "";

    const GLASSES: &str = r#"{"format_version":"1.12.0","minecraft:geometry":[{
        "description":{"identifier":"geometry.glasses","texture_width":16,"texture_height":16},
        "bones":[{"name":"bb_main","pivot":[0,0,0],
          "cubes":[{"origin":[-4,27,-5],"size":[8,2,1],"uv":[0,0]}]}]}]}"#;

    const TIPPING: &str = r#"{"animations":{
        "animation.glasses.rest":{"loop":true,"bones":{"bb_main":{"rotation":[0,0,0]}}},
        "animation.glasses.tip":{"loop":true,"bones":{"bb_main":{"rotation":[20,0,0]}}}}}"#;

    const TIP: &str = r#"{"animation_controllers":{
        "controller.animation.glasses.tip":{"states":{
            "default":{"animations":["rest"],"transitions":[{"tipped":"q.is_sneaking"}]},
            "tipped":{"animations":["tip"],
                "particle_effects":[{"effect":"puff"}],"sound_effects":[{"effect":"ding"}],
                "transitions":[{"default":"!q.is_sneaking"}]}}}}}"#;

    const NOD: &str = r#"{"animations":{"nod":{"bones":{"head":{"rotation":[30,0,0]}}}}}"#;

    #[rustfmt::skip]
    unsafe fn cosmetic(r: *mut Renderer, slot: u32, geometry: &str, body: &str, size: u32) -> i32 {
        let (ptr, len, body_ptr) = (geometry.as_ptr(), n(geometry), body.as_ptr());
        renderer_load_cosmetic(r, slot, ptr, len, body_ptr, n(body), NONE.as_ptr(), 0, size, size)
    }

    #[rustfmt::skip]
    unsafe fn load_tipping(r: *mut Renderer, slot: u32, declared: &str, size: u32) -> i32 {
        let (names, sizes) = ("glasses.animation_controllers.json", [size]);
        let (ptr, len, declared_len) = (TIPPING.as_ptr(), n(TIPPING), n(declared));
        let (names_ptr, names_len, data, data_len) = (names.as_ptr(), n(names), TIP.as_ptr(), n(TIP));
        let declared = declared.as_ptr();
        renderer_load_controllers(
            r, slot, ptr, len, names_ptr, names_len, data, data_len, sizes.as_ptr(), 1, declared, declared_len,
        )
    }

    #[rustfmt::skip]
    unsafe fn wearers(r: *mut Renderer, ids: &[u64], input: &[f32], space: u32, out: &mut [f32]) -> i32 {
        let (count, ptr, len) = (ids.len() as u32, input.as_ptr(), input.len() as u32);
        let (out_ptr, out_len) = (out.as_mut_ptr(), out.len() as u32);
        renderer_frame_wearers(r, 1.0, ids.as_ptr(), count, ptr, len, 0, 0, space, out_ptr, out_len)
    }

    #[test]
    fn a_cosmetic_round_trips_through_the_abi() {
        unsafe {
            let r = renderer_new();
            renderer_load_player(r, 0, 0);
            assert_eq!(cosmetic(r, 1, GLASSES, "glasses", 32), 0);
            assert!(renderer_build(r, 0.0, 10.0, 62.0, 18.0, 512.0, 512.0, 0.0) > 0);
            let count = renderer_run_count(r) as usize * 3;
            let runs = std::slice::from_raw_parts(renderer_runs(r) as *const u32, count);
            assert!(
                runs.chunks(3).any(|run| run[0] == 1),
                "slot 1 draws with texture 1"
            );
            renderer_free(r);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn errors_name_the_problem() {
        unsafe {
            let r = renderer_new();
            assert_eq!(cosmetic(r, 3, "{", "hat", 0), -1);
            assert!((*r).error.contains("malformed geometry"));
            assert_eq!(cosmetic(r, 1, GLASSES, "tail", 0), -1);
            assert!((*r).error.contains("unknown body slot"));
            let (input, mut out, ids) = ([0.0f32; 64], [0.0f32; 64], [1u64]);
            let frame = |ids: *const u64, input: *const f32, out: *mut f32| {
                let code = renderer_frame_wearers(r, 1.0, ids, 1, input, 64, 0, 0, 0, out, 64);
                (code, (*r).error.clone())
            };
            let refused = [
                (frame(null(), input.as_ptr(), out.as_mut_ptr()), "wearers is null with a length of 1"),
                (frame(ids.as_ptr(), null(), out.as_mut_ptr()), "input is null with a length of 64"),
                (frame(ids.as_ptr(), input.as_ptr(), null_mut()), "out is null with a length of 64"),
            ];
            for (failure, message) in refused {
                assert_eq!(failure, (-1, message.to_string()));
            }
            assert_eq!(renderer_frame(r, 1.0, null(), 1, null(), 0, 0), -1);
            assert_eq!((*r).error, "parts is null with a length of 12");
            renderer_free(r);
        }
    }

    #[test]
    fn a_game_host_round_trips_parts_and_builds_a_mesh() {
        let head = "head";
        unsafe {
            let r = renderer_new();
            renderer_load_player(r, 0, 0);
            renderer_set_visible(r, 0, 0);
            renderer_load_animation(r, 0, NOD.as_ptr(), n(NOD));
            renderer_play(r, 0, 0.0);
            cosmetic(r, 1, GLASSES, "glasses", 16);
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
            let [x, y, z, rx, ry, rz, sx, sy, sz] = d;
            let head = head.as_ptr();
            renderer_set_game_part(r, head, 4, 0.0, 0.0, 0.0, x, y, z, rx, ry, rz, sx, sy, sz);

            let quads = renderer_build_mesh(r, 1);
            assert_eq!(
                quads, 6,
                "only the glasses: the hidden player is the game's to draw"
            );
            let count = renderer_mesh_run_count(r) as usize * 4;
            let runs = std::slice::from_raw_parts(renderer_mesh_runs(r) as *const u32, count);
            assert_eq!(runs, [1, 0, 6, 0]);
            renderer_free(r);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn one_wearer_call_matches_single_frames_and_refuses_short_buffers() {
        let sneak = r#"{"animations":{"a":{"loop":true,"bones":{"bb_main":{"rotation":["q.is_sneaking * 20",0,0]}}}}}"#;
        unsafe {
            let r = renderer_new();
            cosmetic(r, 0, GLASSES, "glasses", 16);
            assert_eq!(renderer_load_animation(r, 0, sneak.as_ptr(), n(sneak)), 0);
            renderer_play(r, 0, 0.0);
            let id = renderer_query_id(r, "is_sneaking".as_ptr(), 11);
            let mut singles = Vec::new();
            for on in [1.0, 0.0] {
                assert!(renderer_frame(r, 1.0, null(), 0, [on].as_ptr(), id + 1, 1) >= 0);
                singles.extend_from_slice(&(*r).scene.skin_transforms);
                singles.extend_from_slice(&(*r).scene.skin_offsets);
            }
            let mut input = vec![0.0f32; renderer_wearer_input_stride(0, 0) as usize * 2];
            input[plusrenderer::signals::signal_id("is_sneaking").unwrap()] = 1.0;
            let mut out = vec![0.0f32; renderer_wearer_output_stride(r) as usize * 2];
            assert!(wearers(r, &[11, 12], &input, 1, &mut out) >= 0);
            assert_eq!(out, singles);
            assert_eq!(renderer_wearer_count(r), 3, "two wearers and the preview");
            renderer_set_seed_wearer(r, 11, 12);
            assert!(wearers(r, &[11, 12], &input, 1, &mut out) >= 0);
            assert_eq!(out, singles, "a seed wearer leaves unseeded poses alone");
            assert_eq!((renderer_forget_wearer(r, 11), renderer_wearer_count(r)), (1, 2));
            assert_eq!((renderer_set_motion(r, 3), (*r).scene.motion()), (-1, Motion::Full));
            assert_eq!(wearers(r, &[11, 12], &input, 1, &mut out[1..]), -1);
            assert!((*r).error.starts_with("out holds"));
            assert_eq!(wearers(r, &[11, 12], &input[1..], 1, &mut out), -1);
            assert!((*r).error.starts_with("input holds"));
            assert!(wearers(r, &[11, 12], &input, 1, &mut out) >= 0, "a refused call leaves the scene usable");
            renderer_free(r);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn controllers_run_in_the_wearer_call_and_report_through_the_abi() {
        unsafe {
            let r = renderer_new();
            cosmetic(r, 0, GLASSES, "glasses", 16);
            assert_eq!(load_tipping(r, 0, "controller.animation.glasses.tip", n(TIP)), 0);
            renderer_play(r, 0, 0.0);
            let mut input = vec![0.0f32; renderer_wearer_input_stride(0, 0) as usize * 2];
            input[plusrenderer::signals::signal_id("is_sneaking").unwrap()] = 1.0;
            let mut out = vec![0.0f32; renderer_wearer_output_stride(r) as usize * 2];
            assert!(wearers(r, &[21, 22], &input, 0, &mut out) >= 0);
            let (mut events, mut short) = ([u32::MAX; 12], [u32::MAX; 8]);
            assert_eq!(renderer_effect_events(r, events.as_mut_ptr(), 12), 2);
            assert_eq!(events, [0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 1]);
            assert_eq!(renderer_effect_events(r, short.as_mut_ptr(), 8), 2);
            assert_eq!(short, [0, 0, 0, 1, 0, 0, u32::MAX, u32::MAX], "only whole events are copied");
            assert_eq!(renderer_effect_events(r, null_mut(), 0), 2);
            let mut seconds = -1.0f32;
            let state = |wearer, name: &mut [u8], seconds: &mut f32| {
                renderer_controller_state(r, 0, wearer, 0, name.as_mut_ptr(), name.len() as u32, seconds)
            };
            let (mut name, mut tiny) = ([0u8; 16], [0u8; 3]);
            assert_eq!((state(21, &mut name, &mut seconds), &name[..6], seconds), (6, &b"tipped"[..], 0.0));
            assert_eq!((state(22, &mut name, &mut seconds), &name[..7]), (7, &b"default"[..]));
            assert_eq!(state(23, &mut name, &mut seconds), -1);
            assert_eq!((state(22, &mut tiny, &mut seconds), &tiny), (7, b"def"));
            assert_eq!(load_tipping(r, 0, "controller.animation.glasses.gone", n(TIP)), -1);
            assert!((*r).error.contains("controller_missing"), "{}", (*r).error);
            assert_eq!(load_tipping(r, 0, NONE, n(TIP) + 1), -1);
            renderer_free(r);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn slots_past_the_last_are_refused_before_the_scene_grows() {
        unsafe {
            let r = renderer_new();
            assert_eq!(cosmetic(r, 255, GLASSES, "glasses", 16), 0, "{}", (*r).error);
            for slot in [256, u32::MAX] {
                let refused = format!("no slot {slot}, the last is 255");
                let loads: [&dyn Fn() -> i32; 5] = [
                    &|| cosmetic(r, slot, GLASSES, "glasses", 16),
                    &|| renderer_load_animation(r, slot, NOD.as_ptr(), n(NOD)),
                    &|| load_tipping(r, slot, NONE, n(TIP)),
                    &|| renderer_load_cape(r, slot, 64, 32, 0, 0, NONE.as_ptr(), 0),
                    &|| renderer_load_elytra(r, slot, 64, 32, 0, 0, NONE.as_ptr(), 0),
                ];
                let quiet: [&dyn Fn(); 7] = [
                    &|| renderer_load_player(r, slot, 0), &|| renderer_play(r, slot, 0.0), &|| renderer_stop(r, slot),
                    &|| renderer_set_visible(r, slot, 0), &|| renderer_set_scale(r, slot, 2.0),
                    &|| renderer_set_elytra_pose(r, slot, 2, 0.5), &|| renderer_clear(r, slot),
                ];
                for load in loads {
                    (*r).error.clear();
                    assert_eq!((load(), &(*r).error), (-1, &refused));
                }
                for call in quiet {
                    (*r).error.clear();
                    call();
                    assert_eq!(&(*r).error, &refused);
                }
            }
            renderer_free(r);
        }
    }

    #[test]
    fn guarded_turns_a_panic_into_failed() {
        assert_eq!(guarded(-1, || panic!("core bug")), -1);
    }

    #[test]
    fn bundle_names_cross_as_newline_joined_lists() {
        let names = "wings.png\nwings.geo.json";
        let controllers = "a.png\nx.animation_controllers.json\nanimation_controllers/y.json";
        let mut out = [u32::MAX; 4];
        unsafe {
            assert_eq!(bundle_part(names.as_ptr(), n(names), 0), 1);
            assert_eq!(bundle_part(names.as_ptr(), n(names), 2), 0);
            assert_eq!(bundle_part(names.as_ptr(), n(names), 3), -1);
            let (ptr, len) = (controllers.as_ptr(), n(controllers));
            assert_eq!(bundle_controllers(ptr, len, out.as_mut_ptr(), 4), 2);
            assert_eq!(out, [2, 1, u32::MAX, u32::MAX]);
            assert_eq!(bundle_controllers(ptr, len, out.as_mut_ptr(), 1), 2);
        }
    }
}
