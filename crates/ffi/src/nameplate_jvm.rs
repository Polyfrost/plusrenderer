#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

use std::sync::Arc;

use jni::objects::{
    JByteArray, JClass, JFloatArray, JIntArray, JLongArray, JPrimitiveArray, JString, TypeArray,
};
use jni::sys::{jboolean, jdouble, jfloat, jint, jlong, jstring};
use jni::JNIEnv;
use plusrenderer::nameplate::abi::{
    GLYPH_FLOATS, SIGNAL_FLOATS, TAG_COLOR_COUNT, TAG_COLOR_OFFSET, TAG_GLYPH_COUNT,
    TAG_GLYPH_OFFSET, TAG_INTS,
};

use super::nameplate::{
    add_asset, analyze_json, check_molang_json, constants_json, dialect_issues_json, dialect_of,
    finish_fallback, fire, frame, issues_json, meta_json, mode_of, needed_words, parse_asset,
    run_fallbacks, set_custom_colors, set_motion, set_signal_override, set_vanilla_texture,
    shaders_json, texture, texture_count, Fail, Fallback, Nameplates, ParsedAsset,
};

const ILLEGAL_ARGUMENT: &str = "java/lang/IllegalArgumentException";
const ILLEGAL_STATE: &str = "java/lang/IllegalStateException";

struct JvmNameplates {
    inner: Nameplates,
    wearers: Vec<u64>,
    tags: Vec<i32>,
    signals: Vec<f32>,
    glyphs: Vec<f32>,
    colors: Vec<i32>,
    out: Vec<i32>,
}

unsafe fn parsed<'a>(h: jlong) -> &'a ParsedAsset {
    &*(h as *const ParsedAsset)
}

