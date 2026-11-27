//! JNI for `org.polyfrost.plusrenderer.Renderer` (`java/`): the C ABI above, with Java strings in and
//! arrays copied out. The Java side checks handles and turns the -1 codes into exceptions.
#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::mem;
use std::sync::{Mutex, OnceLock, PoisonError};

use jni::objects::{GlobalRef, JByteArray, JClass, JFloatArray, JIntArray, JLongArray, JString};
use jni::sys::{
    jboolean, jbyteArray, jdouble, jfloat, jfloatArray, jint, jintArray, jlong, jstring,
};
use jni::JNIEnv;

use plusrenderer::scene::FrameError;

use super::*;

fn text(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(Into::into).unwrap_or_default()
}

fn with_name<R>(env: &mut JNIEnv, s: &JString, f: impl FnOnce(&str) -> R) -> R {
    let java = if s.is_null() {
        None
    } else {
        env.get_string(s).ok()
    };
    match &java {
        Some(java) => {
            let text: Cow<str> = java.into();
            f(&text)
        }
        None => f(""),
    }
}

pub(crate) fn with_name_checked<R>(
    env: &mut JNIEnv,
    s: &JString,
    f: impl FnOnce(&str) -> R,
) -> Option<R> {
    if s.is_null() {
        return None;
    }
    let java = env.get_string(s).ok()?;
    let text: Cow<str> = (&java).into();
    Some(f(&text))
}

const NAME_CACHE_LIMIT: usize = 4096;

static NAME_CACHE: OnceLock<Mutex<HashMap<String, GlobalRef>>> = OnceLock::new();

pub(crate) fn cached_string(env: &mut JNIEnv, text: &str) -> jstring {
    let mut cache = NAME_CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(global) = cache.get(text) {
        return env
            .new_local_ref(global)
            .map_or(std::ptr::null_mut(), |s| s.into_raw());
    }
    let Ok(local) = env.new_string(text) else {
        return std::ptr::null_mut();
    };
    if cache.len() < NAME_CACHE_LIMIT {
        if let Ok(global) = env.new_global_ref(&local) {
            cache.insert(text.to_owned(), global);
        }
    }
    local.into_raw()
}

fn len(s: &str) -> u32 {
    s.len() as u32
}

fn scene<'a>(r: jlong) -> &'a Scene {
    // SAFETY: the Java side only passes live handles from `nCreate`.
    unsafe { &(*(r as *const Renderer)).scene }
}

fn floats(env: &mut JNIEnv, data: &[f32]) -> jfloatArray {
    match env.new_float_array(data.len() as jint) {
        Ok(array) if env.set_float_array_region(&array, 0, data).is_ok() => array.into_raw(),
        _ => std::ptr::null_mut(),
    }
}

fn ints(env: &mut JNIEnv, data: &[i32]) -> jintArray {
    match env.new_int_array(data.len() as jint) {
        Ok(array) if env.set_int_array_region(&array, 0, data).is_ok() => array.into_raw(),
        _ => std::ptr::null_mut(),
    }
}

