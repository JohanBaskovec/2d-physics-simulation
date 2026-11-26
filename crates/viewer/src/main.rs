//! Interactive viewer for the simulation.
//!
//!   cargo run --release -p viewer
//!
//! Options:
//!   --empty                 start with nothing but air
//!   --stress                start with the benchmark scene (every chunk busy)
//!   --size W H              world size in cells (multiples of 64)
//!   --screenshot PATH N     run N frames, save a screenshot, and quit

use macroquad::prelude::*;
use sim::cell::{air, AMBIENT_P, ROOM_T};
use sim::render::{self, View};
use sim::{scene, Cell, Form, SubId, Substances, World, SLOTS};

const HELP: &[&str] = &[
    "left mouse  paint          right mouse  erase to air",
    "1-9         substance      Tab          wall / powder / liquid / gas",
    "- =         brush size     [ ]          paint temperature",
    "wheel       zoom           WASD/arrows  pan (or drag middle mouse)",
    "Space       pause          .            single step",
    "V           view: material / temperature / pressure",
    "R           reset demo     C            clear to air      H  hide help",
];

/// The four ways a substance can be painted.
const FORMS: [(Form, &str); 4] = [
    (Form::Rigid, "wall"),
    (Form::Granular, "powder"),
    (Form::Liquid, "liquid"),
    (Form::Gas, "gas"),
];

struct Args {
    empty: bool,
    stress: bool,
    size: (usize, usize),
    screenshot: Option<(String, u32)>,
}

fn parse_args() -> Args {
    let mut args = Args { empty: false, stress: false, size: (1024, 512), screenshot: None };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--empty" => args.empty = true,
            "--stress" => args.stress = true,
            "--size" => {
                let w = it.next().and_then(|v| v.parse().ok()).expect("--size needs W H");
                let h = it.next().and_then(|v| v.parse().ok()).expect("--size needs W H");
                args.size = (w, h);
            }
            "--screenshot" => {
                let path = it.next().expect("--screenshot needs PATH N");
                let n = it.next().and_then(|v| v.parse().ok()).expect("--screenshot needs PATH N");
                args.screenshot = Some((path, n));
            }
            other => panic!("unknown option {other}"),
        }
    }
    args
}

fn window_conf() -> Conf {
    Conf {
        window_title: "2D physics simulation".into(),
        window_width: 1280,
        window_height: 720,
        ..Default::default()
    }
}

/// Build the cell the brush paints.
fn brush_cell(subs: &Substances, id: SubId, form: Form, t: f32) -> Cell {
    match form {
        Form::Rigid | Form::Liquid => Cell::filled(subs, id, form, t),
        Form::Granular => Cell::powder(subs, id, t, &air(subs), AMBIENT_P),
        _ => Cell::gas(subs, &[(id, 1.0)], t, AMBIENT_P),
    }
}

/// One line describing what a cell holds.
fn describe(world: &World, x: usize, y: usize) -> String {
    let c = world.cell(x, y);
    let subs = &world.subs;
    let mut parts = Vec::new();
    for s in 0..SLOTS {
        let what = match c.form[s] {
            Form::None => continue,
            Form::Rigid => "solid",
            Form::Granular => "grains",
            Form::Liquid => "liquid",
            Form::Gas => "gas",
        };
        let sym = &subs.get(c.id[s]).symbol;
        if c.form[s] == Form::Gas {
            parts.push(format!("{sym} {what} {:.2e} mol", c.amount[s]));
        } else {
            parts.push(format!("{sym} {what} {:.0}%", 100.0 * c.slot_volume(subs, s)));
        }
    }
    if parts.is_empty() {
        return format!("({x}, {y})  vacuum");
    }
    // Pressure here is gas pressure, so it is only shown where there is gas.
    let pressure = if c.has_form(Form::Gas) {
        format!("{:.3} atm  ", c.pressure(subs) / sim::ATM)
    } else {
        String::new()
    };
    format!(
        "({x}, {y})  {:.1} K  {pressure}{:.3} g/cm3  |  {}",
        c.temperature(subs),
        c.mass(subs),
        parts.join(", ")
    )
}

