//! Renders the core to SVG as flat triangles in draw order: `cargo run --example dump_svg -- slim > out.svg`.
//! Top row: four yaws at rest. Bottom row: four moments of the demo emote.

use plusrenderer::{Arms, Camera, Scene};

const SIZE: f32 = 320.0;
const YAWS: [f32; 4] = [0.0, 45.0, 90.0, 180.0];
const TIMES: [f32; 4] = [0.0, 0.5, 1.0, 1.5];
const EMOTE: &str = include_str!("../../../packages/demo/public/emote.json");

fn main() {
    let arms = match std::env::args().nth(1).as_deref() {
        Some("slim") => Arms::Slim,
        _ => Arms::Wide,
    };

    let mut scene = Scene::default();
    scene.load_player(0, arms);
    scene.load_animation(0, EMOTE).expect("emote");

    let width = SIZE * YAWS.len() as f32;
    let height = SIZE * 2.0;
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"><rect width="100%" height="100%" fill="#202225"/>"##
    );

    for (column, yaw) in YAWS.into_iter().enumerate() {
        draw(&mut svg, &mut scene, yaw, 0.0, column, 0);
    }

    scene.play(0, 0.0);
    for (column, time) in TIMES.into_iter().enumerate() {
        draw(&mut svg, &mut scene, 30.0, time, column, 1);
    }

    svg.push_str("</svg>");
    println!("{svg}");
}

fn draw(svg: &mut String, scene: &mut Scene, yaw: f32, time: f32, column: usize, row: usize) {
    let camera = Camera::orbit(yaw, 12.0, 62.0, [0.0, 18.0, 0.0]);
    scene.pose(time);
    let b = scene.build(&camera, [SIZE, SIZE]);

    svg.push_str(&format!(
        r#"<g transform="translate({},{})">"#,
        SIZE * column as f32,
        SIZE * row as f32
    ));

    for triangle in b.indices.chunks(3) {
        let points: Vec<String> = triangle
            .iter()
            .map(|&i| {
                format!(
                    "{:.1},{:.1}",
                    b.positions[i as usize * 2],
                    b.positions[i as usize * 2 + 1]
                )
            })
            .collect();

        let color = b.colors[triangle[0] as usize] & 0xff_ffff;
        svg.push_str(&format!(
            r##"<polygon points="{}" fill="#{color:06x}" stroke="#111" stroke-width="0.25"/>"##,
            points.join(" ")
        ));
    }

    svg.push_str("</g>");
}
