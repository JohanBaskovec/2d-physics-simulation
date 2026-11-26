//! Per-step derived fields.
//!
//! Temperature, conductivity, heat capacity and gas state are computed once
//! per step from each cell's slots and kept in flat arrays. The heat and gas
//! passes then read one number per neighbour instead of walking eight slots
//! on every lookup.

use crate::cell::Form;
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
    /// [`FIXED`], [`POWDER`] and [`FLOWING`] bits.
    pub flags: Vec<u8>,
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
    pub fn new(n: usize) -> Self {
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
        }
    }
}

pub fn run(world: &mut World) {
    let World { cells, chunks, derived: d, subs, .. } = world;
    let w = chunks.w;
    for cy in 0..chunks.ch {
        for cx in 0..chunks.cw {
            if !chunks.active[cy * chunks.cw + cx] {
                continue;
            }
            for y in cy * CHUNK..(cy + 1) * CHUNK {
                let row = y * w + cx * CHUNK;
                for i in row..row + CHUNK {
                    let c = &cells[i];
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
                    let t = if ncp > 0.0 { (c.energy - nh0) / ncp } else { 0.0 };
                    let gas_vol = (1.0 - cond_vol).max(0.0);
                    let k_gas = if gas_n > 0.0 { gas_k / gas_n } else { 0.0 };
                    let k = if grain_vol > 0.0 {
                        // Grains touch only at small contact points, so heat
                        // has to cross the pore fluid. The geometric mean is
                        // the usual estimate for packed beds; it is what makes
                        // dry sand an insulator and wet sand a fair conductor.
                        (lnk_vol + gas_vol * k_gas.max(VACUUM_PORE_K).ln()).exp()
                    } else {
                        k_vol + gas_vol * k_gas
                    };
                    let v_floor = gas_vol.max(MIN_GAS_VOL);
                    d.temp[i] = t;
                    d.cond[i] = k;
                    d.heat_cap[i] = ncp;
                    let pv = gas_n * R_GAS * t / CELL_M3;
                    d.pressure[i] = pv / v_floor;
                    d.gas_pv[i] = pv;
                    d.gas_vol[i] = gas_vol;
                    d.gas_rho[i] = gas_m / v_floor;
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
                    d.flags[i] = flags;
                    d.fill[i] = (liquid_vol + rigid_vol) / (1.0 - grain_vol).max(crate::EPS_VOL);
                }
            }
        }
    }
}
