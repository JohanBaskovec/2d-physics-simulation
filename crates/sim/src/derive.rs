//! Per-step derived fields.
//!
//! Temperature, conductivity, heat capacity and gas state are computed from
//! each cell's slots and kept in flat arrays, refreshed at the start of a
//! step for the cells that changed. The heat and gas
//! passes then read one number per neighbour instead of walking eight slots
//! on every lookup.

use crate::cell::{Cell, Form};
use crate::substance::Substances;
use crate::world::World;
use crate::{CELL_M3, CHUNK, R_GAS, SLOTS};

/// Gas volume is floored at this when computing pressure, so a trace of gas
/// squeezed by rounding gets a large but finite pressure and is pushed out.
pub const MIN_GAS_VOL: f32 = 1.0e-3;

/// Conductivity assumed for empty pores when taking the geometric mean, so a
/// powder in vacuum still conducts a little through grain contacts.
const VACUUM_PORE_K: f32 = 1.0e-3;

pub struct Derived {
    /// Temperature, K. Zero for vacuum.
    pub temp: Vec<f32>,
    /// Effective thermal conductivity, W/(m K).
    pub cond: Vec<f32>,
    /// Heat capacity of the whole cell, J/K.
    pub heat_cap: Vec<f32>,
    /// Gas pressure, Pa.
    pub pressure: Vec<f32>,
    /// Volume fraction available to gas.
    pub gas_vol: Vec<f32>,
    /// Gas density, g/cm^3.
    pub gas_rho: Vec<f32>,
    /// Pressure times gas volume, Pa x cell volume. Proportional to the
    /// gas's `n * T`, which is what moves when gas is transferred.
    pub gas_pv: Vec<f32>,
    /// Liquid fill level: volume of flowing matter over the volume not
    /// taken by grains.
    pub fill: Vec<f32>,
    /// [`FIXED`], [`POWDER`], [`FLOWING`] and [`OPEN`] bits.
    pub flags: Vec<u8>,
    /// Per chunk: a counter that goes up whenever any cell's flags in the
    /// chunk change. Lets other passes keep results that depend only on
    /// the flags (where the walls and the open gas are) until they move.
    pub flags_version: Vec<u64>,
}

/// The cell is fixed in place (rigid volume above 50%).
pub const FIXED: u8 = 1;
/// The cell holds grains.
pub const POWDER: u8 = 2;
/// The cell holds liquid, or loose rigid pieces carried by it.
pub const FLOWING: u8 = 4;
/// The cell is open gas (or vacuum): gas space fills it and it is neither
/// fixed nor powder. Gas can shift freely through such cells.
pub const OPEN: u8 = 8;

impl Derived {
    pub fn new(n: usize, chunks: usize) -> Self {
        Derived {
            temp: vec![0.0; n],
            cond: vec![0.0; n],
            heat_cap: vec![0.0; n],
            pressure: vec![0.0; n],
            gas_vol: vec![0.0; n],
            gas_rho: vec![0.0; n],
            gas_pv: vec![0.0; n],
            fill: vec![0.0; n],
            flags: vec![0; n],
            flags_version: vec![0; chunks],
        }
    }
}

/// A chunk's cells are all recomputed once every this many steps whether
/// marked or not. Temperatures kept up to date by adding each step's heat
/// would otherwise drift by rounding, slowly and without limit.
const REFRESH_EVERY: u64 = 64;

/// Everything derived from one cell.
#[derive(Clone, Copy, Debug)]
struct Values {
    temp: f32,
    cond: f32,
    heat_cap: f32,
    pressure: f32,
    gas_vol: f32,
    gas_rho: f32,
    gas_pv: f32,
    fill: f32,
    flags: u8,
}

/// Bring the derived fields up to date for every active chunk.
///
/// Only cells whose contents changed are recomputed. Most cells in an awake
/// chunk are unchanged from one step to the next (rock around a cave, the
/// inside of a pool), and for those the only thing that moves is
/// temperature, which the heat pass keeps current as it applies each
/// step's heat.
pub fn run(world: &mut World) {
    let step = world.step_count;
    let World { cells, chunks, derived: d, subs, .. } = world;
    let w = chunks.w;
    for cy in 0..chunks.ch {
        for cx in 0..chunks.cw {
            let c = cy * chunks.cw + cx;
            if !chunks.active[c] {
                continue;
            }
            let everything = (step + c as u64) % REFRESH_EVERY == 0;
            let mut flags_changed = false;
            for y in cy * CHUNK..(cy + 1) * CHUNK {
                let row = y * w + cx * CHUNK;
                // Look at eight marks at a time; unmarked stretches are the
                // common case.
                for group in (row..row + CHUNK).step_by(8) {
                    let marks: [u8; 8] = chunks.stale[group..group + 8].try_into().expect("eight cells");
                    if !everything && u64::from_ne_bytes(marks) == 0 {
                        continue;
                    }
                    for i in group..group + 8 {
                        if everything || chunks.stale[i] != 0 {
                            chunks.stale[i] = 0;
                            let v = compute(&cells[i], subs);
                            flags_changed |= v.flags != d.flags[i];
                            store(d, i, v);
                        }
                    }
                }
            }
            if flags_changed {
                d.flags_version[c] += 1;
            }
        }
    }
}

