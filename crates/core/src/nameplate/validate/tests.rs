use serde_json::{json, Value};

use crate::issues::{Issue, IssueCode, Mode, Report};
use crate::signals::signal_id;

use super::super::anim::schedule::roll;
use super::super::format::tests::{testdata, Kit};
use super::super::geom::Rect;
use super::super::raster::Canvas;
use super::super::types::NameplateAsset;
use super::super::{analyze, load, LoadOptions, Loaded, Source};
use super::budget::measure;
use super::flash::{
    check as flash_check, placements, planned, ranked, scenarios, simulate, spend, Luminance,
    Tracker, SCALE, SCENARIO_FRAMES, SCENARIO_ROWS,
};
use super::sim::{Scenario, Source as From, Stage};
use super::validate;

fn kit(script: &str) -> Kit {
    let (header, script) = script.split_once(" & ").unwrap_or((script, ""));
    let (fps, frames) = header.split_once('/').unwrap();
    let manifest = json!({ "format": 1, "kind": "nameplate", "id": "plate", "fps": fps.parse::<u32>().unwrap(), "loop": frames.parse::<u32>().unwrap(),
        "text": { "color": "ink" }, "layers": [] });
    let palettes = json!({ "slots": [{ "id": "ink", "ramp": ["#FFFFFF"] }, { "id": "fill", "ramp": ["#202020"] },
        { "id": "lite", "ramp": ["#FFFFFF"] }, { "id": "dark", "ramp": ["#000000"] }] });
    let mut kit = Kit::new(manifest, palettes);
    for step in script.split(" & ") {
        let parts: Vec<&str> = step.split(':').collect();
        match parts[..] {
            ["body", color] => kit.edit(&format!(
                "+body:background {{'kind': 'body', 'fill': {{'color': '{color}'}}}}"
            )),
            ["solid", id, size, frames] => {
                let size: usize = size.parse().unwrap();
                let frames: Vec<Vec<String>> = frames
                    .chars()
                    .map(|c| vec![c.to_string().repeat(size); size])
                    .collect();
                kit.edit(&format!(
                    "sprites/{id}.json {}",
                    json!({ "colors": { "w": "lite", "k": "dark" }, "frames": frames })
                ))
            }
            _ => kit.edit(step),
        };
    }
    kit
}

fn plate(script: &str) -> NameplateAsset {
    kit(script).asset()
}

fn check(asset: &NameplateAsset, mode: Mode) -> Vec<Issue> {
    let mut report = Report::new(mode);
    validate(asset, &mut report);
    report.into_issues()
}

fn value(issue: &Issue, key: &str) -> f64 {
    issue
        .values
        .iter()
        .find(|(k, _)| k == key)
        .map_or(f64::NAN, |(_, v)| *v)
}

trait Shows {
    fn shows(&self, spec: &str);
}

impl Shows for NameplateAsset {
    fn shows(&self, spec: &str) {
        let (mode, rest) = spec
            .strip_prefix("client ")
            .map_or((Mode::Authoring, spec), |s| (Mode::Client, s));
        let issues = check(self, mode);
        let mut words = rest.split(' ');
        let code = words.next().unwrap();
        let codes: Vec<&str> = issues.iter().map(|i| i.code.as_str()).collect();
        if let Some(code) = code.strip_prefix('!') {
            return assert!(!codes.contains(&code), "{spec}: {codes:?}");
        }
        if code == "clean" {
            return assert!(issues.is_empty(), "{spec}: {codes:?}");
        }
        let issue = issues
            .iter()
            .find(|i| i.code.as_str() == code)
            .unwrap_or_else(|| panic!("{spec}: {codes:?}"));
        for word in words {
            let at = word.find(['=', '<', '>']).unwrap();
            let (key, op, expected) = (&word[..at], &word[at..at + 1], &word[at + 1..]);
            let actual = match key {
                "path" => issue.path.clone(),
                "level" => format!("{:?}", issue.level).to_lowercase(),
                _ => issue
                    .names
                    .iter()
                    .find(|(k, _)| k == key)
                    .map_or_else(|| value(issue, key).to_string(), |(_, v)| v.clone()),
            };
            let (a, e) = (
                actual.parse().unwrap_or(f64::NAN),
                expected.parse().unwrap_or(f64::NAN),
            );
            let holds = [a < e, a > e, actual == expected || a == e]
                [(op != "<") as usize + (op == "=") as usize];
            assert!(holds, "{spec}: {key} is {actual} in {issue:?}");
        }
    }
}

const GEM: &str = "solid:gem:3:w & +gem:over_text {'kind': 'sprite', 'sprite': 'gem', 'anchor': 'top', 'pivot': 'bottom'}";

