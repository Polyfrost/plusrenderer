//! The core inside a Freya app: Freya owns the window and GPU backend, a rect's render callback draws.
//! `cargo run -p plusrenderer-skia --example freya_viewer`. Drag to orbit, scroll to zoom, space toggles the emote.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use plusrenderer::{Arms, Camera, Scene};
use freya::prelude::*;
use skia_safe::{Data, Image};

const PLAYER: usize = 0;
const SKIN: &[u8] = include_bytes!("../../../packages/demo/public/skin.png");
const EMOTE: &str = include_str!("../../../packages/demo/public/emote.json");

struct Player {
    scene: Scene,
    textures: Vec<Option<Image>>,
    start: Instant,
    emote: bool,
}

fn player() -> Player {
    let image = |bytes: &[u8]| Image::from_encoded(Data::new_copy(bytes)).expect("a PNG");

    let mut scene = Scene::default();
    scene.load_player(PLAYER, Arms::Wide);
    scene.load_animation(PLAYER, EMOTE).expect("emote");

    Player {
        scene,
        textures: vec![Some(image(SKIN))],
        start: Instant::now(),
        emote: false,
    }
}

fn main() {
    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new(app)
                .with_title("plusrenderer")
                .with_size(640.0, 800.0),
        ),
    )
}

fn app() -> impl IntoElement {
    let player = use_hook(|| Rc::new(RefCell::new(player())));
    let mut seconds = use_state(|| 0.0f32);
    let mut orbit = use_state(|| (0.0f32, 8.0f32, 70.0f32));
    let mut drag = use_state(|| None::<CursorPoint>);

    // A new time re-renders the component, which repaints the rect, which ticks again: frames run
    // back to back only while the scene animates. Input redraws through its own state.
    use_hook({
        let player = player.clone();
        move || {
            let ticker = RenderingTicker::get();
            spawn(async move {
                loop {
                    ticker.tick().await;
                    let player = player.borrow();
                    if player.scene.is_animating() {
                        seconds.set(player.start.elapsed().as_secs_f32());
                    }
                }
            });
        }
    });

    let (yaw, pitch, distance) = orbit();
    let now = seconds();
    let draw = player.clone();
    let render = RenderCallback::new(move |ctx| {
        let player = &mut *draw.borrow_mut();
        let camera = Camera::orbit(yaw, pitch, distance, [0.0, 18.0, 0.0]);
        player.scene.pose(now);
        let buffers = player
            .scene
            .build(&camera, [ctx.size.width, ctx.size.height]);
        plusrenderer_skia::draw(ctx.canvas, buffers, &player.textures);
    });

    rect()
        .expanded()
        .background(Color::from_rgb(0x20, 0x22, 0x25))
        .child(rect().expanded().background(render))
        .on_pointer_down(move |e: Event<PointerEventData>| drag.set(Some(e.global_location())))
        .on_global_pointer_up(move |_| drag.set_if_modified(None))
        .on_global_pointer_move(move |e: Event<PointerEventData>| {
            let Some(from) = drag() else { return };
            let to = e.global_location();
            orbit.with_mut(|mut o| {
                o.0 += (to.x - from.x) as f32 * 0.6;
                o.1 = (o.1 - (to.y - from.y) as f32 * 0.4).clamp(-90.0, 90.0);
            });
            drag.set(Some(to));
        })
        .on_wheel(move |e: Event<WheelEventData>| {
            orbit.with_mut(|mut o| o.2 = (o.2 - e.delta_y as f32 * 0.1).clamp(25.0, 160.0));
        })
        .on_global_key_down(move |e: Event<KeyboardEventData>| {
            if e.key != Key::Character(" ".into()) {
                return;
            }

            let player = &mut *player.borrow_mut();
            player.emote = !player.emote;
            let now = player.start.elapsed().as_secs_f32();

            if player.emote {
                player.scene.play(PLAYER, now)
            } else {
                player.scene.stop(PLAYER)
            }
            seconds.set(now);
        })
}
