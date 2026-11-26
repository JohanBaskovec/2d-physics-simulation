//! Behaviour tests for step 1: movement, conservation and sleeping.

use sim::cell::{air, AMBIENT_P, ROOM_T};
use sim::{Cell, Form, SubId, Substances, World, PACKING, SLOTS};

fn world(w: usize, h: usize) -> World {
    World::new(w, h, Substances::builtin())
}

fn wall(w: &World, sym: &str) -> Cell {
    Cell::filled(&w.subs, w.subs.must(sym), Form::Rigid, ROOM_T)
}

fn liquid(w: &World, sym: &str) -> Cell {
    Cell::filled(&w.subs, w.subs.must(sym), Form::Liquid, ROOM_T)
}

fn powder(w: &World, sym: &str) -> Cell {
    Cell::powder(&w.subs, w.subs.must(sym), ROOM_T, &air(&w.subs), AMBIENT_P)
}

/// Volume fraction of a substance in a given form in one cell.
fn vol(w: &World, x: usize, y: usize, sym: &str, form: Form) -> f32 {
    let c = w.cell(x, y);
    match c.find(w.subs.must(sym), form) {
        Some(s) => c.slot_volume(&w.subs, s),
        None => 0.0,
    }
}

fn moles(w: &World, x: usize, y: usize, sym: &str, form: Form) -> f32 {
    let c = w.cell(x, y);
    c.find(w.subs.must(sym), form).map_or(0.0, |s| c.amount[s])
}

fn all_ids(w: &World) -> Vec<SubId> {
    w.subs.ids().collect()
}

fn assert_close(a: f64, b: f64, rel: f64, what: &str) {
    let scale = a.abs().max(b.abs()).max(1e-30);
    assert!((a - b).abs() / scale <= rel, "{what}: {a} vs {b}");
}

#[test]
#[cfg(not(any(feature = "slots-4", feature = "slots-6")))]
fn cell_is_one_cache_line() {
    assert_eq!(SLOTS, 8);
    assert_eq!(std::mem::size_of::<Cell>(), 64);
}

#[test]
fn dry_sand_has_real_bulk_density() {
    let w = world(64, 64);
    let dry = powder(&w, "SiO2");
    // 60% quartz at 2.648 g/cm^3 plus a little air.
    assert!((dry.mass(&w.subs) - 1.589).abs() < 0.01, "{}", dry.mass(&w.subs));
    assert!((dry.temperature(&w.subs) - ROOM_T).abs() < 0.01);
    assert!((dry.pressure(&w.subs) - AMBIENT_P).abs() < 50.0);
}

#[test]
fn still_air_goes_to_sleep() {
    let mut w = world(128, 128);
    for _ in 0..3 {
        w.step();
    }
    assert_eq!(w.stats.active_chunks, 0);
}

/// The worked example from the design: a sand cell (60% sand, 40% air)
/// falls into a full water cell.
#[test]
fn sand_falling_into_water_swaps_pore_air_for_water() {
    let mut w = world(64, 64);
    // A one-cell-wide shaft so nothing can spread sideways.
    w.fill_rect(9, 10, 12, 31, wall(&w, "Fe"));
    let air_cell = w.ambient();
    w.fill_rect(10, 10, 11, 30, air_cell);
    w.set(10, 29, liquid(&w, "H2O"));
    w.set(10, 28, powder(&w, "SiO2"));
    let pore_air = moles(&w, 10, 28, "N2", Form::Gas);
    assert!(pore_air > 0.0);

    for _ in 0..200 {
        w.step();
    }

    // Lower cell: 60% sand with 40% water in the pores, no air.
    assert!((vol(&w, 10, 29, "SiO2", Form::Granular) - PACKING).abs() < 1e-4);
    assert!((vol(&w, 10, 29, "H2O", Form::Liquid) - 0.4).abs() < 1e-3);
    assert_eq!(moles(&w, 10, 29, "N2", Form::Gas), 0.0);
    // Upper cell: the other 60% of the water. The pore air came up into it
    // and from there is free to mix with the air above.
    assert!((vol(&w, 10, 28, "H2O", Form::Liquid) - 0.6).abs() < 1e-3);
    assert!(moles(&w, 10, 28, "N2", Form::Gas) > 0.0);
    assert_eq!(vol(&w, 10, 28, "SiO2", Form::Granular), 0.0);
}