#[test]
fn budgets_over_their_limits_are_errors() {
    let checker = |size: usize, frames: usize| {
        let row = |y: usize| {
            (0..size)
                .map(|x| if (x + y) % 2 == 0 { 'o' } else { '.' })
                .collect::<String>()
        };
        json!({ "colors": { "o": "lite" }, "frames": vec![(0..size).map(row).collect::<Vec<_>>(); frames] })
    };
    let repeated = |max: u32| {
        plate(&format!("6/12 & sprites/row.json {} & +row:over_background {{'kind': 'sprite', 'sprite': 'row', 'anchor': 'top', 'repeat': {{'edge': 'top', 'spacing': 1, 'max': {max}}}}}", checker(8, 1)))
    };
    repeated(64).shows("client quads_over_budget path=manifest.json quads=2048 limit=800");
    repeated(2).shows("!quads_over_budget");
    assert_eq!(analyze(&repeated(2)).quads, 64);

    let walker = "{'kind': 'sprite', 'sprite': 'dot', 'path': {'preset': 'top', 'speed': 1, 'walkers': {'count': 32, 'even': true}}}";
    let walkers = |n: usize| {
        plate(&(0..n).fold(
            "6/12 & sprites/dot.json {'colors': {'o': 'lite'}, 'rows': ['o']}".to_string(),
            |s, i| format!("{s} & +w{i}:over_background {walker}"),
        ))
    };
    walkers(7).shows("client over_cpu units=4480 paths=4480 limit=4000");
    walkers(6).shows("!over_cpu");

    let big = |frames: usize| {
        plate(&format!("6/12 & sprites/big.json {} & +big:over_background {{'kind': 'sprite', 'sprite': 'big', 'anchor': 'top'}}", checker(64, frames)))
    };
    big(12).shows(&format!(
        "client over_cache bytes={}",
        12 * (64 + 20 * 2048)
    ));
    big(1).shows("client !over_cache");

    let masked = |frames: usize| {
        let mask = "w".repeat(frames);
        plate(&format!("6/120 & solid:b:1:{mask} & +b:background {{'kind': 'sprite', 'sprite': 'b'}} & solid:a:1:wwwwwwwwwwww \
            & +a:over_background {{'kind': 'sprite', 'sprite': 'a'}} & manifest.json/layers/1/mask 'b'"))
    };
    masked(12).shows("client over_states path=layers/a.json states=144");
    masked(10).shows("client !over_states");
}

fn lifted(offset: i32) -> NameplateAsset {
    plate(&format!(
        "6/12 & {GEM} & layers/gem.json/offset [0, {offset}]"
    ))
}

fn signal_lift(expr: &str) -> NameplateAsset {
    plate(&format!(
        "6/12 & {GEM} & manifest.json/animations/loop/tracks/gem/position [[0, [0, '{expr}']]]"
    ))
}

#[test]
fn margins_follow_offsets_tracks_signals_and_paths() {
    lifted(-60).shows("client over_margin path=layers/gem.json px=62");
    lifted(-20).shows("!over_margin");
    assert_eq!(analyze(&lifted(-20)).layers[0].margin, 22);
    assert_eq!(analyze(&lifted(-60)).layers[0].margin, 62);
    assert_eq!(analyze(&plate("6/12 & body:fill")).layers[0].margin, 0);

    let moving = plate(&format!("6/12 & {GEM} & body:fill & manifest.json/ownsBackground true & manifest.json/animations/loop/tracks \
        {{'gem': {{'position': [[0, [0, 0]], [6, [0, -40]]]}}, '$plate': {{'position': [[0, [0, 0]], [6, [40, 0]]]}}}}"));
    let paths: Vec<String> = check(&moving, Mode::Client)
        .into_iter()
        .filter(|i| i.code == IssueCode::OverMargin)
        .map(|i| i.path)
        .collect();
    assert!(paths.contains(&"layers/gem.json".to_string()));
    assert!(paths.contains(&"manifest.json#animations.loop.tracks.$plate".to_string()));

    for expr in [
        "q.is_sneaking * -64",
        "q.ground_speed > 5 ? -64 : 0",
        "q.is_hurt_time > 1 ? -64 : 0",
        "q.ground_speed > 4 && q.time_of_day > 0.9 ? -64 : 0",
        "q.biome_temperature > 1.5 && q.vertical_speed < -50 && q.is_hurt + q.is_sneaking >= 0 ? -64 : 0",
    ] {
        signal_lift(expr).shows("client over_margin");
    }
    signal_lift("q.is_sneaking * -8").shows("!over_margin");

    let climber = |end: &str| {
        plate(&format!("12/12 & body:dark & solid:gem:4:w & +gem:over_text {{'kind': 'sprite', 'sprite': 'gem', 'path': \
        {{'points': [{{'anchor': 'top_left'}}, {{'anchor': 'top_left', 'offset': [0, -60]}}], 'speed': 0.25, 'end': '{end}'}}}}"))
    };
    climber("wrap").shows("over_margin");
    climber("stop").shows("!over_margin");

    let flag = |frames: &str| {
        let sprite = json!({ "colors": { "w": "lite" }, "frames": [vec![format!("w{}", ".".repeat(7)); 8]] });
        plate(&format!("12/12 & body:dark & sprites/flag.json {sprite} & +flag:over_text {{'kind': 'sprite', 'sprite': 'flag', 'path': \
            {{'points': [{{'anchor': 'right'}}, {{'anchor': 'right', 'offset': [28, 0]}}], 'end': 'wrap'{frames}}}}}"))
    };
    flag("").shows("!over_margin");
    flag(", 'frames': {'right': [0], 'left': [0]}").shows("!over_margin");
    flag(", 'frames': {'right': [0]}").shows("over_margin px=36");
}

