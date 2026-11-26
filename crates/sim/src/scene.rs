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