/// `DrawRun` / `MeshRun` are `repr(C)` structs of `u32`, read as their flat words.
fn words<T>(runs: &[T]) -> &[i32] {
    let n = std::mem::size_of_val(runs) / 4;
    // SAFETY: both run types are `repr(C)` and made only of `u32` fields.
    unsafe { std::slice::from_raw_parts(runs.as_ptr().cast(), n) }
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nCreate(
    _: JNIEnv,
    _: JClass,
) -> jlong {
    renderer_new() as jlong
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nFree(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) {
    renderer_free(r as *mut Renderer);
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nError(
    env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jstring {
    let error = &(*(r as *const Renderer)).error;
    env.new_string(error)
        .map_or(std::ptr::null_mut(), |s| s.into_raw())
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLoadPlayer(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    slim: jboolean,
) {
    renderer_load_player(r as *mut Renderer, slot as u32, slim as u32);
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLoadCosmetic(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    json: JString,
    body: JString,
    anchor: JString,
    image_width: jint,
    image_height: jint,
) -> jint {
    let (json, body, anchor) = (
        text(&mut env, &json),
        text(&mut env, &body),
        text(&mut env, &anchor),
    );
    renderer_load_cosmetic(
        r as *mut Renderer,
        slot as u32,
        json.as_ptr(),
        len(&json),
        body.as_ptr(),
        len(&body),
        anchor.as_ptr(),
        len(&anchor),
        image_width as u32,
        image_height as u32,
    )
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLoadCape(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    still_width: jint,
    still_height: jint,
    sheet_width: jint,
    sheet_height: jint,
    name: JString,
) -> jint {
    let name = text(&mut env, &name);
    renderer_load_cape(
        r as *mut Renderer,
        slot as u32,
        still_width as u32,
        still_height as u32,
        sheet_width as u32,
        sheet_height as u32,
        name.as_ptr(),
        len(&name),
    )
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLoadElytra(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    still_width: jint,
    still_height: jint,
    sheet_width: jint,
    sheet_height: jint,
    name: JString,
) -> jint {
    let name = text(&mut env, &name);
    renderer_load_elytra(
        r as *mut Renderer,
        slot as u32,
        still_width as u32,
        still_height as u32,
        sheet_width as u32,
        sheet_height as u32,
        name.as_ptr(),
        len(&name),
    )
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nBundlePart(
    mut env: JNIEnv,
    _: JClass,
    names: JString,
    kind: jint,
) -> jint {
    let names = text(&mut env, &names);
    bundle_part(names.as_ptr(), len(&names), kind as u32)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLoadAnimation(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    json: JString,
) -> jint {
    let json = text(&mut env, &json);
    renderer_load_animation(r as *mut Renderer, slot as u32, json.as_ptr(), len(&json))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLoadControllers(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    json: JString,
    names: JString,
    data: JByteArray,
    sizes: JIntArray,
    declared: JString,
) -> jint {
    let renderer = &mut *(r as *mut Renderer);
    let (json, names, declared) = (
        text(&mut env, &json),
        text(&mut env, &names),
        text(&mut env, &declared),
    );
    let Ok(data) = env.convert_byte_array(&data) else {
        return fail(renderer, "could not read the controller files");
    };
    let count = env.get_array_length(&sizes).unwrap_or(0).max(0) as usize;
    let mut lengths = vec![0i32; count];
    if env.get_int_array_region(&sizes, 0, &mut lengths).is_err() {
        return fail(renderer, "could not read the controller file sizes");
    }
    if lengths.iter().any(|&size| size < 0) {
        return fail(renderer, "controller file sizes must not be negative");
    }
    let lengths: Vec<u32> = lengths.iter().map(|&size| size as u32).collect();
    renderer_load_controllers(
        renderer,
        slot as u32,
        json.as_ptr(),
        len(&json),
        names.as_ptr(),
        len(&names),
        data.as_ptr(),
        data.len() as u32,
        lengths.as_ptr(),
        count as u32,
        declared.as_ptr(),
        len(&declared),
    )
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nBundleControllers(
    mut env: JNIEnv,
    _: JClass,
    names: JString,
) -> jintArray {
    let names = text(&mut env, &names);
    let list: Vec<&str> = names.split('\n').collect();
    let indices: Vec<i32> = bundle::parts(&list)
        .controllers
        .iter()
        .map(|&i| i as i32)
        .collect();
    ints(&mut env, &indices)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nEffectEvents(
    env: JNIEnv,
    _: JClass,
    r: jlong,
    out: JIntArray,
) -> jint {
    let renderer = &mut *(r as *mut Renderer);
    let events = renderer.scene.effect_events();
    let held = env.get_array_length(&out).unwrap_or(0).max(0) as usize;
    let whole = (held / EFFECT_EVENT_INTS * EFFECT_EVENT_INTS).min(events.len());
    if whole > 0
        && env
            .set_int_array_region(&out, 0, words(&events[..whole]))
            .is_err()
    {
        return fail(renderer, "could not write the effect events");
    }
    (events.len() / EFFECT_EVENT_INTS) as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nControllerState(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    wearer: jlong,
    controller: jint,
    seconds: JFloatArray,
) -> jstring {
    if slot < 0 || controller < 0 {
        return std::ptr::null_mut();
    }
    let scene = &mut (*(r as *mut Renderer)).scene;
    let Some((name, time)) =
        scene.controller_state(slot as usize, wearer as u64, controller as usize)
    else {
        return std::ptr::null_mut();
    };
    if env.set_float_array_region(&seconds, 0, &[time]).is_err() {
        return std::ptr::null_mut();
    }
    cached_string(&mut env, name)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nPlay(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    at: jdouble,
) {
    renderer_play_f64(r as *mut Renderer, slot as u32, at);
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nStop(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
) {
    renderer_stop(r as *mut Renderer, slot as u32);
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetElytraPose(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    pose: jint,
    fall: jfloat,
) {
    renderer_set_elytra_pose(r as *mut Renderer, slot as u32, pose as u32, fall);
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetVisible(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    visible: jboolean,
) {
    renderer_set_visible(r as *mut Renderer, slot as u32, visible as u32);
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetScale(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    scale: jfloat,
) {
    renderer_set_scale(r as *mut Renderer, slot as u32, scale);
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nClear(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
) {
    renderer_clear(r as *mut Renderer, slot as u32);
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetLightLevel(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    level: jfloat,
) {
    renderer_set_light_level(r as *mut Renderer, level);
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nPose(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    seconds: jfloat,
) -> jint {
    renderer_pose(r as *mut Renderer, seconds)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nPoseF64(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    seconds: jdouble,
) -> jint {
    renderer_pose_f64(r as *mut Renderer, seconds)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetGamePart(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    name: JString,
    rest_x: jfloat,
    rest_y: jfloat,
    rest_z: jfloat,
    x: jfloat,
    y: jfloat,
    z: jfloat,
    x_rot: jfloat,
    y_rot: jfloat,
    z_rot: jfloat,
    x_scale: jfloat,
    y_scale: jfloat,
    z_scale: jfloat,
) {
    with_name(&mut env, &name, |name| {
        renderer_set_game_part(
            r as *mut Renderer,
            name.as_ptr(),
            len(name),
            rest_x,
            rest_y,
            rest_z,
            x,
            y,
            z,
            x_rot,
            y_rot,
            z_rot,
            x_scale,
            y_scale,
            z_scale,
        )
    });
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetQuery(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    name: JString,
    value: jfloat,
) {
    with_name(&mut env, &name, |name| {
        renderer_set_query(r as *mut Renderer, name.as_ptr(), len(name), value)
    });
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nClearGameParts(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) {
    renderer_clear_game_parts(r as *mut Renderer);
}

unsafe fn part_delta(
    env: &mut JNIEnv,
    r: jlong,
    slot: jint,
    name: &JString,
    delta: &mut [f32; 9],
) -> bool {
    with_name(env, name, |name| {
        renderer_part_delta(
            r as *const Renderer,
            slot as u32,
            name.as_ptr(),
            len(name),
            delta.as_mut_ptr(),
        ) >= 0
    })
}

/// Nine floats (offset, rotation in radians, scale), or null if the slot has no such bone.
#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nPartDelta(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    name: JString,
) -> jfloatArray {
    let mut delta = [0.0f32; 9];
    if !part_delta(&mut env, r, slot, &name, &mut delta) {
        return std::ptr::null_mut();
    }

    floats(&mut env, &delta)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nPartDeltaInto(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    name: JString,
    out: JFloatArray,
) -> jboolean {
    let mut delta = [0.0f32; 9];
    let found = part_delta(&mut env, r, slot, &name, &mut delta)
        && env.get_array_length(&out).is_ok_and(|held| held >= 9)
        && env.set_float_array_region(&out, 0, &delta).is_ok();
    found as jboolean
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nBuildMesh(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    space: jint,
) -> jint {
    renderer_build_mesh(r as *mut Renderer, space as u32)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nBuild(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    yaw: jfloat,
    pitch: jfloat,
    distance: jfloat,
    target_y: jfloat,
    width: jfloat,
    height: jfloat,
    seconds: jfloat,
) -> jint {
    renderer_build(
        r as *mut Renderer,
        yaw,
        pitch,
        distance,
        target_y,
        width,
        height,
        seconds,
    )
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nBuildF64(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    yaw: jfloat,
    pitch: jfloat,
    distance: jfloat,
    target_y: jfloat,
    width: jfloat,
    height: jfloat,
    seconds: jdouble,
) -> jint {
    renderer_build_f64(
        r as *mut Renderer,
        yaw,
        pitch,
        distance,
        target_y,
        width,
        height,
        seconds,
    )
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nMeshPositions(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jfloatArray {
    floats(&mut env, &scene(r).mesh.positions)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nMeshNormals(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jfloatArray {
    floats(&mut env, &scene(r).mesh.normals)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nMeshUvs(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jfloatArray {
    floats(&mut env, &scene(r).mesh.uvs)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nMeshGlow(
    env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jbyteArray {
    env.byte_array_from_slice(&scene(r).mesh.glow)
        .map_or(std::ptr::null_mut(), |a| a.into_raw())
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nMeshRuns(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jintArray {
    ints(&mut env, words(&scene(r).mesh.runs))
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nPositions(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jfloatArray {
    floats(&mut env, &scene(r).buffers.positions)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nUvs(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jfloatArray {
    floats(&mut env, &scene(r).buffers.uvs)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nColors(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jintArray {
    ints(&mut env, words(&scene(r).buffers.colors))
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nIndices(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jintArray {
    let indices: Vec<i32> = scene(r).buffers.indices.iter().map(|&i| i as i32).collect();
    ints(&mut env, &indices)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nRuns(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jintArray {
    ints(&mut env, words(&scene(r).buffers.runs))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nPartId(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    name: JString,
) -> jint {
    with_name(&mut env, &name, |name| {
        renderer_part_id(r as *mut Renderer, name.as_ptr(), len(name)) as jint
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nQueryId(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    name: JString,
) -> jint {
    with_name(&mut env, &name, |name| {
        renderer_query_id(r as *mut Renderer, name.as_ptr(), len(name)) as jint
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nBuildSkin(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) -> jint {
    renderer_build_skin(r as *mut Renderer)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLayout(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) -> jint {
    renderer_layout(r as *const Renderer) as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSkinBoneCount(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) -> jint {
    renderer_skin_bone_count(r as *const Renderer) as jint
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSkinPositions(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jfloatArray {
    floats(&mut env, &scene(r).skin.positions)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSkinNormals(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jfloatArray {
    floats(&mut env, &scene(r).skin.normals)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSkinUvs(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jfloatArray {
    floats(&mut env, &scene(r).skin.uvs)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSkinGlow(
    env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jbyteArray {
    env.byte_array_from_slice(&scene(r).skin.glow)
        .map_or(std::ptr::null_mut(), |a| a.into_raw())
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSkinBones(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jintArray {
    ints(&mut env, words(&scene(r).skin.bones))
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSkinRuns(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
) -> jintArray {
    ints(&mut env, words(&scene(r).skin.runs))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nFrame(
    env: JNIEnv,
    _: JClass,
    r: jlong,
    seconds: jfloat,
    input: JFloatArray,
    part_count: jint,
    query_count: jint,
    space: jint,
    out: JFloatArray,
) -> jint {
    let counts = (part_count, query_count, space);
    frame_into(env, r, f64::from(seconds), input, counts, out)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nFrameF64(
    env: JNIEnv,
    _: JClass,
    r: jlong,
    seconds: jdouble,
    input: JFloatArray,
    part_count: jint,
    query_count: jint,
    space: jint,
    out: JFloatArray,
) -> jint {
    let counts = (part_count, query_count, space);
    frame_into(env, r, seconds, input, counts, out)
}

unsafe fn frame_into(
    env: JNIEnv,
    r: jlong,
    seconds: f64,
    input: JFloatArray,
    (part_count, query_count, space): (jint, jint, jint),
    out: JFloatArray,
) -> jint {
    let renderer = &mut *(r as *mut Renderer);
    let (parts, queries) = (part_count.max(0) as usize * 12, query_count.max(0) as usize);

    renderer.input.resize(parts + queries, 0.0);
    if env
        .get_float_array_region(&input, 0, &mut renderer.input)
        .is_err()
    {
        return fail(
            renderer,
            "input holds fewer floats than its parts and queries",
        );
    }
    let input = mem::take(&mut renderer.input);
    let bones = renderer_frame_f64(
        renderer,
        seconds,
        input.as_ptr(),
        part_count.max(0) as u32,
        input[parts..].as_ptr(),
        query_count.max(0) as u32,
        space as u32,
    );
    renderer.input = input;
    if bones < 0 {
        return bones;
    }

    let scene = &renderer.scene;
    let needed = scene.skin_transforms.len() + scene.skin_offsets.len();
    let held = env.get_array_length(&out).unwrap_or(0) as usize;
    if held < needed {
        return fail(
            renderer,
            &format!("out holds {held} floats, the frame needs {needed}"),
        );
    }
    let written = env
        .set_float_array_region(&out, 0, &scene.skin_transforms)
        .is_ok()
        && env
            .set_float_array_region(
                &out,
                scene.skin_transforms.len() as i32,
                &scene.skin_offsets,
            )
            .is_ok();
    if written {
        bones
    } else {
        fail(renderer, "could not write the frame")
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nFrameWearers(
    env: JNIEnv,
    _: JClass,
    r: jlong,
    clock: jdouble,
    wearers: JLongArray,
    count: jint,
    input: JFloatArray,
    part_count: jint,
    query_count: jint,
    space: jint,
    out: JFloatArray,
) -> jint {
    let renderer = &mut *(r as *mut Renderer);
    if count < 0 || part_count < 0 || query_count < 0 {
        return fail(renderer, "counts must not be negative");
    }
    let count = count as usize;
    let shape = FrameShape {
        parts: part_count as usize,
        queries: query_count as usize,
    };

    let held = env.get_array_length(&wearers).unwrap_or(0) as usize;
    if held < count {
        return fail(
            renderer,
            &format!("wearers holds {held} ids, the frame has {count} wearers"),
        );
    }
    let needed = shape.stride().saturating_mul(count);
    let held = env.get_array_length(&input).unwrap_or(0) as usize;
    if held < needed {
        return fail(renderer, &FrameError::Input { held, needed }.to_string());
    }
    let stride = renderer_wearer_output_stride(renderer);
    if stride < 0 {
        return stride;
    }
    let out_needed = (stride as usize).saturating_mul(count);
    let held = env.get_array_length(&out).unwrap_or(0) as usize;
    if held < out_needed {
        return fail(
            renderer,
            &FrameError::Output {
                held,
                needed: out_needed,
            }
            .to_string(),
        );
    }

    renderer.ids.resize(count, 0);
    renderer.input.resize(needed, 0.0);
    renderer.output.resize(out_needed, 0.0);
    let read = (count == 0
        || env
            .get_long_array_region(&wearers, 0, &mut renderer.ids)
            .is_ok())
        && (needed == 0
            || env
                .get_float_array_region(&input, 0, &mut renderer.input)
                .is_ok());
    if !read {
        return fail(renderer, "could not read the frame input");
    }

    let (ids, input, mut output) = (
        mem::take(&mut renderer.ids),
        mem::take(&mut renderer.input),
        mem::take(&mut renderer.output),
    );
    let bones = renderer_frame_wearers(
        renderer,
        clock,
        ids.as_ptr().cast(),
        count as u32,
        input.as_ptr(),
        input.len() as u32,
        shape.parts as u32,
        shape.queries as u32,
        space as u32,
        output.as_mut_ptr(),
        output.len() as u32,
    );
    renderer.ids = ids;
    renderer.input = input;
    renderer.output = output;
    if bones < 0 {
        return bones;
    }

    if out_needed > 0
        && env
            .set_float_array_region(&out, 0, &renderer.output)
            .is_err()
    {
        return fail(renderer, "could not write the frame");
    }
    bones
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nWearerOutputStride(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) -> jint {
    renderer_wearer_output_stride(r as *mut Renderer)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetMotion(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    motion: jint,
) -> jint {
    if motion < 0 {
        return fail(&mut *(r as *mut Renderer), &format!("no motion {motion}"));
    }
    renderer_set_motion(r as *mut Renderer, motion as u32)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nForgetWearer(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    wearer: jlong,
) -> jboolean {
    renderer_forget_wearer(r as *mut Renderer, wearer as u64) as jboolean
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nBreakRings(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    wearer: jlong,
) -> jboolean {
    renderer_break_rings(r as *mut Renderer, wearer as u64) as jboolean
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetSignalOverride(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    wearer: jlong,
    signal: jint,
    value: jfloat,
) -> jint {
    if signal < 0 {
        return fail(&mut *(r as *mut Renderer), &format!("no signal {signal}"));
    }
    renderer_set_signal_override(r as *mut Renderer, wearer as u64, signal as u32, value)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetSeedWearer(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    wearer: jlong,
    seed: jlong,
) {
    renderer_set_seed_wearer(r as *mut Renderer, wearer as u64, seed as u64)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nWearerCount(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) -> jint {
    renderer_wearer_count(r as *const Renderer) as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nWearerBytes(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) -> jint {
    renderer_wearer_bytes(r as *const Renderer) as jint
}