fn halo(frames: u32) -> NameplateAsset {
    plate(&format!("6/{frames} & materials/glow.json {{'resolution': 'pixel', 'margins': [32, 32, 32, 32], 'nodes': [{{'id': 'f', 'type': 'Frame'}}, \
        {{'id': 'm', 'type': 'mod', 'in': {{'a': 'f', 'b': {frames}}}}}, {{'id': 'd', 'type': 'div', 'in': {{'a': 'm', 'b': {frames}}}}}, \
        {{'id': 'h', 'type': 'Hsv', 'in': {{'h': 'd'}}}}, {{'id': 'out', 'type': 'Output', 'in': {{'color': 'h'}}}}]}} & +glow:behind {{'kind': 'material', 'material': 'glow'}}"))
}

#[test]
fn effect_pictures_are_measured_and_fall_back_to_a_still_frame() {
    let fits = halo(6);
    let frame = (417 + 64) * (30 + 64) * 4 + 64;
    fits.shows(&format!(
        "fallback_cache level=info frames=6 frame_bytes={frame}"
    ));
    fits.shows("!fallback_still");
    let analysis = analyze(&fits);
    assert_eq!(analysis.fallback_bytes, 6 * frame);
    let pixels = u64::from((417 + 64) * (30 + 64) as u32);
    let actual = &analysis.material_build_ops;
    assert_eq!(*actual, pixels * u64::from(fits.materials[0].cost) * 6);
    halo(24).shows("fallback_still");
    halo(24).shows("!over_fallback");
    halo(24).shows("client clean");
    let plain = analyze(&plate("6/12 & body:fill"));
    assert!(plain.material_build_ops == 0 && plain.effects.is_empty());
}

fn strobe(tones: usize) -> NameplateAsset {
    let ramp = json!(std::iter::once("#FFFFFF")
        .chain(std::iter::repeat("#000000").take(tones - 1))
        .collect::<Vec<_>>());
    plate(&format!("12/12 & palettes.json/slots/- {{'id': 'strobe', 'ramp': {ramp}}} & body:strobe & manifest.json/animations/loop/tracks/palette.strobe {{'cycle': {{'every': 1, 'mode': 'forward'}}}}"))
}

fn gated(visible: &str) -> NameplateAsset {
    plate(&format!(
        "12/12 & body:lite & manifest.json/animations/loop/tracks/body/visible [[0, '{visible}']]"
    ))
}

fn event(trigger: &str, visible: &str) -> NameplateAsset {
    plate(&format!("12/12 & body:lite & manifest.json/animations/pop {{'trigger': {trigger}, 'tracks': {{'body': {{'visible': [[0, '{visible}']]}}}}}}"))
}

#[test]
fn flashes_are_caught_in_every_signal_state_and_named() {
    strobe(3).shows(
        "flash level=error path=layers/body.json layer=body animation=loop px>64 flashes>3.5",
    );
    strobe(4).shows("!flash");
    plate("6/12 & body:lite").shows("client clean");
    plate("6/12 & body:lite").shows("readability");
    let keys: Vec<Value> = (0..12)
        .map(|f| json!([f, if f % 2 == 0 { "#FFFFFF" } else { "#000000" }]))
        .collect();
    plate(&format!("12/12 & palettes.json/slots/- {{'id': 'strobe', 'ramp': ['#202020']}} & body:strobe & manifest.json/animations/panic \
        {{'trigger': {{'reaction': 'is_hurt', 'on': 'while'}}, 'tracks': {{'palette.strobe': {{'keys': {}}}}}}}", json!(keys)))
    .shows("flash path=manifest.json#animations.panic animation=panic layer=body");

    for visible in [
        "q.is_sneaking && math.mod(q.frame, 2) == 0",
        "q.ground_speed > 5 && math.mod(q.frame, 2) == 0",
        "q.is_hurt_time > 1 && math.mod(q.frame, 2) == 0",
    ] {
        gated(visible).shows("flash path=layers/body.json");
    }
    gated("q.is_sneaking && math.mod(q.frame, 6) < 3").shows("!flash");

    for (trigger, visible) in [
        ("{'reaction': 'is_sneaking', 'on': 'start'}, 'length': 12", "q.is_sneaking && math.mod(q.frame, 2) == 0"),
        ("{'reaction': 'is_sneaking', 'on': 'stop'}, 'length': 12", "!q.is_sneaking && q.is_sneaking_time < 0.9 && math.mod(q.frame, 2) == 0"),
        ("{'reaction': 'is_sneaking', 'on': 'start'}, 'length': 24", "!q.is_sneaking && q.is_sneaking_time < 0.75 && q.frame >= 12 && math.mod(q.frame, 2) == 0"),
        ("{'reaction': 'is_sprinting', 'on': 'while'}", "q.ground_speed > 5 && math.mod(q.frame, 2) == 0"),
        ("'equip', 'length': 12", "q.time_of_day > 0.9 && math.mod(q.frame, 2) == 0"),
    ] {
        event(trigger, visible).shows("flash animation=pop");
    }
    let flicker = plate("8/8 & solid:blink:9:wk & +blink:over_text {'kind': 'sprite', 'sprite': 'blink', 'anchor': 'top', 'pivot': 'bottom', 'breakpoints': [{'min': 20, 'max': 40}]}");
    flicker.shows("flash name_width=20 layer=blink");
}