#[macroquad::main(window_conf)]
async fn main() {
    let args = parse_args();
    let build = |empty: bool| -> World {
        let mut world = World::new(args.size.0, args.size.1, Substances::builtin());
        if args.stress && !empty {
            scene::demo(&mut world);
        } else if !empty {
            scene::playground(&mut world);
        }
        world
    };
    let mut world = build(args.empty);
    let ids: Vec<SubId> = world.subs.ids().collect();

    let mut zoom = 2.0f32; // screen pixels per cell
    // Top-left of the view, in cells. Start at the bottom-left of the world.
    let (mut cam_x, mut cam_y) = (0.0f32, world.height() as f32);
    let mut view = View::Material;
    let mut paused = false;
    let mut show_help = true;
    let mut selected = ids.iter().position(|&id| world.subs.get(id).symbol == "SiO2").unwrap_or(0);
    let mut form_index = 1; // powder
    let mut paint_t = ROOM_T;
    let mut brush = 4i32;
    let mut texture: Option<Texture2D> = None;
    let mut pixels: Vec<u8> = Vec::new();
    let mut last_mouse = mouse_position();
    let mut frame = 0u32;
    let mut step_ms = 0.0f32;

    loop {
        let (sw, sh) = (screen_width(), screen_height());
        let (mx, my) = mouse_position();

        // ---- input: view ------------------------------------------------
        // Ignore the wheel on the first frames: some window systems report a
        // stale scroll value as the window opens.
        let wheel = if frame < 5 { 0.0 } else { mouse_wheel().1 };
        if wheel != 0.0 {
            // Zoom about the cursor.
            let (wx, wy) = (cam_x + mx / zoom, cam_y + my / zoom);
            zoom = if wheel > 0.0 { (zoom + 1.0).min(12.0) } else { (zoom - 1.0).max(1.0) };
            cam_x = wx - mx / zoom;
            cam_y = wy - my / zoom;
        }
        let pan = 600.0 / zoom * get_frame_time();
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            cam_x -= pan;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            cam_x += pan;
        }
        if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
            cam_y -= pan;
        }
        if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down) {
            cam_y += pan;
        }
        if is_mouse_button_down(MouseButton::Middle) {
            cam_x -= (mx - last_mouse.0) / zoom;
            cam_y -= (my - last_mouse.1) / zoom;
        }
        last_mouse = (mx, my);
        let (view_w, view_h) = ((sw / zoom).ceil(), (sh / zoom).ceil());
        cam_x = cam_x.clamp(0.0, (world.width() as f32 - view_w).max(0.0));
        cam_y = cam_y.clamp(0.0, (world.height() as f32 - view_h).max(0.0));

        // ---- input: tools -----------------------------------------------
        const NUMBER_KEYS: [KeyCode; 9] = [
            KeyCode::Key1,
            KeyCode::Key2,
            KeyCode::Key3,
            KeyCode::Key4,
            KeyCode::Key5,
            KeyCode::Key6,
            KeyCode::Key7,
            KeyCode::Key8,
            KeyCode::Key9,
        ];
        for (k, key) in NUMBER_KEYS.iter().enumerate() {
            if is_key_pressed(*key) && k < ids.len() {
                selected = k;
            }
        }
        if is_key_pressed(KeyCode::Tab) {
            form_index = (form_index + 1) % FORMS.len();
        }
        if is_key_pressed(KeyCode::Minus) {
            brush = (brush - 1).max(0);
        }
        if is_key_pressed(KeyCode::Equal) {
            brush = (brush + 1).min(40);
        }
        if is_key_pressed(KeyCode::LeftBracket) {
            paint_t = (paint_t / 1.25).max(20.0);
        }
        if is_key_pressed(KeyCode::RightBracket) {
            paint_t = (paint_t * 1.25).min(4000.0);
        }
        if is_key_pressed(KeyCode::Space) {
            paused = !paused;
        }
        if is_key_pressed(KeyCode::V) {
            view = view.next();
        }
        if is_key_pressed(KeyCode::H) {
            show_help = !show_help;
        }
        if is_key_pressed(KeyCode::R) || is_key_pressed(KeyCode::C) {
            world = build(is_key_pressed(KeyCode::C));
        }

        let cursor = (
            (cam_x + mx / zoom).floor() as i32,
            (cam_y + my / zoom).floor() as i32,
        );
        let (id, (form, form_name)) = (ids[selected], FORMS[form_index]);
        if is_mouse_button_down(MouseButton::Left) {
            let cell = brush_cell(&world.subs, id, form, paint_t);
            world.fill_circle(cursor.0, cursor.1, brush, cell);
        } else if is_mouse_button_down(MouseButton::Right) {
            let cell = world.ambient();
            world.fill_circle(cursor.0, cursor.1, brush, cell);
        }

        // ---- simulate ---------------------------------------------------
        if !paused || is_key_pressed(KeyCode::Period) {
            world.step();
            let ms = world.stats.total().as_secs_f32() * 1e3;
            step_ms += 0.1 * (ms - step_ms);
        }

        // ---- draw -------------------------------------------------------
        let (tw, th) = (view_w as usize, view_h as usize);
        if pixels.len() != tw * th * 4 {
            pixels = vec![0; tw * th * 4];
            let t = Texture2D::from_rgba8(tw as u16, th as u16, &pixels);
            t.set_filter(FilterMode::Nearest);
            texture = Some(t);
        }
        let (ox, oy) = (cam_x.floor() as i32, cam_y.floor() as i32);
        render::draw(&world, view, ox, oy, tw, th, &mut pixels);
        let tex = texture.as_ref().expect("texture exists once pixels are sized");
        tex.update_from_bytes(tw as u32, th as u32, &pixels);
        clear_background(BLACK);
        draw_texture_ex(
            tex,
            -(cam_x - ox as f32) * zoom,
            -(cam_y - oy as f32) * zoom,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(tw as f32 * zoom, th as f32 * zoom)),
                ..Default::default()
            },
        );
        draw_circle_lines(mx, my, (brush as f32 + 0.5) * zoom, 1.0, WHITE);

        let st = world.stats;
        let lines = [
            format!(
                "{} fps   step {:.1} ms   chunks {}/{}   {}{}",
                get_fps(),
                step_ms,
                st.active_chunks,
                world.chunks.cw * world.chunks.ch,
                match view {
                    View::Material => "material view",
                    View::Temperature => "temperature view",
                    View::Pressure => "pressure view",
                },
                if paused { "   PAUSED" } else { "" },
            ),
            format!(
                "brush: {} ({}) as {}   {:.0} K   radius {}",
                world.subs.get(id).name,
                world.subs.get(id).symbol,
                form_name,
                paint_t,
                brush
            ),
            if cursor.0 >= 0
                && cursor.1 >= 0
                && (cursor.0 as usize) < world.width()
                && (cursor.1 as usize) < world.height()
            {
                describe(&world, cursor.0 as usize, cursor.1 as usize)
            } else {
                String::new()
            },
        ];
        let panel_h = 22.0 * lines.len() as f32 + 8.0;
        draw_rectangle(0.0, 0.0, sw, panel_h, Color::new(0.0, 0.0, 0.0, 0.6));
        for (k, line) in lines.iter().enumerate() {
            draw_text(line, 8.0, 20.0 + 22.0 * k as f32, 20.0, WHITE);
        }
        if show_help {
            // Top right, under the status bar: the scene starts bottom-left.
            let (bw, bh) = (590.0, 20.0 * HELP.len() as f32 + 10.0);
            let (left, top) = (sw - bw, panel_h);
            draw_rectangle(left, top, bw, bh, Color::new(0.0, 0.0, 0.0, 0.6));
            for (k, line) in HELP.iter().enumerate() {
                draw_text(line, left + 8.0, top + 18.0 + 20.0 * k as f32, 18.0, LIGHTGRAY);
            }
        }

        frame += 1;
        if let Some((path, n)) = &args.screenshot {
            if frame >= *n {
                get_screen_data().export_png(path);
                return;
            }
        }
        next_frame().await
    }
}