#[test]
fn sand_column_falls_into_a_pile_and_sleeps() {
    let mut w = world(128, 128);
    w.fill_rect(0, 120, 128, 128, wall(&w, "Fe"));
    let sand = powder(&w, "SiO2");
    w.fill_rect(60, 20, 68, 60, sand); // 8 x 40 column
    let before = w.total_moles(w.subs.must("SiO2"));
    for _ in 0..1500 {
        w.step();
    }
    assert_close(before, w.total_moles(w.subs.must("SiO2")), 1e-6, "sand moles");
    // Everything has landed: no powder left in the upper part.
    for y in 0..90 {
        for x in 0..128 {
            assert!(!w.cell(x, y).is_powder(), "powder still at {x},{y}");
        }
    }
    // It formed a pile, wider than the column that fell and resting on the floor.
    let on_floor = (0..128).filter(|&x| w.cell(x, 119).is_powder()).count();
    assert!(on_floor > 16, "pile base {on_floor}");
    // The pile is at rest, so the world sleeps.
    assert_eq!(w.stats.active_chunks, 0, "world should be asleep");
}

#[test]
fn water_levels_in_a_basin() {
    let mut w = world(128, 64);
    w.fill_rect(20, 20, 21, 60, wall(&w, "Fe"));
    w.fill_rect(100, 20, 101, 60, wall(&w, "Fe"));
    w.fill_rect(20, 59, 101, 60, wall(&w, "Fe"));
    w.fill_rect(21, 20, 31, 59, liquid(&w, "H2O")); // 10 wide, 39 tall on the left
    let id = w.subs.must("H2O");
    let before = w.total_moles(id);
    for _ in 0..4000 {
        w.step();
    }
    assert_close(before, w.total_moles(id), 1e-5, "water moles");
    // 390 cells of water over 79 columns: depth is close to 4.94 everywhere.
    let depth = |x: usize| -> f32 { (0..64).map(|y| vol(&w, x, y, "H2O", Form::Liquid)).sum() };
    let depths: Vec<f32> = (21..100).map(depth).collect();
    let (lo, hi) = depths.iter().fold((f32::MAX, f32::MIN), |(l, h), &d| (l.min(d), h.max(d)));
    assert!(hi - lo < 0.5, "surface not level: {lo}..{hi}");
    assert!((lo + hi) / 2.0 > 4.5 && (lo + hi) / 2.0 < 5.4);
}

#[test]
fn mercury_sinks_under_water() {
    let mut w = world(64, 64);
    w.fill_rect(9, 10, 12, 41, wall(&w, "Fe"));
    let air_cell = w.ambient();
    w.fill_rect(10, 10, 11, 40, air_cell);
    w.fill_rect(10, 30, 11, 40, liquid(&w, "H2O"));
    w.fill_rect(10, 20, 11, 30, liquid(&w, "Hg"));
    for _ in 0..600 {
        w.step();
    }
    for y in 30..40 {
        assert!(vol(&w, 10, y, "Hg", Form::Liquid) > 0.99, "mercury should fill row {y}");
    }
    for y in 20..30 {
        assert!(vol(&w, 10, y, "H2O", Form::Liquid) > 0.99, "water should fill row {y}");
    }
}

#[test]
fn sand_sinks_in_water_but_floats_on_mercury() {
    let mut w = world(64, 64);
    w.fill_rect(9, 10, 12, 41, wall(&w, "Fe"));
    w.fill_rect(19, 10, 22, 41, wall(&w, "Fe"));
    let air_cell = w.ambient();
    w.fill_rect(10, 10, 11, 40, air_cell);
    w.fill_rect(20, 10, 21, 40, air_cell);
    w.fill_rect(10, 30, 11, 40, liquid(&w, "H2O"));
    w.fill_rect(20, 30, 21, 40, liquid(&w, "Hg"));
    w.set(10, 12, powder(&w, "SiO2"));
    w.set(20, 12, powder(&w, "SiO2"));
    w.set(20, 14, powder(&w, "Au"));
    for _ in 0..1500 {
        w.step();
    }
    assert!(w.cell(10, 39).is_powder(), "sand should reach the bottom of the water");
    assert!(w.cell(20, 39).is_powder(), "gold should reach the bottom of the mercury");
    assert!(vol(&w, 20, 39, "Au", Form::Granular) > 0.5);
    let sand_y = (0..64).find(|&y| vol(&w, 20, y, "SiO2", Form::Granular) > 0.0).unwrap();
    assert!(sand_y < 31, "sand should float on mercury, found at y={sand_y}");
}

