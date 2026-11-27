#![allow(non_snake_case, clippy::missing_safety_doc)]

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use jni::objects::{JBooleanArray, JClass, JFloatArray, JIntArray, JString};
use jni::sys::{jboolean, jdouble, jfloat, jint, jlong, jstring};
use jni::JNIEnv;
use plusrenderer::molang::{EntityRuntime, Env, Rules, WarningKind};

type SharedEnv = Arc<RwLock<Env>>;

struct Runtime {
    env: SharedEnv,
    runtime: EntityRuntime,
    ids: Vec<u32>,
    queries: Vec<f32>,
    out: Vec<f32>,
    failed: Vec<bool>,
    flags: Vec<jboolean>,
}

const ILLEGAL_ARGUMENT: &str = "java/lang/IllegalArgumentException";
const ILLEGAL_STATE: &str = "java/lang/IllegalStateException";

fn read(env: &SharedEnv) -> RwLockReadGuard<'_, Env> {
    env.read().unwrap_or_else(PoisonError::into_inner)
}

fn write(env: &SharedEnv) -> RwLockWriteGuard<'_, Env> {
    env.write().unwrap_or_else(PoisonError::into_inner)
}

unsafe fn shared<'a>(env: jlong) -> &'a SharedEnv {
    &*(env as *const SharedEnv)
}

unsafe fn entity<'a>(runtime: jlong) -> &'a mut Runtime {
    &mut *(runtime as *mut Runtime)
}

fn throw(env: &mut JNIEnv, class: &str, message: &str) {
    if !env.exception_check().unwrap_or(true) {
        let _ = env.throw_new(class, message);
    }
}

fn guarded<T>(env: &mut JNIEnv, failed: T, f: impl FnOnce(&mut JNIEnv) -> T) -> T {
    match catch_unwind(AssertUnwindSafe(|| f(env))) {
        Ok(value) => value,
        Err(_) => {
            throw(env, ILLEGAL_STATE, "the MoLang core failed");
            failed
        }
    }
}

fn text(env: &mut JNIEnv, s: &JString) -> Option<String> {
    env.get_string(s).map(Into::into).ok()
}

fn id(slot: Option<u16>) -> jint {
    slot.map_or(-1, jint::from)
}

fn variable_slot(id: jint) -> Option<u16> {
    u16::try_from(id).ok()
}

fn as_jints(ids: &mut [u32]) -> &mut [jint] {
    unsafe { std::slice::from_raw_parts_mut(ids.as_mut_ptr().cast(), ids.len()) }
}

fn warning_kind(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::UnknownFunction => "unknown function",
        WarningKind::UnknownQuery => "unknown query",
        WarningKind::UnknownName => "unknown name",
        WarningKind::UnknownRoot => "unknown root",
        WarningKind::Arity => "wrong argument count",
    }
}

fn warning_lines(env: &Env) -> String {
    let lines: Vec<String> = env
        .warnings()
        .as_slice()
        .iter()
        .map(|w| format!("{}: {}", warning_kind(w.kind), w.name))
        .collect();
    lines.join("\n")
}

