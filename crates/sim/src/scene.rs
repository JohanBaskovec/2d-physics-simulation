//! Ready-made scenes, shared by the viewer, the benchmark and snapshots.

use crate::cell::{air, Cell, Form, AMBIENT_P, ROOM_T};
use crate::world::World;

/// A small scene in the bottom-left corner of the world, with the rest left
/// as open air to build in: a rock floor, a basin of water with mercury
/// poured in, a sand heap, gold dust, a warm iron bar and a helium bubble.
/// Needs a world of at least 448 x 256 cells.
pub fn playground(world: &mut World) {
    let (w, h) = (world.width(), world.height());
    let s = &world.subs;
    let (sio2, h2o, hg, au, fe, he) =
        (s.must("SiO2"), s.must("H2O"), s.must("Hg"), s.must("Au"), s.must("Fe"), s.must("He"));
    let rock = Cell::filled(s, sio2, Form::Rigid, ROOM_T);
    let iron = Cell::filled(s, fe, Form::Rigid, ROOM_T);
    let warm_iron = Cell::filled(s, fe, Form::Rigid, 700.0);
    let water = Cell::filled(s, h2o, Form::Liquid, ROOM_T);
    let mercury = Cell::filled(s, hg, Form::Liquid, ROOM_T);
    let sand = Cell::powder(s, sio2, ROOM_T, &air(s), AMBIENT_P);
    let gold = Cell::powder(s, au, ROOM_T, &air(s), AMBIENT_P);
    let helium = Cell::gas(s, &[(he, 1.0)], ROOM_T, AMBIENT_P);

    let floor = h - 16;
    world.fill_rect(0, floor, w, h, rock);
    // Basin with a low divider.
    world.fill_rect(40, floor - 70, 44, floor, iron);
    world.fill_rect(200, floor - 70, 204, floor, iron);
    world.fill_rect(120, floor - 24, 124, floor, iron);
    world.fill_rect(50, floor - 110, 110, floor - 30, water);
    world.fill_rect(140, floor - 130, 160, floor - 100, mercury);
    world.fill_rect(70, floor - 190, 90, floor - 130, sand);
    world.fill_rect(170, floor - 90, 185, floor - 70, gold);
    // To the right of the basin: a sand column, a warm bar and helium.
    world.fill_rect(240, floor - 150, 270, floor - 20, sand);
    world.fill_rect(320, floor - 60, 360, floor - 52, warm_iron);
    world.fill_rect(390, floor - 40, 420, floor - 10, helium);
}

/// An underground world: solid rock with a chamber in every 128 x 128
/// tile, some joined by tunnels and shafts and some sealed off. Chambers
/// hold different things: water with sand falling in, mercury and gold
/// dust, helium, a warm or a hot iron bar, or just air. The layout is fixed
/// for a given world size.
pub fn caves(world: &mut World) {
    const TILE: usize = 128;
    let (w, h) = (world.width(), world.height());
    let s = &world.subs;
    let (sio2, h2o, hg, au, fe, he) =
        (s.must("SiO2"), s.must("H2O"), s.must("Hg"), s.must("Au"), s.must("Fe"), s.must("He"));
    let rock = Cell::filled(s, sio2, Form::Rigid, ROOM_T);
    let warm_iron = Cell::filled(s, fe, Form::Rigid, 700.0);
    let hot_iron = Cell::filled(s, fe, Form::Rigid, 1500.0);
    let water = Cell::filled(s, h2o, Form::Liquid, ROOM_T);
    let mercury = Cell::filled(s, hg, Form::Liquid, ROOM_T);
    let sand = Cell::powder(s, sio2, ROOM_T, &air(s), AMBIENT_P);
    let gold = Cell::powder(s, au, ROOM_T, &air(s), AMBIENT_P);
    let helium = Cell::gas(s, &[(he, 1.0)], ROOM_T, AMBIENT_P);
    let open = world.ambient();

    // A repeatable pseudo-random number in 0..n for tile `t`, purpose `k`.
    let pick = |t: usize, k: u32, n: u32| -> i32 {
        let mut v = (t as u32).wrapping_mul(0x9E37_79B1) ^ k.wrapping_mul(0x85EB_CA6B);
        v ^= v >> 15;
        v = v.wrapping_mul(0x2C1B_3C6D);
        v ^= v >> 13;
        ((v >> 8) % n) as i32
    };

    world.fill_rect(0, 0, w, h, rock);
    let (cols, rows) = (w / TILE, h / TILE);
    // Chamber centre and half-sizes for a tile.
    let chamber = |tx: usize, ty: usize| -> (i32, i32, i32, i32) {
        let t = ty * cols + tx;
        let cx = (tx * TILE) as i32 + 52 + pick(t, 1, 24);
        let cy = (ty * TILE) as i32 + 56 + pick(t, 2, 16);
        (cx, cy, 30 + pick(t, 3, 16), 22 + pick(t, 4, 14))
    };
    for ty in 0..rows {
        for tx in 0..cols {
            let t = ty * cols + tx;
            let (cx, cy, rx, ry) = chamber(tx, ty);
            // Carve the chamber: an ellipse with a flat floor.
            for y in cy - ry..=cy + ry * 2 / 3 {
                for x in cx - rx..=cx + rx {
                    let (dx, dy) = ((x - cx) as f32 / rx as f32, (y - cy) as f32 / ry as f32);
                    if dx * dx + dy * dy <= 1.0 {
                        world.set(x as usize, y as usize, open);
                    }
                }
            }
            // A tunnel to the chamber on the right, in about half the tiles.
            if tx + 1 < cols && pick(t, 5, 2) == 0 {
                let (nx, ny, _, _) = chamber(tx + 1, ty);
                let y0 = cy.min(ny);
                world.fill_rect(cx as usize, y0 as usize, nx as usize, y0 as usize + 6, open);
                world.fill_rect(nx as usize - 3, y0 as usize, nx as usize + 3, ny.max(cy) as usize + 3, open);
            }
            // A shaft down to the chamber below, in about a third.
            if ty + 1 < rows && pick(t, 6, 3) == 0 {
                let (_, ny, _, _) = chamber(tx, ty + 1);
                world.fill_rect(cx as usize - 3, cy as usize, cx as usize + 3, ny as usize, open);
            }
        }
    }
    // Contents, placed after all carving so tunnels do not cut through them.
    for ty in 0..rows {
        for tx in 0..cols {
            let t = ty * cols + tx;
            let (cx, cy, rx, ry) = chamber(tx, ty);
            let floor = cy + ry * 2 / 3;
            // Fill a box, but only where the chamber is open.
            let mut put = |x0: i32, y0: i32, x1: i32, y1: i32, c: Cell| {
                for y in y0.max(0)..y1.min(h as i32) {
                    for x in x0.max(0)..x1.min(w as i32) {
                        if !world.cell(x as usize, y as usize).has_form(Form::Rigid) {
                            world.set(x as usize, y as usize, c);
                        }
                    }
                }
            };
            match pick(t, 7, 6) {
                0 => {
                    put(cx - rx, floor - ry / 2, cx + rx, floor + 1, water);
                    put(cx - 6, cy - ry + 4, cx + 6, cy - 4, sand);
                }
                1 => {
                    put(cx - rx, floor - ry / 3, cx + rx, floor + 1, mercury);
                    put(cx - 5, cy - ry + 4, cx + 5, cy - 6, gold);
                    put(cx + 8, cy - ry + 6, cx + 16, cy - 2, sand);
                }
                2 => put(cx - 10, floor - 12, cx + 10, floor + 1, helium),
                3 => put(cx - 12, floor - 4, cx + 12, floor, warm_iron),
                4 => put(cx - 8, floor - 4, cx + 8, floor, hot_iron),
                _ => {}
            }
        }
    }
}