#[test]
fn one_shot_runs_and_their_sparks_are_simulated() {
    plate("24/24 & palettes.json/slots/- {'id': 'strobe', 'ramp': ['#000000']} & body:strobe & manifest.json/animations/pop \
        {'trigger': 'equip', 'length': 6, 'tracks': {'palette.strobe': {'keys': [[0, '#000000'], [2, '#FFFFFF'], [4, '#000000']]}}}")
    .shows("flash animation=pop flashes=4");

    let edges = |steady: u32, blink: u32, visible: &str| {
        plate(&format!("12/12 & body:lite & manifest.json/animations {{'steady': {{'trigger': {{'reaction': 'is_sneaking', 'on': 'stop'}}, 'length': {steady}, \
            'tracks': {{'body': {{'visible': [[0, true]]}}}}}}, 'blink': {{'trigger': {{'reaction': 'is_sneaking', 'on': 'start'}}, 'length': {blink}, \
            'tracks': {{'body': {{'visible': [[0, {visible}]]}}}}}}}}"))
    };
    let held = edges(12, 12, "'!q.is_sneaking && math.mod(q.frame, 2) == 0'");
    held.shows("!flash");
    edges(2, 12, "'!q.is_sneaking && math.mod(q.frame, 2) == 0'").shows("flash");

    let tapped = edges(3, 6, "false");
    tapped.shows("flash");

    let tap = |length: u32| {
        plate(&format!("12/12 & body:lite & manifest.json/animations {{'loop': {{'tracks': {{'body': {{'visible': [[0, false]]}}}}}}, \
            'pop': {{'trigger': {{'reaction': 'is_sneaking', 'on': 'start'}}, 'length': {length}, 'tracks': {{'body': {{'visible': [[0, true]]}}}}}}}}"))
    };
    tap(1).shows("flash animation=pop");
    tap(6).shows("!flash");

    let dip = "'dip': {'trigger': {'reaction': 'is_sneaking', 'on': 'start'}, 'length': 2, 'tracks': {'body': {'visible': [[0, true]]}}}";
    let sparks = |frames: &str, emitter: &str, animations: &str| {
        plate(&format!("12/12 & body:dark & solid:gem:9:{frames} & +sparks:over_text {{'kind': 'emitter', 'sprites': ['gem']{emitter}}} & manifest.json/animations {{{animations}}}"))
    };
    let restarted = |first: u32| {
        sparks("w", "", &format!("{dip}, 'sparkle': {{'trigger': {{'reaction': 'is_sneaking', 'on': 'start'}}, 'length': 24, 'restart': true, \
            'tracks': {{'sparks': {{'burst': [[{first}, 1], [18, 1]]}}}}}}"))
    };
    restarted(6).shows("!flash");
    restarted(0).shows("flash");
    sparks("w..w....", ", 'lifetime': [8, 8]", "'pop': {'trigger': {'reaction': 'is_sneaking', 'on': 'start'}, 'length': 1, 'tracks': {'sparks': {'burst': [[0, 1]]}}}")
        .shows("flash animation=pop px>64");
    let bursts = json!((0..12).map(|f| json!([f, 1])).collect::<Vec<_>>());
    let glowing = |life: u32| {
        sparks("w", &format!(", 'lifetime': [{life}, {life}]"), &format!("{dip}, 'glow': {{'trigger': {{'reaction': 'is_sneaking', 'on': 'while'}}, 'tracks': {{'sparks': {{'burst': {bursts}}}}}}}"))
    };
    glowing(10).shows("!flash");
    glowing(1).shows("flash");

    let appear = |length: usize, keys: Value| {
        plate(&format!("12/12 & palettes.json/slots/- {{'id': 'strobe', 'ramp': ['#000000']}} & body:strobe & manifest.json/animations/hello \
            {{'trigger': {{'appear': 1}}, 'length': {length}, 'tracks': {{'palette.strobe': {{'keys': {keys}}}}}}}"))
    };
    appear(3, json!([[0, "#000000"], [1, "#FFFFFF"], [2, "#000000"]])).shows("!flash");
    appear(
        8,
        json!((0..8)
            .map(|f| json!([f, if f % 2 == 0 { "#FFFFFF" } else { "#000000" }]))
            .collect::<Vec<_>>()),
    )
    .shows("flash animation=hello");

    for phase in ["global", "per_player"] {
        let greeted = plate(&format!("12/12 & manifest.json/phase '{phase}' & body:lite & +glint:background {{'kind': 'body', 'fill': {{'color': 'lite'}}}} \
            & manifest.json/animations {{'loop': {{'tracks': {{'body': {{'visible': [[0, true], [1, false], [4, true], [5, false], [8, true], [9, false]]}}, 'glint': {{'visible': [[0, false]]}}}}}}, \
            'hello': {{'trigger': {{'appear': 1}}, 'length': 12, 'tracks': {{'glint': {{'visible': [[0, true], [1, false]]}}}}}}}}"));
        greeted.shows("flash animation=hello");
    }
}

fn scheduled(base: &str, window: u32, jitter: u32, length: u32, keys: &str) -> NameplateAsset {
    plate(&format!("12/12 & body:lite & manifest.json/animations/loop/tracks/body/visible {base} \
        & manifest.json/animations/blink {{'schedule': {{'window': {window}, 'jitter': {jitter}}}, 'length': {length}, 'tracks': {{'body': {{'visible': {keys}}}}}}}"))
}