unsafe fn plates<'a>(n: jlong) -> &'a mut JvmNameplates {
    &mut *(n as *mut JvmNameplates)
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

fn as_longs(ids: &mut [u64]) -> &mut [i64] {
    unsafe { std::slice::from_raw_parts_mut(ids.as_mut_ptr().cast(), ids.len()) }
}

fn as_ints(texels: &[u32]) -> &[i32] {
    unsafe { std::slice::from_raw_parts(texels.as_ptr().cast(), texels.len()) }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nParse(
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
    let parsed = mode_of(i64::from(mode))
        .and_then(|mode| parse_asset(&bytes, mode, capabilities.as_deref(), identity as u64));
    match parsed {
        Ok(parsed) => Arc::into_raw(Arc::new(parsed)) as jlong,
        Err(fail) => {
            throw(&mut env, &fail);
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nFree(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    drop(Arc::from_raw(h as *const ParsedAsset));
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nStatus(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) -> jint {
    parsed(h).status
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nIssuesJson(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jstring {
    string_out(&mut env, Ok(issues_json(parsed(h))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nShadersJson(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    dialect: jint,
) -> jstring {
    let json = dialect_of(i64::from(dialect)).and_then(|d| shaders_json(parsed(h), d));
    string_out(&mut env, json)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nDialectIssuesJson(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    dialect: jint,
) -> jstring {
    let json = dialect_of(i64::from(dialect)).and_then(|d| dialect_issues_json(parsed(h), d));
    string_out(&mut env, json)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nAnalyzeJson(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jstring {
    string_out(&mut env, analyze_json(parsed(h)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nMetaJson(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jstring {
    string_out(&mut env, meta_json(parsed(h)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nTextureCount(
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
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nTexture(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    index: jint,
    size: JIntArray,
    rgba: JByteArray,
) -> jint {
    let image = match texture(parsed(h), i64::from(index)) {
        Ok(image) => image,
        Err(fail) => {
            throw(&mut env, &fail);
            return -1;
        }
    };
    let bytes = image.rgba.len();
    if !rgba.is_null() {
        let held = length(&env, &rgba);
        if held < bytes {
            let message = format!("rgba holds {held} bytes, the texture needs {bytes}");
            throw(&mut env, &Fail::Args(message));
            return -1;
        }
    }
    if !size.is_null() {
        let held = length(&env, &size);
        if held < 2 {
            throw(
                &mut env,
                &Fail::Args(format!("size holds {held} ints, not 2")),
            );
            return -1;
        }
    }
    let dims = [image.width as jint, image.height as jint];
    let sized = size.is_null() || env.set_int_array_region(&size, 0, &dims).is_ok();
    let signed: &[i8] = std::slice::from_raw_parts(image.rgba.as_ptr().cast(), bytes);
    let copied = rgba.is_null() || env.set_byte_array_region(&rgba, 0, signed).is_ok();
    if !(sized && copied) {
        throw(&mut env, &Fail::Args("could not write the texture".into()));
        return -1;
    }
    bytes as jint
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nConstantsJson(
    mut env: JNIEnv,
    _: JClass,
) -> jstring {
    string_out(&mut env, Ok(constants_json()))
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_NameplateAsset_nCheckMolangJson(
    mut env: JNIEnv,
    _: JClass,
    source: JString,
    context: jint,
) -> jstring {
    let Some(source) = text(&mut env, &source) else {
        throw(&mut env, &Fail::Args("source is not a string".into()));
        return std::ptr::null_mut();
    };
    let json = check_molang_json(&source, i64::from(context));
    string_out(&mut env, json)
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nCreate(
    _: JNIEnv,
    _: JClass,
) -> jlong {
    Box::into_raw(Box::new(JvmNameplates {
        inner: Nameplates::new(),
        wearers: Vec::new(),
        tags: Vec::new(),
        signals: Vec::new(),
        glyphs: Vec::new(),
        colors: Vec::new(),
        out: Vec::new(),
    })) as jlong
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nFree(
    _: JNIEnv,
    _: JClass,
    n: jlong,
) {
    drop(Box::from_raw(n as *mut JvmNameplates));
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nError(
    env: JNIEnv,
    _: JClass,
    n: jlong,
) -> jstring {
    env.new_string(&plates(n).inner.error)
        .map_or(std::ptr::null_mut(), |s| s.into_raw())
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nAddAsset(
    _: JNIEnv,
    _: JClass,
    n: jlong,
    h: jlong,
) -> jint {
    let n = &mut plates(n).inner;
    match add_asset(n, parsed(h)) {
        Ok(id) => id as jint,
        Err(fail) => n.fail(&fail),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nRemoveAsset(
    _: JNIEnv,
    _: JClass,
    n: jlong,
    id: jint,
) {
    if let Ok(id) = u32::try_from(id) {
        plates(n).inner.renderer.remove_asset(id);
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nSetMotion(
    _: JNIEnv,
    _: JClass,
    n: jlong,
    motion: jint,
) -> jint {
    let n = &mut plates(n).inner;
    match set_motion(n, i64::from(motion)) {
        Ok(()) => 0,
        Err(fail) => n.fail(&fail),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nSetVanillaTexture(
    mut env: JNIEnv,
    _: JClass,
    n: jlong,
    id: JString,
    width: jint,
    height: jint,
    rgba: JByteArray,
) -> jint {
    let n = &mut plates(n).inner;
    let Some(id) = text(&mut env, &id) else {
        return n.fail(&Fail::Args("texture id is not a string".into()));
    };
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return n.fail(&Fail::Args(format!(
            "a {width}x{height} texture has a negative side"
        )));
    };
    let rgba = if rgba.is_null() {
        Vec::new()
    } else {
        match env.convert_byte_array(&rgba) {
            Ok(rgba) => rgba,
            Err(_) => return n.fail(&Fail::Args("could not read the texture".into())),
        }
    };
    match set_vanilla_texture(n, &id, width, height, &rgba) {
        Ok(()) => 0,
        Err(fail) => n.fail(&fail),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nNeededWords(
    env: JNIEnv,
    _: JClass,
    n: jlong,
    tags: JIntArray,
    count: jint,
) -> jint {
    let jn = plates(n);
    let Some(ints) = rows(count, TAG_INTS) else {
        return jn.inner.fail(&Fail::Args(format!("no tag count {count}")));
    };
    let held = length(&env, &tags);
    if held < ints {
        return jn.inner.fail(&Fail::Args(format!(
            "tags holds {held} ints, {count} tags need {ints}"
        )));
    }
    jn.tags.resize(ints, 0);
    if ints > 0 && env.get_int_array_region(&tags, 0, &mut jn.tags).is_err() {
        return jn.inner.fail(&Fail::Args("could not read the tags".into()));
    }
    match needed_words(&jn.inner, &jn.tags) {
        Ok(words) => words as jint,
        Err(fail) => jn.inner.fail(&fail),
    }
}

fn used_extent(tags: &[i32], offset_at: usize, count_at: usize, width: usize, held: usize) -> usize {
    let mut used = 0;
    for row in tags.chunks_exact(TAG_INTS) {
        let (Ok(offset), Ok(count)) = (
            usize::try_from(row[offset_at]),
            usize::try_from(row[count_at]),
        ) else {
            continue;
        };
        let end = offset
            .checked_mul(width)
            .zip(count.checked_mul(width))
            .and_then(|(start, len)| start.checked_add(len));
        if let Some(end) = end.filter(|&end| end <= held) {
            used = used.max(end);
        }
    }
    used
}

fn rows(count: jint, stride: usize) -> Option<usize> {
    usize::try_from(count).ok()?.checked_mul(stride)
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nFrame(
    env: JNIEnv,
    _: JClass,
    n: jlong,
    clock: jdouble,
    wearers: JLongArray,
    count: jint,
    tags: JIntArray,
    signals: JFloatArray,
    glyphs: JFloatArray,
    colors: JIntArray,
    out: JIntArray,
) -> jint {
    let jn = plates(n);
    let (Some(ints), Some(floats)) = (rows(count, TAG_INTS), rows(count, SIGNAL_FLOATS)) else {
        return jn.inner.fail(&Fail::Args(format!("no tag count {count}")));
    };
    let count = count as usize;
    let held = length(&env, &wearers);
    if held < count {
        return jn.inner.fail(&Fail::Args(format!(
            "wearers holds {held} ids, the frame has {count} tags"
        )));
    }
    let held = length(&env, &tags);
    if held < ints {
        return jn.inner.fail(&Fail::Args(format!(
            "tags holds {held} ints, {count} tags need {ints}"
        )));
    }
    let held = length(&env, &signals);
    if held < floats {
        return jn.inner.fail(&Fail::Args(format!(
            "signals holds {held} floats, {count} tags need {floats}"
        )));
    }
    if out.is_null() {
        return jn.inner.fail(&Fail::Args("out is null".into()));
    }
    let (glyph_held, color_held, out_len) = (
        length(&env, &glyphs),
        length(&env, &colors),
        length(&env, &out),
    );

    jn.wearers.resize(count, 0);
    jn.tags.resize(ints, 0);
    jn.signals.resize(floats, 0.0);
    jn.out.resize(out_len, 0);
    let rows_read = count == 0
        || (env
            .get_long_array_region(&wearers, 0, as_longs(&mut jn.wearers))
            .is_ok()
            && env.get_int_array_region(&tags, 0, &mut jn.tags).is_ok()
            && env
                .get_float_array_region(&signals, 0, &mut jn.signals)
                .is_ok());
    if !rows_read {
        return jn
            .inner
            .fail(&Fail::Args("could not read the frame input".into()));
    }

    let glyph_len = used_extent(
        &jn.tags,
        TAG_GLYPH_OFFSET,
        TAG_GLYPH_COUNT,
        GLYPH_FLOATS,
        glyph_held,
    );
    let color_len = used_extent(&jn.tags, TAG_COLOR_OFFSET, TAG_COLOR_COUNT, 1, color_held);
    jn.glyphs.resize(glyph_len, 0.0);
    jn.colors.resize(color_len, 0);
    let read = (glyph_len == 0
        || env
            .get_float_array_region(&glyphs, 0, &mut jn.glyphs)
            .is_ok())
        && (color_len == 0 || env.get_int_array_region(&colors, 0, &mut jn.colors).is_ok());
    if !read {
        return jn
            .inner
            .fail(&Fail::Args("could not read the frame input".into()));
    }

    let framed = frame(
        &mut jn.inner,
        clock,
        &jn.wearers,
        &jn.tags,
        &jn.signals,
        &jn.glyphs,
        &jn.colors,
        &mut jn.out,
    );
    let words = match framed {
        Ok(words) => words,
        Err(fail) => return jn.inner.fail(&fail),
    };
    if env.set_int_array_region(&out, 0, &jn.out[..words]).is_err() {
        return jn
            .inner
            .fail(&Fail::Args("could not write the frame".into()));
    }
    words as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nFault(
    _: JNIEnv,
    _: JClass,
    n: jlong,
) {
    plates(n).inner.fault = true;
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nForgetWearer(
    _: JNIEnv,
    _: JClass,
    n: jlong,
    wearer: jlong,
) -> jboolean {
    plates(n).inner.renderer.forget_wearer(wearer as u64) as jboolean
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nSetSignalOverride(
    _: JNIEnv,
    _: JClass,
    n: jlong,
    wearer: jlong,
    signal: jint,
    value: jfloat,
) -> jint {
    let n = &mut plates(n).inner;
    match set_signal_override(n, wearer as u64, i64::from(signal), value) {
        Ok(()) => 0,
        Err(fail) => n.fail(&fail),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nSetCustomColors(
    env: JNIEnv,
    _: JClass,
    n: jlong,
    wearer: jlong,
    asset: jint,
    slots: JIntArray,
    rgb: JIntArray,
) -> jint {
    let n = &mut plates(n).inner;
    let custom = int_vec(&env, &slots).and_then(|s| Ok((s, int_vec(&env, &rgb)?)));
    let set = custom.and_then(|(slots, rgb)| {
        set_custom_colors(n, wearer as u64, i64::from(asset), &slots, &rgb)
    });
    match set {
        Ok(()) => 0,
        Err(fail) => n.fail(&fail),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nClearCustomColors(
    _: JNIEnv,
    _: JClass,
    n: jlong,
    wearer: jlong,
    asset: jint,
) -> jboolean {
    let cleared = u32::try_from(asset).is_ok_and(|a| {
        plates(n)
            .inner
            .renderer
            .clear_custom_colors(wearer as u64, a)
    });
    cleared as jboolean
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nFire(
    _: JNIEnv,
    _: JClass,
    n: jlong,
    wearer: jlong,
    asset: jint,
    one_shot: jint,
) -> jint {
    let n = &mut plates(n).inner;
    match fire(n, wearer as u64, i64::from(asset), i64::from(one_shot)) {
        Ok(fired) => fired as jint,
        Err(fail) => n.fail(&fail),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nStripLayout(
    _: JNIEnv,
    _: JClass,
    n: jlong,
) -> jint {
    plates(n).inner.renderer.strip_layout() as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nStripTexels(
    env: JNIEnv,
    _: JClass,
    n: jlong,
    id: jint,
    out: JIntArray,
) -> jint {
    let n = &mut plates(n).inner;
    let strip = u32::try_from(id).ok();
    let count = strip.map_or(0, |id| n.renderer.strip_texels(id).len());
    if out.is_null() || count == 0 {
        return count as jint;
    }
    let held = length(&env, &out);
    if held < count {
        return n.fail(&Fail::Args(format!(
            "out holds {held} texels, strip {id} has {count}"
        )));
    }
    let texels = strip.map_or(&[][..], |id| n.renderer.strip_texels(id));
    if env.set_int_array_region(&out, 0, as_ints(texels)).is_err() {
        return n.fail(&Fail::Args("could not write the texels".into()));
    }
    count as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nWearerCount(
    _: JNIEnv,
    _: JClass,
    n: jlong,
) -> jint {
    plates(n).inner.renderer.wearer_count() as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nWearerBytes(
    _: JNIEnv,
    _: JClass,
    n: jlong,
) -> jint {
    plates(n).inner.renderer.wearer_bytes() as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nPendingFallbacks(
    _: JNIEnv,
    _: JClass,
    n: jlong,
) -> jint {
    plates(n).inner.renderer.pending_fallbacks() as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nRunFallbacks(
    _: JNIEnv,
    _: JClass,
    n: jlong,
    limit: jint,
) -> jint {
    let n = &mut plates(n).inner;
    match run_fallbacks(n, i64::from(limit)) {
        Ok(left) => left as jint,
        Err(fail) => n.fail(&fail),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nTakeFallback(
    _: JNIEnv,
    _: JClass,
    n: jlong,
) -> jlong {
    match plates(n).inner.renderer.take_fallback() {
        Some(job) => Box::into_raw(Box::new(Fallback::Job(Box::new(job)))) as jlong,
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nRunFallback(
    _: JNIEnv,
    _: JClass,
    f: jlong,
) -> jboolean {
    if f == 0 {
        return 0;
    }
    (&mut *(f as *mut Fallback)).run() as jboolean
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nFinishFallback(
    _: JNIEnv,
    _: JClass,
    n: jlong,
    f: jlong,
) -> jint {
    if f == 0 {
        return 0;
    }
    let fallback = *Box::from_raw(f as *mut Fallback);
    let n = &mut plates(n).inner;
    match finish_fallback(n, fallback) {
        Ok(stored) => stored as jint,
        Err(fail) => n.fail(&fail),
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_Nameplates_nFreeFallback(
    _: JNIEnv,
    _: JClass,
    f: jlong,
) {
    if f != 0 {
        drop(Box::from_raw(f as *mut Fallback));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_used_extent_is_the_furthest_range_that_fits() {
        let rows = |tags: &[(i32, i32)]| -> Vec<i32> {
            let mut out = vec![0; tags.len() * TAG_INTS];
            for (row, &(offset, count)) in out.chunks_exact_mut(TAG_INTS).zip(tags) {
                (row[TAG_GLYPH_OFFSET], row[TAG_GLYPH_COUNT]) = (offset, count);
            }
            out
        };
        let extent = |tags: &[i32], held| {
            used_extent(tags, TAG_GLYPH_OFFSET, TAG_GLYPH_COUNT, GLYPH_FLOATS, held)
        };
        let tags = rows(&[(0, 2), (5, 3), (1, 1)]);
        assert_eq!(
            [1000, 16, 15, 0].map(|held| extent(&tags, held)),
            [16, 16, 4, 0]
        );
        let broken = rows(&[(-1, 2), (2, -1), (i32::MAX, i32::MAX), (1, 1)]);
        assert_eq!(extent(&broken, 1000), 4);
    }
}
