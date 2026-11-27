#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

use std::mem;
use std::sync::Arc;

use jni::objects::{
    JByteArray, JClass, JFloatArray, JIntArray, JLongArray, JPrimitiveArray, JString, TypeArray,
};
use jni::sys::{jboolean, jbyteArray, jdouble, jint, jlong, jstring};
use jni::JNIEnv;
use plusrenderer::attach::{actor_wearer, event_key, event_wearer, pair_seed};
use plusrenderer::effects::QUAD_FLOATS;
use plusrenderer::scene::{FrameError, FrameSections, EFFECT_EVENT_FLOATS};

use super::cosmetic::{
    analyze_json, effect_geometry, effect_texture, effect_texture_count, fault_next_frame,
    frame_shaped, input_layout, issues_json, key as texture_key, layout_ints, load_asset,
    load_emote, locator_id, meta_json, output_layout, parse_cosmetic, renderer_fail, resolved,
    sections_of, set_effect_colors, set_physics, set_rig, sound, sound_count, texture,
    texture_count, texture_info, view_of, whole_events, ParsedCosmetic, INPUT_STRIDE,
    LAYOUT_STRIDE, SECTION_INTS, UNHELD_SECTION_INTS, VIEW_FLOATS,
};
use super::jvm::{cached_string, with_name_checked};
use super::nameplate::Fail;
use super::{fail, Renderer};

const ILLEGAL_ARGUMENT: &str = "java/lang/IllegalArgumentException";
const ILLEGAL_STATE: &str = "java/lang/IllegalStateException";

unsafe fn parsed<'a>(h: jlong) -> &'a ParsedCosmetic {
    &*(h as *const ParsedCosmetic)
}

unsafe fn renderer<'a>(r: jlong) -> &'a mut Renderer {
    &mut *(r as *mut Renderer)
}

fn throw(env: &mut JNIEnv, fail: &Fail) {
    let class = match fail {
        Fail::Args(_) => ILLEGAL_ARGUMENT,
        Fail::Core(_) => ILLEGAL_STATE,
    };
    if !env.exception_check().unwrap_or(true) {
        let _ = env.throw_new(class, fail.message());
    }
}

fn string_out(env: &mut JNIEnv, result: Result<String, Fail>) -> jstring {
    match result {
        Ok(text) => env
            .new_string(text)
            .map_or(std::ptr::null_mut(), |s| s.into_raw()),
        Err(fail) => {
            throw(env, &fail);
            std::ptr::null_mut()
        }
    }
}

fn text(env: &mut JNIEnv, s: &JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    env.get_string(s).map(Into::into).ok()
}

fn length<T: TypeArray>(env: &JNIEnv, array: &JPrimitiveArray<T>) -> usize {
    if array.is_null() {
        0
    } else {
        env.get_array_length(array).unwrap_or(0).max(0) as usize
    }
}

fn int_vec(env: &JNIEnv, array: &JIntArray) -> Result<Vec<i32>, Fail> {
    let mut ints = vec![0i32; length(env, array)];
    if !ints.is_empty() && env.get_int_array_region(array, 0, &mut ints).is_err() {
        return Err(Fail::Args("could not read the custom colours".into()));
    }
    Ok(ints)
}

fn ints_out(env: &JNIEnv, array: &JIntArray, ints: &[i32], name: &str) -> Result<(), Fail> {
    if array.is_null() {
        return Ok(());
    }
    let held = length(env, array);
    if held < ints.len() {
        return Err(Fail::Args(format!(
            "{name} holds {held} ints, not {}",
            ints.len()
        )));
    }
    env.set_int_array_region(array, 0, ints)
        .map_err(|_| Fail::Args(format!("could not write the {name}")))
}

fn sections(env: &JNIEnv, array: &JIntArray) -> Result<FrameSections, Fail> {
    let held = length(env, array);
    if held != SECTION_INTS && held != UNHELD_SECTION_INTS {
        return Err(Fail::Args(format!(
            "sections hold {held} ints, not {SECTION_INTS}"
        )));
    }
    let mut ints = [0i32; SECTION_INTS];
    let ints = &mut ints[..held];
    if env.get_int_array_region(array, 0, ints).is_err() {
        return Err(Fail::Args("could not read the sections".into()));
    }
    sections_of(ints)
}