#[test]
fn schedules_are_checked_at_every_offset_and_sequence() {
    let (quiet, blinking, pulse) = (
        "[[0, false]]",
        "[[0, true], [1, false]]",
        "[[0, true], [1, false]]",
    );
    let windowed = scheduled(quiet, 6, 4, 2, pulse);
    windowed.shows("!flash");
    scheduled(quiet, 3, 1, 2, pulse).shows("flash");

    let jittered = scheduled(blinking, 6, 2, 2, pulse);
    let starts: Vec<i64> = (0..4)
        .filter_map(|k| roll(&jittered.animations.schedules[0], 3, k))
        .map(|e| e.start)
        .collect();
    assert_eq!(starts, [0, 7, 14, 18]);
    jittered.shows("flash animation=blink");
    let late = scheduled(
        blinking,
        12,
        4,
        6,
        "[[0, true], [1, false], [4, true], [5, false]]",
    );
    late.shows("flash animation=blink");
    let calm = scheduled(blinking, 12, 4, 2, pulse);
    calm.shows("!flash");

    let phased = |phase: &str| {
        let base = format!("[[0, true], [1, false], [4, true], [5, false], [8, true], [9, false]] & manifest.json/phase '{phase}'");
        scheduled(&base, 12, 0, 2, pulse)
    };
    let per_player = phased("per_player");
    assert!(scenarios(&per_player)
        .iter()
        .any(|s| s.source == From::Schedule(0) && s.seed == 6));
    per_player.shows("flash animation=blink");
    let global = phased("global");
    global.shows("!flash");
    assert!(scenarios(&global).iter().all(|s| s.seed == 0));

    let patterned = |window: u32, patterns: &[&str]| {
        let event = |(i, p): (usize, &&str)| {
            let keys: Vec<Value> = p
                .chars()
                .enumerate()
                .map(|(f, c)| json!([f, c == '1']))
                .collect();
            format!("manifest.json/animations/e{i} {{'schedule': {{'id': 'blink', 'window': {window}}}, 'length': {}, 'tracks': {{'body': {{'visible': {}}}}}}}", p.len(), json!(keys))
        };
        plate(&format!(
            "12/12 & body:lite & {}",
            patterns
                .iter()
                .enumerate()
                .map(event)
                .collect::<Vec<_>>()
                .join(" & ")
        ))
    };
    let mixed = patterned(4, &["0010", "1110"]);
    mixed.shows("flash animation=blink");
    let spread = patterned(8, &["00100000", "11100000"]);
    spread.shows("!flash");
    let three = patterned(5, &["00001", "00010", "10000"]);
    three.shows("flash animation=blink");

    let chance = |chance: f64| {
        plate(&format!("24/24 & body:lite & manifest.json/animations {{'loop': {{'tracks': {{'body': {{'visible': [[0, false]]}}}}}}, \
        'blink': {{'schedule': {{'window': 2, 'chance': {chance}}}, 'length': 2, 'tracks': {{'body': {{'visible': [[0, true], [1, false]]}}}}}}}}"))
    };
    chance(1.0).shows("flash");
    let silent = chance(0.0);
    silent.shows("!flash");
    assert!(scenarios(&silent)
        .iter()
        .all(|s| s.events.is_empty() && !matches!(s.source, From::Schedule(_))));

    for changes in [
        &[10, 20, 21, 22, 23, 24, 25, 26][..],
        &[1, 10, 20, 21, 22, 23, 24, 25, 26],
    ] {
        let mut tracker = Tracker::new(1, 12, 40);
        let mut light = 0;
        tracker.start(0, 0);
        for frame in 1..40 {
            if changes.contains(&frame) {
                light = SCALE - light;
            }
            tracker.step(0, light, frame);
        }
        tracker.finish();
        assert_eq!(tracker.most_times(), 4, "{changes:?}");
    }
}

fn runner(base: f64, dash: &str) -> NameplateAsset {
    plate(&format!("24/24 & body:dark & solid:gem:9:w & +gem:over_text {{'kind': 'sprite', 'sprite': 'gem', 'path': {{'preset': 'top', 'speed': {base}}}}} {dash}"))
}

fn dash(trigger: &str, track: &str, speed: f64) -> String {
    format!("& manifest.json/animations/dash {{'trigger': {trigger}, 'tracks': {{'{track}': {{'speed': [[0, {speed}]]}}}}}}")
}

fn covered(dash: &str) -> NameplateAsset {
    plate(&format!("12/12 & solid:runner:16:wk & +runner:over_background {{'kind': 'sprite', 'sprite': 'runner', 'path': {{'preset': 'top', 'speed': 0.03125}}}} \
        & solid:lid:16:k & +lid:over_text {{'kind': 'sprite', 'sprite': 'lid'}} {dash}"))
}

const SPRINT: &str = "{'reaction': 'is_sprinting', 'on': 'while'}";