/// The benchmark stress scene: a repeating tile that keeps every chunk
/// busy. Each tile has a sand column and a water column that collapse into
/// a basin, mercury poured on the water, gold dust, a hot iron bar and a
/// helium bubble.
pub fn demo(world: &mut World) {
    const TILE: usize = 128;
    let (w, h) = (world.width(), world.height());
    let s = &world.subs;
    let (sio2, h2o, hg, au, fe, he) =
        (s.must("SiO2"), s.must("H2O"), s.must("Hg"), s.must("Au"), s.must("Fe"), s.must("He"));
    let rock = Cell::filled(s, sio2, Form::Rigid, ROOM_T);
    let iron = Cell::filled(s, fe, Form::Rigid, ROOM_T);
    let hot_iron = Cell::filled(s, fe, Form::Rigid, 1500.0);
    let water = Cell::filled(s, h2o, Form::Liquid, ROOM_T);
    let mercury = Cell::filled(s, hg, Form::Liquid, ROOM_T);
    let sand = Cell::powder(s, sio2, ROOM_T, &air(s), AMBIENT_P);
    let gold = Cell::powder(s, au, ROOM_T, &air(s), AMBIENT_P);
    let helium = Cell::gas(s, &[(he, 1.0)], ROOM_T, AMBIENT_P);

    for ty in (0..h).step_by(TILE) {
        for tx in (0..w).step_by(TILE) {
            let mut put = |x0: usize, y0: usize, x1: usize, y1: usize, c: Cell| {
                world.fill_rect(tx + x0, ty + y0, (tx + x1).min(w), (ty + y1).min(h), c);
            };
            put(0, 120, 128, 128, rock); // floor
            put(0, 60, 3, 120, rock); // basin walls
            put(125, 60, 128, 120, rock);
            put(62, 96, 66, 120, iron); // divider
            put(10, 8, 30, 70, sand);
            put(36, 20, 58, 90, water);
            put(70, 10, 82, 40, mercury);
            put(88, 30, 100, 50, gold);
            put(104, 70, 120, 78, hot_iron);
            put(100, 96, 122, 116, helium);
        }
    }
}

/// Every cell gets a different gas mixture, temperature and pressure, so no
/// two neighbours are in equilibrium. The most expensive case for the heat
/// and gas passes.
pub fn gas_noise(world: &mut World) {
    let (w, h) = (world.width(), world.height());
    let s = &world.subs;
    let (n2, o2, he) = (s.must("N2"), s.must("O2"), s.must("He"));
    let mut cells = Vec::with_capacity(w * h);
    for i in 0..w * h {
        let r = |k: u32| -> f32 {
            let mut v = (i as u32).wrapping_mul(0x9E37_79B1) ^ k.wrapping_mul(0x85EB_CA6B);
            v ^= v >> 15;
            v = v.wrapping_mul(0x2C1B_3C6D);
            v ^= v >> 13;
            (v >> 8) as f32 / (1u32 << 24) as f32
        };
        let x_he = 0.5 * r(1);
        let mix = [(n2, 0.79 * (1.0 - x_he)), (o2, 0.21 * (1.0 - x_he)), (he, x_he)];
        cells.push(Cell::gas(s, &mix, 250.0 + 300.0 * r(2), AMBIENT_P * (0.7 + 0.6 * r(3))));
    }
    for (i, c) in cells.into_iter().enumerate() {
        world.set(i % w, i / w, c);
    }
}