#[test]
fn helium_rises() {
    let mut w = world(64, 128);
    let he = w.subs.must("He");
    let blob = Cell::gas(&w.subs, &[(he, 1.0)], ROOM_T, AMBIENT_P);
    w.fill_rect(24, 100, 40, 116, blob);
    let centre = |w: &World| -> f64 {
        let (mut sum, mut n) = (0.0f64, 0.0f64);
        for y in 0..128 {
            for x in 0..64 {
                let m = moles(w, x, y, "He", Form::Gas) as f64;
                sum += m * y as f64;
                n += m;
            }
        }
        sum / n
    };
    let before = w.total_moles(he);
    let y0 = centre(&w);
    for _ in 0..1200 {
        w.step();
    }
    assert_close(before, w.total_moles(he), 1e-5, "helium moles");
    let y1 = centre(&w);
    assert!(y1 < y0 - 40.0, "helium centre moved from {y0} to {y1}");
}

#[test]
fn heat_flows_and_energy_is_conserved() {
    let mut w = world(64, 64);
    let fe = w.subs.must("Fe");
    let hot = Cell::filled(&w.subs, fe, Form::Rigid, 900.0);
    let cold = Cell::filled(&w.subs, fe, Form::Rigid, 300.0);
    w.fill_rect(0, 0, 64, 64, cold);
    w.fill_rect(0, 0, 32, 64, hot);
    let e0 = w.total_energy();
    for _ in 0..3000 {
        w.step();
    }
    assert_close(e0, w.total_energy(), 1e-5, "energy");
    let t_left = w.cell(30, 32).temperature(&w.subs);
    let t_right = w.cell(33, 32).temperature(&w.subs);
    assert!(t_left < 700.0 && t_right > 500.0, "{t_left} {t_right}");
    assert!(t_left > t_right);
}

#[test]
fn dry_sand_insulates_better_than_wet_sand() {
    let mut w = world(64, 64);
    let dry = powder(&w, "SiO2");
    // The same grains with water in the pores instead of air.
    let (sio2, h2o) = (w.subs.must("SiO2"), w.subs.must("H2O"));
    let grains = dry.amount[dry.find(sio2, Form::Granular).unwrap()];
    let mut wet = Cell::EMPTY;
    wet.add_at(&w.subs, sio2, Form::Granular, grains, ROOM_T);
    wet.add_at(&w.subs, h2o, Form::Liquid, (1.0 - PACKING) / w.subs.props(h2o, Form::Liquid).vm, ROOM_T);
    w.set(1, 1, dry);
    w.set(3, 1, wet);
    w.step();
    let i_dry = w.index(1, 1);
    let i_wet = w.index(3, 1);
    let (k_dry, k_wet) = (w.derived.cond[i_dry], w.derived.cond[i_wet]);
    assert!(k_dry > 0.2 && k_dry < 0.8, "dry sand k = {k_dry}");
    assert!(k_wet > 1.0 && k_wet < 2.5, "wet sand k = {k_wet}");
}

/// A busy mixed scene: every substance and the total energy must come out
/// exactly as they went in.
#[test]
fn mixed_scene_conserves_matter_and_energy() {
    let mut w = world(128, 128);
    w.fill_rect(0, 120, 128, 128, wall(&w, "SiO2"));
    w.fill_rect(30, 60, 34, 120, wall(&w, "Fe"));
    w.fill_rect(10, 10, 30, 40, powder(&w, "SiO2"));
    w.fill_rect(40, 10, 70, 50, liquid(&w, "H2O"));
    w.fill_rect(75, 5, 85, 25, liquid(&w, "Hg"));
    w.fill_rect(50, 60, 60, 70, powder(&w, "Au"));
    let he = w.subs.must("He");
    let hot_he = Cell::gas(&w.subs, &[(he, 1.0)], 600.0, 2.0 * AMBIENT_P);
    w.fill_rect(90, 90, 110, 110, hot_he);
    let fe = w.subs.must("Fe");
    w.fill_rect(100, 40, 110, 50, Cell::filled(&w.subs, fe, Form::Rigid, 1200.0));

    let ids = all_ids(&w);
    let before: Vec<f64> = ids.iter().map(|&id| w.total_moles(id)).collect();
    let e0 = w.total_energy();
    for _ in 0..1500 {
        w.step();
    }
    for (k, &id) in ids.iter().enumerate() {
        let sym = w.subs.get(id).symbol.clone();
        assert_close(before[k], w.total_moles(id), 2e-5, &sym);
    }
    assert_close(e0, w.total_energy(), 2e-5, "energy");
    // Nothing went non-physical along the way.
    for (i, c) in w.cells.iter().enumerate() {
        let t = c.temperature(&w.subs);
        assert!(t.is_finite() && (0.0..5000.0).contains(&t), "cell {i} has T = {t}");
        assert!(c.condensed_volume(&w.subs) < 1.001, "cell {i} is overfilled");
        for s in 0..SLOTS {
            assert!(c.amount[s] >= 0.0 && c.amount[s].is_finite());
        }
    }
}