#[test]
fn paths_are_simulated_at_every_speed_and_along_their_route() {
    runner(16.0, "").shows("flash");
    runner(1.0, &dash(SPRINT, "gem", 16.0)).shows("flash animation=dash");
    runner(1.0, &dash(SPRINT, "gem", 2.0)).shows("!flash");
    let tap = "{'reaction': 'is_sneaking', 'on': 'start'}, 'length': 48";
    runner(1.0, &dash(tap, "gem", 16.0)).shows("flash animation=dash");

    covered(&dash(SPRINT, "runner", 1.0)).shows("flash px>64");
    let slow = covered("");
    let ahead: Vec<u32> = scenarios(&slow)
        .iter()
        .map(|s| s.ahead)
        .filter(|a| *a > 0)
        .collect();
    assert!(!ahead.is_empty() && ahead.iter().all(|a| a % u32::from(slow.loop_frames) == 0));
    slow.shows("flash px>64");
    let still = plate("12/12 & solid:runner:16:wk & +runner:over_background {'kind': 'sprite', 'sprite': 'runner'} & solid:lid:16:k & +lid:over_text {'kind': 'sprite', 'sprite': 'lid'}");
    assert!(scenarios(&still).iter().all(|s| s.ahead == 0));
    still.shows("!flash");
}

fn busy(loop_frames: u32, length: u32) -> NameplateAsset {
    let reads = "q.is_sneaking + q.is_sprinting + q.is_hurt + q.is_in_rain + q.ground_speed + q.vertical_speed + q.time_of_day + q.biome_temperature + q.dimension + q.is_hurt_time + q.is_moving + q.is_moving_time > 1000";
    let tracks = format!("'tracks': {{'body': {{'visible': [[0, '{reads}']]}}}}");
    let mut script =
        format!("24/{loop_frames} & body:lite & manifest.json/animations/loop {{{tracks}}}");
    for (i, signal) in ["is_sneaking", "is_sprinting", "is_hurt", "is_in_rain"]
        .iter()
        .enumerate()
    {
        script += &format!(" & manifest.json/animations/while{i} {{'trigger': {{'reaction': '{signal}', 'on': 'while'}}, {tracks}}}");
    }
    for (i, on) in ["start", "stop"].iter().cycle().take(8).enumerate() {
        script += &format!(" & manifest.json/animations/shot{i} {{'trigger': {{'reaction': 'is_moving', 'on': '{on}'}}, 'length': {length}, {tracks}}}");
    }
    plate(&script)
}

fn blinkers(script: &str, depth: &str, layer: &str, count: usize) -> String {
    let mut script = script.to_string();
    for i in 0..count {
        script += &format!(" & +blinker{i}:{depth} {layer} & manifest.json/animations/blink{i} {{'schedule': {{'window': {}, 'jitter': {}}}, 'length': 10, \
            'tracks': {{'blinker{i}': {{'visible': [[0, false], [5, true]]}}}}}}", 590 - i, 580 - i);
    }
    script
}

fn crowded(schedules: usize) -> NameplateAsset {
    let bar = json!({ "colors": { "w": "lite" }, "frames": [vec!["w".repeat(16); 8]] });
    let script = blinkers(
        &format!("24/600 & manifest.json/phase 'per_player' & body:fill & sprites/bar.json {bar}"),
        "over_background",
        "{'kind': 'sprite', 'sprite': 'bar', 'anchor': 'top'}",
        schedules,
    );
    plate(&format!("{script} & +strobe:over_background {{'kind': 'sprite', 'sprite': 'bar', 'anchor': 'bottom'}} \
        & manifest.json/animations/loop/tracks/strobe/visible [[0, 'q.ground_speed > 5 && math.mod(q.frame, 2) == 0']]"))
}

fn jittery(schedules: usize) -> NameplateAsset {
    plate(&blinkers(
        "24/600",
        "background",
        "{'kind': 'body', 'fill': {'color': 'lite'}}",
        schedules,
    ))
}

#[test]
fn flash_work_is_planned_within_its_budget() {
    for asset in [busy(600, 600), jittery(16)] {
        let list = scenarios(&asset);
        assert_eq!(list, scenarios(&asset));
        let frames: u64 = list.iter().map(|s| u64::from(s.frames)).sum();
        assert!(frames <= SCENARIO_FRAMES && list.len() <= SCENARIO_ROWS);
    }
    let list = scenarios(&busy(48, 120));
    let (moving, speed) = (
        signal_id("is_moving").unwrap(),
        signal_id("ground_speed").unwrap(),
    );
    let speeds = [0.0, 1.3, 4.3, 5.6, 10.0, 30.0];
    for shot in 0..8u16 {
        let rows: Vec<&Scenario> = list
            .iter()
            .filter(|s| s.source == From::OneShot(shot))
            .collect();
        assert!(rows
            .iter()
            .all(|s| s.sample.values[moving] == if shot % 2 == 0 { 1.0 } else { 0.0 }));
        assert!(
            speeds
                .iter()
                .all(|v| rows.iter().any(|s| s.sample.values[speed] == *v)),
            "shot {shot}"
        );
    }

    let crowd = crowded(5);
    let required = ranked(&crowd);
    assert!(speeds.iter().all(|v| required
        .iter()
        .any(|(s, r)| *r && s.source == From::Loop && s.sample.values[speed] == *v)));
    crowd.shows("flash");
    assert!(planned(&crowd).1.is_empty());
    let packed = crowded(16);
    let (list, unplanned) = planned(&packed);
    let frames: u64 = list.iter().map(|(s, _)| u64::from(s.frames)).sum();
    assert!(!unplanned.is_empty() && frames <= SCENARIO_FRAMES && list.len() <= SCENARIO_ROWS);
    let codes: Vec<IssueCode> = check(&packed, Mode::Authoring)
        .iter()
        .map(|i| i.code)
        .collect();
    assert!(codes.contains(&IssueCode::Flash) || codes.contains(&IssueCode::FlashUnchecked));

    let drift = "materials/drift.json {'resolution': 'pixel', 'nodes': [{'id': 'f', 'type': 'Frame'}, {'id': 'd', 'type': 'div', 'in': {'a': 'f', 'b': 6000}}, \
        {'id': 'h', 'type': 'Hsv', 'in': {'h': 'd'}}, {'id': 'out', 'type': 'Output', 'in': {'color': 'h'}}]}";
    let drifting = plate(&format!("24/600 & {drift} & +drift0:behind {{'kind': 'material', 'material': 'drift'}} & +drift1:behind {{'kind': 'material', 'material': 'drift'}}"));
    let mut report = Report::new(Mode::Authoring);
    let limit = 20_000_000;
    let spent = spend(&drifting, &measure(&drifting), &mut report, limit);
    assert!(spent >= limit && spent < limit + limit / 10, "{spent}");
    assert!(report.has_errors() && !report.issues().iter().any(|i| i.code == IssueCode::Flash));
    let unchecked = report
        .issues()
        .iter()
        .find(|i| i.code == IssueCode::FlashUnchecked)
        .unwrap();
    assert_eq!(value(unchecked, "checked"), 0.0);

    let asset = busy(48, 24);
    let measured = measure(&asset);
    let order = ranked(&asset);
    let required = order.iter().take_while(|(_, r)| *r).count();
    assert!(required < order.len() && order[required..].iter().all(|(_, r)| !*r));
    let run = |limit: u64| {
        let mut report = Report::new(Mode::Authoring);
        (
            spend(&asset, &measured, &mut report, limit),
            report.into_issues(),
        )
    };
    let (total, issues) = run(u64::MAX);
    assert!(issues.is_empty());
    let (spent, issues) = run(total * 9 / 10);
    assert!(spent < total && issues.is_empty());
    assert!(run(1).1.iter().any(|i| i.code == IssueCode::FlashUnchecked));
}