#[no_mangle]
pub extern "system" fn Java_org_polyfrost_plusrenderer_MolangEnv_nCreate(
    _: JNIEnv,
    _: JClass,
    strict: jboolean,
) -> jlong {
    let rules = if strict != 0 {
        Rules::Strict
    } else {
        Rules::Bedrock
    };
    let env: SharedEnv = Arc::new(RwLock::new(Env::new(rules)));
    Box::into_raw(Box::new(env)) as jlong
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangEnv_nFree(
    _: JNIEnv,
    _: JClass,
    env: jlong,
) {
    drop(Box::from_raw(env as *mut SharedEnv));
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangEnv_nCompile(
    mut jni: JNIEnv,
    _: JClass,
    env: jlong,
    source: JString,
) -> jint {
    let Some(source) = text(&mut jni, &source) else {
        throw(&mut jni, ILLEGAL_ARGUMENT, "source is not a string");
        return -1;
    };
    let env = shared(env);
    let compiled = guarded(&mut jni, None, |_| Some(write(env).compile(&source)));
    match compiled {
        Some(Ok(id)) => id as jint,
        Some(Err(e)) => {
            throw(&mut jni, ILLEGAL_ARGUMENT, &e.to_string());
            -1
        }
        None => -1,
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangEnv_nQueryId(
    mut jni: JNIEnv,
    _: JClass,
    env: jlong,
    name: JString,
) -> jint {
    let name = text(&mut jni, &name).unwrap_or_default();
    id(read(shared(env)).query_id(&name))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangEnv_nVariableId(
    mut jni: JNIEnv,
    _: JClass,
    env: jlong,
    name: JString,
) -> jint {
    let name = text(&mut jni, &name).unwrap_or_default();
    id(read(shared(env)).variable_id(&name))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangEnv_nQueryCount(
    _: JNIEnv,
    _: JClass,
    env: jlong,
) -> jint {
    read(shared(env)).query_count() as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangEnv_nVariableCount(
    _: JNIEnv,
    _: JClass,
    env: jlong,
) -> jint {
    read(shared(env)).variable_count() as jint
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangEnv_nWarnings(
    jni: JNIEnv,
    _: JClass,
    env: jlong,
) -> jstring {
    let lines = warning_lines(&read(shared(env)));
    jni.new_string(lines)
        .map_or(std::ptr::null_mut(), |s| s.into_raw())
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangRuntime_nCreate(
    _: JNIEnv,
    _: JClass,
    env: jlong,
    seed: jlong,
) -> jlong {
    let env = shared(env).clone();
    let runtime = EntityRuntime::new(&read(&env), seed as u64);
    Box::into_raw(Box::new(Runtime {
        env,
        runtime,
        ids: Vec::new(),
        queries: Vec::new(),
        out: Vec::new(),
        failed: Vec::new(),
        flags: Vec::new(),
    })) as jlong
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangRuntime_nFree(
    _: JNIEnv,
    _: JClass,
    runtime: jlong,
) {
    drop(Box::from_raw(runtime as *mut Runtime));
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangRuntime_nInitialize(
    mut jni: JNIEnv,
    _: JClass,
    runtime: jlong,
    script_ids: JIntArray,
    count: jint,
) -> jint {
    let rt = entity(runtime);
    rt.ids.resize(count.max(0) as usize, 0);
    if jni
        .get_int_array_region(&script_ids, 0, as_jints(&mut rt.ids))
        .is_err()
    {
        return -1;
    }
    guarded(&mut jni, -1, |_| {
        let env = read(&rt.env);
        rt.runtime.initialize(&env, &rt.ids) as jint
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangRuntime_nEvaluate(
    mut jni: JNIEnv,
    _: JClass,
    runtime: jlong,
    expr_ids: JIntArray,
    expr_count: jint,
    queries: JFloatArray,
    clock: jdouble,
    out: JFloatArray,
    failed: JBooleanArray,
) -> jint {
    let rt = entity(runtime);
    let count = expr_count.max(0) as usize;

    rt.ids.resize(count, 0);
    if jni
        .get_int_array_region(&expr_ids, 0, as_jints(&mut rt.ids))
        .is_err()
    {
        return -1;
    }

    let query_count = if queries.is_null() {
        0
    } else {
        jni.get_array_length(&queries).unwrap_or(0).max(0) as usize
    };
    rt.queries.resize(query_count, 0.0);
    if query_count > 0
        && jni
            .get_float_array_region(&queries, 0, &mut rt.queries)
            .is_err()
    {
        return -1;
    }

    rt.out.resize(count, 0.0);
    rt.failed.resize(count, false);
    let failures = guarded(&mut jni, -1, |_| {
        let env = read(&rt.env);
        rt.runtime.evaluate(
            &env,
            &rt.ids,
            &rt.queries,
            clock,
            &mut rt.out,
            &mut rt.failed,
        ) as jint
    });
    if failures < 0 {
        return failures;
    }

    if jni.set_float_array_region(&out, 0, &rt.out).is_err() {
        return -1;
    }
    if !failed.is_null() {
        rt.flags.clear();
        rt.flags.extend(rt.failed.iter().map(|&f| f as jboolean));
        if jni.set_boolean_array_region(&failed, 0, &rt.flags).is_err() {
            return -1;
        }
    }
    failures
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangRuntime_nVariable(
    _: JNIEnv,
    _: JClass,
    runtime: jlong,
    id: jint,
) -> jfloat {
    variable_slot(id).map_or(0.0, |slot| entity(runtime).runtime.variable(slot))
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangRuntime_nSetVariable(
    _: JNIEnv,
    _: JClass,
    runtime: jlong,
    id: jint,
    value: jfloat,
) {
    if let Some(slot) = variable_slot(id) {
        let rt = entity(runtime);
        rt.runtime.fit(&read(&rt.env));
        rt.runtime.set_variable(slot, value);
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_org_polyfrost_plusrenderer_MolangRuntime_nReset(
    _: JNIEnv,
    _: JClass,
    runtime: jlong,
) {
    entity(runtime).runtime.reset();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_ids_and_int_arrays_cross_the_jni_shape() {
        let mut env = Env::new(Rules::Bedrock);
        env.compile("math.nope(1) + math.nope(2)").unwrap();
        env.compile("math.clamp(1)").unwrap();
        let lines = warning_lines(&env);
        assert_eq!(lines.lines().count(), env.warnings().len());
        assert!(
            lines.starts_with("unknown function: ") && lines.contains("nope"),
            "{lines}"
        );
        let slots = [-1, 70_000, 3].map(variable_slot);
        assert_eq!(
            (slots, id(None), id(Some(7))),
            ([None, None, Some(3)], -1, 7)
        );
        let mut ids = vec![0u32; 2];
        as_jints(&mut ids).copy_from_slice(&[5, -1]);
        assert_eq!(ids, [5, u32::MAX]);
    }
}