/// "Block, never destroy": a substance that finds no free slot stays where
/// it is, and both cells are left exactly as they were.
#[test]
fn full_cell_blocks_a_new_substance() {
    use sim::xfer::Pair;
    let w = world(64, 64);
    // Fill every slot of one cell with a different (substance, form).
    let mut full = Cell::EMPTY;
    let mut used = 0;
    'outer: for form in [Form::Gas, Form::Liquid] {
        for id in w.subs.ids() {
            if id == w.subs.must("H2O") {
                continue;
            }
            assert!(full.add_at(&w.subs, id, form, 1e-6, ROOM_T));
            used += 1;
            if used == SLOTS {
                break 'outer;
            }
        }
    }
    assert_eq!(used, SLOTS, "table is too small to fill a cell");
    let water = liquid(&w, "H2O");
    let mut p = Pair::new(water, full, &w.subs);
    assert!(!p.a_to_b(0, 0.01), "transfer into a full cell must be refused");
    let (a, b) = p.finish();
    assert_eq!(a.amount, water.amount);
    assert_eq!(b.amount, full.amount);
    assert_eq!(b.id, full.id);
    assert_eq!(a.energy, water.energy);
    assert_eq!(b.energy, full.energy);
}

/// Lowest and highest gas pressure over all cells that are mostly gas, in atm.
fn pressure_range(w: &World) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for c in &w.cells {
        if c.has_form(Form::Gas) && c.free_space(&w.subs) > 0.5 {
            let p = c.pressure(&w.subs) / sim::ATM;
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    (lo, hi)
}

#[test]
fn pressure_evens_out_within_a_second() {
    let mut w = world(256, 128);
    let squeezed = Cell::gas(&w.subs, &air(&w.subs), ROOM_T, 3.0 * AMBIENT_P);
    w.fill_rect(120, 56, 136, 72, squeezed);
    let n2 = w.subs.must("N2");
    let before = w.total_moles(n2);
    let e0 = w.total_energy();
    for _ in 0..60 {
        w.step();
    }
    assert_close(before, w.total_moles(n2), 1e-5, "nitrogen");
    assert_close(e0, w.total_energy(), 1e-5, "energy");
    // 256 cells at 3 atm in 32768 cells: everything settles near 1.016 atm.
    // Differences below the dead band (`Params::rel_tol` per pair) are left.
    let (lo, hi) = pressure_range(&w);
    assert!(hi - lo < 0.02, "pressure still uneven: {lo}..{hi} atm");
    assert!((lo + hi) / 2.0 > 1.01 && (lo + hi) / 2.0 < 1.02, "{lo}..{hi}");
}

#[test]
fn sealed_box_keeps_its_pressure() {
    let mut w = world(256, 128);
    w.fill_rect(100, 40, 140, 80, wall(&w, "Fe"));
    let squeezed = Cell::gas(&w.subs, &air(&w.subs), ROOM_T, 2.0 * AMBIENT_P);
    w.fill_rect(102, 42, 138, 78, squeezed);
    // Disturb the outside so the pressure pass has work to do all around.
    let puff = Cell::gas(&w.subs, &air(&w.subs), ROOM_T, 1.5 * AMBIENT_P);
    w.fill_rect(60, 50, 80, 70, puff);
    for _ in 0..600 {
        w.step();
    }
    for y in 42..78 {
        for x in 102..138 {
            let p = w.cell(x, y).pressure(&w.subs) / sim::ATM;
            assert!((p - 2.0).abs() < 1e-3, "inside the box at {x},{y}: {p} atm");
        }
    }
    let p_out = w.cell(20, 20).pressure(&w.subs) / sim::ATM;
    assert!(p_out < 1.02, "outside: {p_out} atm");
}

#[test]
fn air_beside_hot_iron_stays_near_ambient_pressure() {
    let mut w = world(256, 256);
    let fe = w.subs.must("Fe");
    w.fill_rect(112, 120, 144, 128, Cell::filled(&w.subs, fe, Form::Rigid, 1500.0));
    let mut worst = 0.0f32;
    for step in 0..900 {
        w.step();
        if step % 30 == 29 {
            let (lo, hi) = pressure_range(&w);
            worst = worst.max(hi - lo);
        }
    }
    // The old neighbour-only rule let 2.3 atm build up beside the bar.
    assert!(worst < 0.04, "pressure spread reached {worst} atm");
    let (_, hi) = pressure_range(&w);
    assert!(hi < 1.1, "pressure rose to {hi} atm");
}