fn full_render(stage: &Stage<'_>, region: Rect, scenario: &Scenario) -> (u32, u32) {
    let light = Luminance::new();
    let mut canvas = Canvas::new(region);
    let count = canvas.pixels().len();
    let fps = u32::from(stage.asset.fps.max(1));
    let mut trackers = [
        Tracker::new(count, fps, scenario.frames),
        Tracker::new(count, fps, scenario.frames),
    ];
    for frame in 0..scenario.frames {
        let shot = stage.shot(scenario, frame);
        stage.render(&shot, &stage.draws(&shot, 0), &mut canvas);
        for (i, argb) in canvas.pixels().iter().enumerate() {
            for (tracker, backdrop) in trackers.iter_mut().zip([0, 0x00ff_ffff]) {
                let value = light.over(*argb, backdrop);
                if frame == 0 {
                    tracker.start(i, value);
                } else {
                    tracker.step(i, value, frame);
                }
            }
        }
    }
    trackers.iter_mut().for_each(Tracker::finish);
    let [dark, bright] = trackers;
    (
        dark.most_pixels().max(bright.most_pixels()),
        dark.most_times().max(bright.most_times()),
    )
}

#[test]
fn incremental_flash_tracking_matches_full_frames() {
    let light = Luminance::new();
    for asset in [
        jittery(3),
        strobe(2),
        busy(12, 6),
        runner(1.0, &dash(SPRINT, "gem", 16.0)),
        covered(&dash(SPRINT, "runner", 1.0)),
    ] {
        let list = scenarios(&asset);
        for input in placements(&asset).iter().take(4) {
            let stage = Stage::new(&asset, 0, input).unwrap();
            let region = stage.region();
            for scenario in list.iter().take(12) {
                let fast = simulate(&stage, region, scenario, 0, &light);
                let actual = &(fast.pixels, fast.times);
                assert_eq!(*actual, full_render(&stage, region, scenario));
            }
        }
    }
}

fn lettered(text: &str) -> NameplateAsset {
    plate(&format!("6/12 & body:fill & manifest.json/text {text}"))
}

fn owned(motion: &str) -> NameplateAsset {
    lettered(&format!("{{'mode': 'owned', 'motion': '{motion}'}}"))
}