/// Recompute every cell of every active chunk and panic if a kept value
/// disagrees. Catches a change to a cell that was not marked.
pub fn verify(world: &World) {
    let World { cells, chunks, derived: d, subs, .. } = world;
    let w = chunks.w;
    let close = |a: f32, b: f32, rel: f32| (a - b).abs() <= rel * a.abs().max(b.abs()) + 1.0e-12;
    for cy in 0..chunks.ch {
        for cx in 0..chunks.cw {
            if !chunks.active[cy * chunks.cw + cx] {
                continue;
            }
            for y in cy * CHUNK..(cy + 1) * CHUNK {
                for x in cx * CHUNK..(cx + 1) * CHUNK {
                    let i = y * w + x;
                    let v = compute(&cells[i], subs);
                    // Temperature and what follows from it are carried
                    // forward by addition, so allow for rounding. The rest
                    // must match exactly.
                    let ok = d.flags[i] == v.flags
                        && d.cond[i] == v.cond
                        && d.heat_cap[i] == v.heat_cap
                        && d.gas_vol[i] == v.gas_vol
                        && d.gas_rho[i] == v.gas_rho
                        && d.fill[i] == v.fill
                        && close(d.temp[i], v.temp, 1.0e-4)
                        && close(d.pressure[i], v.pressure, 1.0e-4)
                        && close(d.gas_pv[i], v.gas_pv, 1.0e-4);
                    assert!(
                        ok,
                        "derived values out of date at ({x}, {y}), step {}:\n  kept:  T {} k {} C {} p {} vol {} rho {} pv {} fill {} flags {}\n  fresh: {v:?}",
                        world.step_count, d.temp[i], d.cond[i], d.heat_cap[i], d.pressure[i], d.gas_vol[i], d.gas_rho[i], d.gas_pv[i], d.fill[i], d.flags[i],
                    );
                }
            }
        }
    }
}

#[inline(always)]
fn store(d: &mut Derived, i: usize, v: Values) {
    d.temp[i] = v.temp;
    d.cond[i] = v.cond;
    d.heat_cap[i] = v.heat_cap;
    d.pressure[i] = v.pressure;
    d.gas_vol[i] = v.gas_vol;
    d.gas_rho[i] = v.gas_rho;
    d.gas_pv[i] = v.gas_pv;
    d.fill[i] = v.fill;
    d.flags[i] = v.flags;
}

#[inline(always)]
fn compute(c: &Cell, subs: &Substances) -> Values {
    let (mut ncp, mut nh0) = (0.0f32, 0.0f32);
    let (mut cond_vol, mut k_vol, mut lnk_vol) = (0.0f32, 0.0f32, 0.0f32);
    let (mut gas_n, mut gas_m, mut gas_k) = (0.0f32, 0.0f32, 0.0f32);
    let (mut grain_vol, mut rigid_vol, mut liquid_vol) = (0.0f32, 0.0f32, 0.0f32);
    for s in 0..SLOTS {
        let form = c.form[s];
        if form == Form::None {
            continue;
        }
        let n = c.amount[s];
        let p = subs.props(c.id[s], form);
        ncp += n * p.cp;
        nh0 += n * p.h0;
        let v = n * p.vm;
        cond_vol += v;
        k_vol += v * p.k;
        lnk_vol += v * p.ln_k;
        match form {
            Form::Gas => {
                gas_n += n;
                gas_m += n * p.molar_mass;
                gas_k += n * p.k;
            }
            Form::Granular => grain_vol += v,
            Form::Rigid => rigid_vol += v,
            Form::Liquid => liquid_vol += v,
            Form::None => {}
        }
    }
    let temp = if ncp > 0.0 { (c.energy - nh0) / ncp } else { 0.0 };
    let gas_vol = (1.0 - cond_vol).max(0.0);
    let k_gas = if gas_n > 0.0 { gas_k / gas_n } else { 0.0 };
    let cond = if grain_vol > 0.0 {
        // Grains touch only at small contact points, so heat has to cross
        // the pore fluid. The geometric mean is the usual estimate for
        // packed beds; it is what makes dry sand an insulator and wet sand
        // a fair conductor.
        (lnk_vol + gas_vol * k_gas.max(VACUUM_PORE_K).ln()).exp()
    } else {
        k_vol + gas_vol * k_gas
    };
    let v_floor = gas_vol.max(MIN_GAS_VOL);
    let gas_pv = gas_n * R_GAS * temp / CELL_M3;
    let mut flags = 0;
    if grain_vol > 0.0 {
        flags |= POWDER;
    } else if rigid_vol > 0.5 {
        flags |= FIXED;
    }
    if flags & FIXED == 0 && liquid_vol + rigid_vol > 0.0 {
        flags |= FLOWING;
    }
    if flags & (FIXED | POWDER) == 0 && gas_vol >= 0.99 {
        flags |= OPEN;
    }
    Values {
        temp,
        cond,
        heat_cap: ncp,
        pressure: gas_pv / v_floor,
        gas_vol,
        gas_rho: gas_m / v_floor,
        gas_pv,
        fill: (liquid_vol + rigid_vol) / (1.0 - grain_vol).max(crate::EPS_VOL),
        flags,
    }
}