fn as_wearers(ids: &[i64]) -> &[u64] {
    unsafe { std::slice::from_raw_parts(ids.as_ptr().cast(), ids.len()) }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nParse(
    mut env: JNIEnv,
    _: JClass,
    bytes: JByteArray,
    mode: jint,
    identity: jlong,
    capabilities: JString,
) -> jlong {
    let Ok(bytes) = env.convert_byte_array(&bytes) else {
        throw(
            &mut env,
            &Fail::Args("could not read the bundle bytes".into()),
        );
        return 0;
    };
    let capabilities = text(&mut env, &capabilities);
    match parse_cosmetic(
        &bytes,
        i64::from(mode),
        capabilities.as_deref(),
        identity as u64,
    ) {
        Ok(parsed) => Arc::into_raw(Arc::new(parsed)) as jlong,
        Err(fail) => {
            throw(&mut env, &fail);
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nFree(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    drop(Arc::from_raw(h as *const ParsedCosmetic));
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nStatus(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) -> jint {
    parsed(h).status
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nIssuesJson(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jstring {
    string_out(&mut env, Ok(issues_json(parsed(h))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nAnalyzeJson(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jstring {
    string_out(&mut env, analyze_json(parsed(h)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nMetaJson(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jstring {
    string_out(&mut env, meta_json(parsed(h)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nTextureCount(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jint {
    match texture_count(parsed(h)) {
        Ok(count) => count as jint,
        Err(fail) => {
            throw(&mut env, &fail);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nTextureInfo(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    index: jint,
    info: JIntArray,
) -> jint {
    let t = match texture(parsed(h), i64::from(index)) {
        Ok(t) => t,
        Err(fail) => {
            throw(&mut env, &fail);
            return -1;
        }
    };
    if let Err(fail) = ints_out(&env, &info, &texture_info(t), "info") {
        throw(&mut env, &fail);
        return -1;
    }
    t.pixels().saturating_mul(4) as jint
}

fn count_out(env: &mut JNIEnv, result: Result<usize, Fail>) -> jint {
    match result {
        Ok(count) => count as jint,
        Err(fail) => {
            throw(env, &fail);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nEffectTextureCount(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jint {
    count_out(&mut env, effect_texture_count(parsed(h)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nEffectTextureInfo(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    index: jint,
    info: JIntArray,
) -> jint {
    let t = match effect_texture(parsed(h), i64::from(index)) {
        Ok(t) => t,
        Err(fail) => {
            throw(&mut env, &fail);
            return -1;
        }
    };
    if let Err(fail) = ints_out(&env, &info, &texture_info(t), "info") {
        throw(&mut env, &fail);
        return -1;
    }
    t.rgba.len() as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nEffectTexture(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    index: jint,
    rgba: JByteArray,
) -> jint {
    let t = match effect_texture(parsed(h), i64::from(index)) {
        Ok(t) => t,
        Err(fail) => {
            throw(&mut env, &fail);
            return -1;
        }
    };
    let bytes = t.rgba.len();
    if rgba.is_null() {
        return bytes as jint;
    }
    let held = length(&env, &rgba);
    if held < bytes {
        let message = format!("rgba holds {held} bytes, the texture needs {bytes}");
        throw(&mut env, &Fail::Args(message));
        return -1;
    }
    let signed: &[i8] = std::slice::from_raw_parts(t.rgba.as_ptr().cast(), bytes);
    if env.set_byte_array_region(&rgba, 0, signed).is_err() {
        throw(&mut env, &Fail::Args("could not write the texture".into()));
        return -1;
    }
    bytes as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nSoundCount(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jint {
    count_out(&mut env, sound_count(parsed(h)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nSound(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    index: jint,
) -> jbyteArray {
    let bytes = match sound(parsed(h), i64::from(index)) {
        Ok(bytes) => bytes,
        Err(fail) => {
            throw(&mut env, &fail);
            return std::ptr::null_mut();
        }
    };
    match env.byte_array_from_slice(bytes) {
        Ok(array) => array.into_raw(),
        Err(_) => {
            throw(&mut env, &Fail::Args("could not copy the sound".into()));
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nResolveTexture(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    index: jint,
    colorway: jint,
    slots: JIntArray,
    rgb: JIntArray,
    size: JIntArray,
    rgba: JByteArray,
) -> jint {
    let custom = int_vec(&env, &slots).and_then(|s| Ok((s, int_vec(&env, &rgb)?)));
    let painted = custom.and_then(|(slots, rgb)| {
        resolved(
            parsed(h),
            i64::from(index),
            i64::from(colorway),
            &slots,
            &rgb,
        )
    });
    let (t, image) = match painted {
        Ok(painted) => painted,
        Err(fail) => {
            throw(&mut env, &fail);
            return -1;
        }
    };
    let bytes = image.len();
    if !rgba.is_null() {
        let held = length(&env, &rgba);
        if held < bytes {
            let message = format!("rgba holds {held} bytes, the texture needs {bytes}");
            throw(&mut env, &Fail::Args(message));
            return -1;
        }
    }
    let dims = [t.width as jint, t.height as jint];
    if let Err(fail) = ints_out(&env, &size, &dims, "size") {
        throw(&mut env, &fail);
        return -1;
    }
    let signed: &[i8] = std::slice::from_raw_parts(image.as_ptr().cast(), bytes);
    let copied = rgba.is_null() || env.set_byte_array_region(&rgba, 0, signed).is_ok();
    if !copied {
        throw(&mut env, &Fail::Args("could not write the texture".into()));
        return -1;
    }
    bytes as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_CosmeticAsset_nTextureKey(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    index: jint,
    colorway: jint,
    slots: JIntArray,
    rgb: JIntArray,
) -> jlong {
    let custom = int_vec(&env, &slots).and_then(|s| Ok((s, int_vec(&env, &rgb)?)));
    let key = custom.and_then(|(slots, rgb)| {
        texture_key(
            parsed(h),
            i64::from(index),
            i64::from(colorway),
            &slots,
            &rgb,
        )
    });
    match key {
        Ok(key) => key as jlong,
        Err(fail) => {
            throw(&mut env, &fail);
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLoadAsset(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    h: jlong,
) -> jint {
    let renderer = renderer(r);
    match load_asset(renderer, i64::from(slot), parsed(h)) {
        Ok(()) => 0,
        Err(f) => renderer_fail(renderer, &f),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLoadEmote(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    slot: jint,
    props: jint,
    h: jlong,
) -> jint {
    let renderer = renderer(r);
    match load_emote(renderer, i64::from(slot), i64::from(props), parsed(h)) {
        Ok(()) => 0,
        Err(f) => renderer_fail(renderer, &f),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nFrameWearersShaped(
    env: JNIEnv,
    _: JClass,
    r: jlong,
    clock: jdouble,
    wearers: JLongArray,
    count: jint,
    section_ints: JIntArray,
    input: JFloatArray,
    space: jint,
    out: JFloatArray,
) -> jint {
    let renderer = renderer(r);
    let s = match sections(&env, &section_ints) {
        Ok(s) => s,
        Err(f) => return renderer_fail(renderer, &f),
    };
    let Ok(count) = usize::try_from(count) else {
        return fail(renderer, "counts must not be negative");
    };
    let held = length(&env, &wearers);
    if held < count {
        return fail(
            renderer,
            &format!("wearers holds {held} ids, the frame has {count} wearers"),
        );
    }
    let needed = s.stride().saturating_mul(count);
    let held = length(&env, &input);
    if held < needed {
        return fail(renderer, &FrameError::Input { held, needed }.to_string());
    }
    let layout = match output_layout(renderer, &s) {
        Ok(layout) => layout,
        Err(f) => return renderer_fail(renderer, &f),
    };
    let out_needed = layout.stride().saturating_mul(count);
    let held = length(&env, &out);
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
    let framed = frame_shaped(
        renderer,
        clock,
        as_wearers(&ids),
        &s,
        &input,
        space as u32,
        &mut output,
    );
    renderer.ids = ids;
    renderer.input = input;
    renderer.output = output;
    let bones = match framed {
        Ok(bones) => bones,
        Err(f) => return renderer_fail(renderer, &f),
    };
    if out_needed > 0
        && env
            .set_float_array_region(&out, 0, &renderer.output)
            .is_err()
    {
        return fail(renderer, "could not write the frame");
    }
    bones as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nOutputLayout(
    env: JNIEnv,
    _: JClass,
    r: jlong,
    section_ints: JIntArray,
    out: JIntArray,
) -> jint {
    let renderer = renderer(r);
    let layout = sections(&env, &section_ints).and_then(|s| output_layout(renderer, &s));
    let ints = match layout {
        Ok(layout) => layout_ints(&layout),
        Err(f) => return renderer_fail(renderer, &f),
    };
    let n = length(&env, &out).min(ints.len());
    if n > 0 && env.set_int_array_region(&out, 0, &ints[..n]).is_err() {
        return fail(renderer, "could not write the layout");
    }
    ints[LAYOUT_STRIDE]
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nInputLayout(
    mut env: JNIEnv,
    _: JClass,
    section_ints: JIntArray,
    out: JIntArray,
) -> jint {
    let ints = match sections(&env, &section_ints) {
        Ok(s) => input_layout(&s),
        Err(f) => {
            throw(&mut env, &f);
            return -1;
        }
    };
    let n = length(&env, &out).min(ints.len());
    if n > 0 && env.set_int_array_region(&out, 0, &ints[..n]).is_err() {
        throw(&mut env, &Fail::Args("could not write the layout".into()));
        return -1;
    }
    ints[INPUT_STRIDE]
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nHostVariableId(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    name: JString,
) -> jint {
    let renderer = renderer(r);
    match text(&mut env, &name) {
        Some(name) => renderer.scene.host_variable_id(&name) as jint,
        None => fail(renderer, "the variable name is not a string"),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nLocatorId(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    name: JString,
) -> jint {
    let renderer = renderer(r);
    let Some(name) = text(&mut env, &name) else {
        return fail(renderer, "the locator name is not a string");
    };
    match locator_id(renderer, &name) {
        Ok(id) => id as jint,
        Err(f) => renderer_fail(renderer, &f),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetRig(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    kind: jint,
) -> jint {
    let renderer = renderer(r);
    match set_rig(renderer, i64::from(kind)) {
        Ok(()) => 0,
        Err(f) => renderer_fail(renderer, &f),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetPhysics(
    _: JNIEnv,
    _: JClass,
    r: jlong,
    mode: jint,
    own: jlong,
) -> jint {
    let renderer = renderer(r);
    match set_physics(renderer, i64::from(mode), own as u64) {
        Ok(()) => 0,
        Err(f) => renderer_fail(renderer, &f),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nFireEffect(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    wearer: jlong,
    name: JString,
) -> jint {
    let renderer = renderer(r);
    match with_name_checked(&mut env, &name, |name| {
        renderer.scene.fire_named(wearer as u64, name)
    }) {
        Some(fired) => jint::from(fired),
        None => fail(renderer, "the effect name is not a string"),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nSetEffectColors(
    env: JNIEnv,
    _: JClass,
    r: jlong,
    wearer: jlong,
    colorway: jint,
    slots: JIntArray,
    rgb: JIntArray,
) -> jint {
    let renderer = renderer(r);
    let custom = int_vec(&env, &slots).and_then(|s| Ok((s, int_vec(&env, &rgb)?)));
    let set = custom.and_then(|(slots, rgb)| {
        set_effect_colors(renderer, wearer as u64, i64::from(colorway), &slots, &rgb)
    });
    match set {
        Ok(()) => 0,
        Err(f) => renderer_fail(renderer, &f),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nHasEffects(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) -> jboolean {
    renderer(r).scene.has_effects() as jboolean
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nEffectGeometry(
    env: JNIEnv,
    _: JClass,
    r: jlong,
    view: JFloatArray,
    out: JFloatArray,
) -> jint {
    let renderer = renderer(r);
    let held = length(&env, &view);
    if held != VIEW_FLOATS {
        return fail(
            renderer,
            &format!("view holds {held} floats, not {VIEW_FLOATS}"),
        );
    }
    let mut floats = [0.0f32; VIEW_FLOATS];
    if env.get_float_array_region(&view, 0, &mut floats).is_err() {
        return fail(renderer, "could not read the view");
    }
    let view = match view_of(&floats) {
        Ok(view) => view,
        Err(f) => return renderer_fail(renderer, &f),
    };
    if out.is_null() {
        return match effect_geometry(renderer, &view, &mut [], true) {
            Ok(quads) => quads as jint,
            Err(f) => renderer_fail(renderer, &f),
        };
    }
    let held = length(&env, &out);
    renderer.output.resize(held, 0.0);
    let mut output = mem::take(&mut renderer.output);
    let drawn = effect_geometry(renderer, &view, &mut output, false);
    renderer.output = output;
    let quads = match drawn {
        Ok(quads) => quads,
        Err(f) => return renderer_fail(renderer, &f),
    };
    let floats = quads * QUAD_FLOATS;
    if floats > 0
        && env
            .set_float_array_region(&out, 0, &renderer.output[..floats])
            .is_err()
    {
        return fail(renderer, "could not write the effect geometry");
    }
    quads as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nEffectEventFloats(
    env: JNIEnv,
    _: JClass,
    r: jlong,
    out: JFloatArray,
) -> jint {
    let renderer = renderer(r);
    let floats = renderer.scene.effect_event_floats();
    let events = floats.len() / EFFECT_EVENT_FLOATS;
    let whole = whole_events(floats, length(&env, &out));
    let written = whole.is_empty() || env.set_float_array_region(&out, 0, whole).is_ok();
    if !written {
        return fail(renderer, "could not write the effect event floats");
    }
    events as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nEffectTextureCount(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) -> jint {
    renderer(r).scene.effect_texture_count() as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nEffectTexture(
    mut env: JNIEnv,
    _: JClass,
    r: jlong,
    id: jint,
    slot: JIntArray,
) -> jstring {
    let Ok(id) = usize::try_from(id) else {
        return std::ptr::null_mut();
    };
    let Some((owner, path)) = renderer(r).scene.effect_texture(id) else {
        return std::ptr::null_mut();
    };
    if length(&env, &slot) > 0
        && env
            .set_int_array_region(&slot, 0, &[owner as jint])
            .is_err()
    {
        return std::ptr::null_mut();
    }
    cached_string(&mut env, path)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nPairSeed(
    _: JNIEnv,
    _: JClass,
    a: jlong,
    b: jlong,
    minute: jlong,
) -> jlong {
    pair_seed(a as u64, b as u64, minute) as jlong
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nActorWearer(
    _: JNIEnv,
    _: JClass,
    owner: jlong,
    slot: jint,
) -> jlong {
    actor_wearer(owner as u64, slot as u8) as jlong
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nEventWearer(
    _: JNIEnv,
    _: JClass,
    owner: jlong,
    event: jint,
    seq: jint,
) -> jlong {
    event_wearer(owner as u64, event as u8, seq as u32) as jlong
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nEventKey(
    _: JNIEnv,
    _: JClass,
    owner: jlong,
    event: jint,
    seq: jint,
    nonce: jint,
) -> jlong {
    event_key(owner as u64, event as u8, seq as u32, nonce as u32) as jlong
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Renderer_nFault(
    _: JNIEnv,
    _: JClass,
    r: jlong,
) {
    fault_next_frame(r as *const Renderer);
}