#[test]
fn letter_readability_is_judged_in_every_layout_signal_and_variant() {
    owned("v.opacity = 0; return 0;").shows(
        "letter_readable level=error path=manifest.json#text.motion frames=0 of=12 variant=default",
    );
    owned("v.opacity = 0; return 0;").shows("client !letter_readable");
    owned("return q.frame < 5 ? 0 : 8;").shows("letter_readable frames=5 of=12");
    for when in [
        "q.glyph_index >= 16",
        "q.glyph_count == 7",
        "q.glyph_x > 150",
        "q.glyph_index == 0 && q.glyph_width == 4",
        "q.glyph_index == 5 && q.glyph_width == 2",
        "q.glyph_index == 2 && q.glyph_x == 10",
        "q.is_sneaking",
        "q.is_hurt_time > 0.5",
        "q.ground_speed > 3",
        "q.dimension == 1",
        "q.is_sneaking && q.is_hurt_time > 1 && !q.is_on_fire && !q.is_sprinting && !q.is_swimming",
        "q.glyph_count == 2 && q.is_hurt",
        "q.glyph_count == 2 && q.glyph_width == 3",
        "q.glyph_count == 5 && q.glyph_index == 4 && q.glyph_width == 8",
    ] {
        owned(&format!("v.opacity = {when} ? 0 : 255; return 0;")).shows("letter_readable");
    }
    owned("v.rotation = q.glyph_index == 1 ? 90 : 0; return 0;").shows("letter_readable");
    for motion in [
        "v.opacity = q.glyph_count > 24 ? 0 : 255; return 0;",
        "v.opacity = q.is_hurt_time < 0.25 ? 0 : 255; return 0;",
        "return q.frame < 6 ? 0 : 8;",
        "return q.is_sneaking ? 2 : 0;",
        "return q.ground_speed > 3 ? 2 : 0;",
        "v.color = 255; v.scale = 1.25; v.rotation = 15; return 2;",
    ] {
        owned(motion).shows("!letter_readable");
    }
    lettered("{'mode': 'motion', 'motion': 'v.opacity = 0; return 0;'}").shows("!letter_readable");

    let variants = plate("6/12 & body:fill & manifest.json/text {'mode': 'owned', 'motion': 'return 0;'} \
        & manifest.json/variants {'calm': {}, 'wild': {'text': {'mode': 'owned', 'motion': 'return 20;'}}}");
    let flagged = check(&variants, Mode::Authoring)
        .into_iter()
        .filter(|i| i.code == IssueCode::LetterReadable)
        .count();
    assert_eq!(flagged, 1);
    variants.shows("letter_readable path=manifest.json#variants.wild.text.motion variant=wild");
}

#[test]
fn reviewers_are_warned_about_readability_layout_and_hosts() {
    let duck = "manifest.json/animations/duck {'trigger': {'reaction': 'is_sneaking', 'on': 'while'}, 'tracks': {'palette.fill': {'keys': [[0, '#FFFFFF']]}}}";
    plate("6/12 & body:fill").shows("!readability");
    plate(&format!("6/12 & body:fill & {duck}"))
        .shows("readability level=warning path=manifest.json#text animation=duck ratio<3");
    plate("6/12 & body:fill & manifest.json/animations/loop/tracks/$plate/position [[0, [0, 0]], [6, [0, -1]]]")
        .shows("plate_background path=manifest.json#animations.loop.tracks.$plate");
    plate("6/12 & body:fill & manifest.json/animations/loop/tracks/$badge/opacity [[0, 0.25]]")
        .shows("badge_faded path=manifest.json#animations.loop.tracks.$badge");

    let bands = json!((0..9)
        .map(|_| json!({ "color": "fill" }))
        .collect::<Vec<_>>());
    let nine = plate(&format!("6/12 & +body:background {{'kind': 'body', 'fill': {{'flag': true}}}} & flags.json {{'flags': [{{'id': 'nine', 'bands': {bands}}}]}} \
        & manifest.json/variants {{'a': {{'flag': 'nine'}}}}"));
    nine.shows("flag_unfit path=flags.json#flags.nine lines=1 height=8");
    nine.shows("client !flag_unfit");

    let banded = kit("6/12 & palettes.json/slots/- {'id': 'glow', 'ramp': ['#101010', '#808080', '#F0F0F0']} \
        & +body:background {'kind': 'body', 'fill': {'gradient': 'glow'}} & palettes.json/colorways {'mint': {'glow': ['#000000', '#FFFFFF']}}");
    let issues = check(&banded.compile(Mode::Client).0.unwrap(), Mode::Authoring);
    let paths: Vec<&str> = issues
        .iter()
        .filter(|i| i.code == IssueCode::ColorwayTones)
        .map(|i| i.path.as_str())
        .collect();
    assert_eq!(paths, ["palettes.json#colorways.mint.glow"]);

    let looping = |max: u32| {
        plate(&format!("6/12 & manifest.json/params {{'speed': {{'value': 4, 'min': 1, 'max': {max}}}}} & +wave:behind {{'kind': 'material', 'material': 'wave'}} \
            & materials/wave.json {{'resolution': 'pixel', 'nodes': [{{'id': 'f', 'type': 'Frame'}}, {{'id': 'p', 'type': 'Param', 'set': {{'name': 'speed'}}}}, \
            {{'id': 'm', 'type': 'mod', 'in': {{'a': 'f', 'b': 'p'}}}}, {{'id': 'd', 'type': 'div', 'in': {{'a': 'm', 'b': 8}}}}, {{'id': 'h', 'type': 'Hsv', 'in': {{'h': 'd'}}}}, \
            {{'id': 'out', 'type': 'Output', 'in': {{'color': 'h'}}}}]}}"))
    };
    looping(5).shows("material_loop path=layers/wave.json#material param=speed at=max");
    looping(6).shows("!material_loop");
    looping(5).shows("client !material_loop");
}

#[test]
fn kit_bundles_pass_client_validation_without_flashes() {
    for name in "test_plate wardrobe_plate reaction_plate dome_plate material_plate atlas_plate sample_plate".split(' ') {
        let files = testdata(name);
        let Loaded::Asset { asset, .. } = load(Source::Files(&files), &LoadOptions::default())
        else {
            panic!("{name}")
        };
        assert!(check(&asset, Mode::Client).is_empty(), "{name}");
        let mut report = Report::new(Mode::Authoring);
        flash_check(&asset, &measure(&asset), &mut report);
        assert!(report.issues().is_empty(), "{name}: {:?}", report.issues());
    }
}
