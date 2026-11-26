//! Gas movement: pressure-driven flow, buoyancy and diffusion.
//!
//! Pressure is evened out first, on plain numbers, by [`crate::pressure`].
//! This pass then moves the gas itself: once per cell face, by the flux the
//! pressure pass asked for.
//!
//! Simplifications in this version:
//! - Gas has no momentum, so there is no wind that keeps blowing and there
//!   are no sound waves. Pressure just evens out, quickly.
//! - Energy is carried as enthalpy (`cp * T`), so gas does not heat when
//!   compressed or cool when it expands.
//! - The weight of the gas column is ignored (0.12 Pa per cell for air), so
//!   pressure in still air is uniform. Buoyancy is a separate rule
//!   ([`overturn`]) and works within a column only.

use crate::cell::Form;
use crate::derive::{FIXED, OPEN, POWDER};
use crate::world::{Dirty, World};
use crate::xfer::{exchange_gas, move_fast, Pair};
use crate::{CHUNK, SLOTS};

/// Each pair of neighbours diffuses once every this many steps, with the
/// rate scaled up to match. Diffusion is slow, so doing it in turns costs
/// no accuracy and saves most of the work in gas that is merely mixing.
const DIFFUSE_EVERY: usize = 8;

pub fn run(world: &mut World) {
    crate::pressure::summarise(world);
    // Buoyancy goes first because it relocates whole parcels of gas. The
    // pressure pass must see where each parcel ends up, or it would ask a
    // cell to expel gas on behalf of a hot parcel that has since left it.
    overturn(world);
    crate::pressure::solve(world);

    let (w, h) = (world.chunks.w, world.chunks.h);
    let (cw, ch) = (world.chunks.cw, world.chunks.ch);
    // Alternate sweep direction so no side is favoured.
    let flip = world.step_count & 1 == 1;
    let mut dirty = Dirty::new();
    for cy in 0..ch {
        for cx in 0..cw {
            let c = cy * cw + cx;
            if !world.chunks.active[c] {
                continue;
            }
            // Nothing can happen at any face of a chunk with no pressure
            // flux and the same gas throughout.
            let right = (cx + 1 < cw).then(|| c + 1);
            let down = (cy + 1 < ch).then(|| c + cw);
            let flux = world.press.has_flux[c];
            if !flux && world.press.density_uniform(c, right, down, world.params.mix_tol) {
                continue;
            }
            for y in cy * CHUNK..(cy + 1) * CHUNK {
                for k in 0..CHUNK {
                    let x = cx * CHUNK + if flip { CHUNK - 1 - k } else { k };
                    // Nothing crosses the faces of a wall.
                    if world.derived.flags[y * w + x] & FIXED != 0 {
                        continue;
                    }
                    if x + 1 < w {
                        pair(world, &mut dirty, x, y, false, flux);
                    }
                    if y + 1 < h {
                        pair(world, &mut dirty, x, y, true, flux);
                    }
                }
            }
            dirty.flush(&mut world.chunks);
        }
    }
}

/// Buoyancy. Heavier gas above lighter gas overturns at a speed of about
/// sqrt(A * g * L), with A the Atwood number and L one cell. That speed is
/// used as the chance per step that the heavy parcel sinks.
fn overturn(world: &mut World) {
    let (w, h) = (world.chunks.w, world.chunks.h);
    let mix_tol = world.params.mix_tol;
    let mut dirty = Dirty::new();
    for cy in 0..world.chunks.ch {
        for cx in 0..world.chunks.cw {
            let c = cy * world.chunks.cw + cx;
            if !world.chunks.active[c] {
                continue;
            }
            // Gas of one density cannot be top-heavy.
            let down = (cy + 1 < world.chunks.ch).then(|| c + world.chunks.cw);
            if world.press.density_uniform(c, None, down, mix_tol) {
                continue;
            }
            for y in cy * CHUNK..((cy + 1) * CHUNK).min(h - 1) {
                for x in cx * CHUNK..(cx + 1) * CHUNK {
                    let i = y * w + x;
                    let d = &world.derived;
                    if d.flags[i] & d.flags[i + w] & OPEN == 0 {
                        continue;
                    }
                    let (ra, rb) = (d.gas_rho[i], d.gas_rho[i + w]);
                    if ra - rb <= mix_tol * ra {
                        continue;
                    }
                    // Unstable but not overturning this step: keep awake.
                    dirty.add_pair(x, y, x, y + 1);
                    let atwood = (ra - rb) / (ra + rb);
                    if world.rng.unit() < (atwood * crate::GRAVITY).sqrt() {
                        sink(world, i, i + w);
                    }
                }
            }
            dirty.flush(&mut world.chunks);
        }
    }
}

/// Exchange gas between the cell at (x, y) and its right neighbour, or its
/// lower neighbour if `vertical`.
#[inline]
fn pair(world: &mut World, dirty: &mut Dirty, x: usize, y: usize, vertical: bool, has_flux: bool) {
    let w = world.chunks.w;
    let i = y * w + x;
    let (j, jx, jy) = if vertical { (i + w, x, y + 1) } else { (i + 1, x + 1, y) };
    let d = &world.derived;
    let (ra, rb) = (d.gas_rho[i], d.gas_rho[j]);
    let params = &world.params;
    // Flux the pressure pass wants through this face, as `pV` going from
    // this cell to the neighbour (negative: the other way).
    let flux = if !has_flux {
        0.0
    } else if vertical {
        world.press.fy[i]
    } else {
        world.press.fx[i]
    };
    let (ga, gb) = (d.gas_pv[i], d.gas_pv[j]);
    let push = if flux > 0.0 { flux > params.gas_flow_tol * ga } else { -flux > params.gas_flow_tol * gb };
    let mix = (ra - rb).abs() > params.mix_tol * ra.max(rb);
    // No flow asked for and the same density: nothing to do.
    if !push && !mix {
        return;
    }
    let (va, vb) = (d.gas_vol[i], d.gas_vol[j]);
    let v_min = va.min(vb);
    if d.flags[i] & FIXED != 0 || d.flags[j] & FIXED != 0 || va + vb <= 0.0 {
        return;
    }

    // Fraction of each cell's gas that moves to the other.
    let (mut fa, mut fb) = (0.0f32, 0.0f32);

    // Pressure: the giving cell hands over the share of its gas that carries
    // the requested flux. A cell that started the step empty passes on a
    // share of whatever has reached it so far.
    if push {
        let limit = params.gas_max_flow;
        if flux > 0.0 {
            fa = if ga > 0.0 { (flux / ga).min(limit) } else { limit };
        } else {
            fb = if gb > 0.0 { (-flux / gb).min(limit) } else { limit };
        }
    }

    if mix && v_min > 0.0 {
        // This pair is still mixing, so its chunk must stay awake even on
        // the steps where it is not this pair's turn to diffuse.
        dirty.add_pair(x, y, jx, jy);

        // Diffusion: neighbours trade a small, equal volume of gas.
        let turn = (x + y + vertical as usize * (DIFFUSE_EVERY / 2) + world.step_count as usize)
            % DIFFUSE_EVERY
            == 0;
        if turn {
            let mut swap = params.gas_diffusion * DIFFUSE_EVERY as f32;
            if (d.flags[i] | d.flags[j]) & POWDER != 0 {
                swap *= params.pore_gas_flow;
            }
            fa += swap * v_min / va;
            fb += swap * v_min / vb;
        }
    }
    if fa <= 0.0 && fb <= 0.0 {
        return;
    }
    let (ta, tb) = (d.temp[i], d.temp[j]);

    // `j` is always after `i`, so the slice can be split between them.
    let (head, tail) = world.cells.split_at_mut(j);
    let (a, b) = (&mut head[i], &mut tail[0]);
    let subs = &world.subs;
    // Amounts are read before anything moves, so gas that arrives in this
    // exchange is not sent straight back.
    let mut gas_a = [0.0f32; SLOTS];
    let mut gas_b = [0.0f32; SLOTS];
    for s in 0..SLOTS {
        if a.form[s] == Form::Gas {
            gas_a[s] = a.amount[s] * fa;
        }
        if b.form[s] == Form::Gas {
            gas_b[s] = b.amount[s] * fb;
        }
    }
    let mut moved = false;
    for s in 0..SLOTS {
        if gas_a[s] > 0.0 {
            moved |= move_fast(a, b, s, gas_a[s], ta, subs);
        }
    }
    for s in 0..SLOTS {
        if gas_b[s] > 0.0 {
            moved |= move_fast(b, a, s, gas_b[s], tb, subs);
        }
    }
    if moved {
        // A cell with no matter cannot hold energy.
        if a.is_empty() {
            b.energy += a.energy;
            a.energy = 0.0;
        } else if b.is_empty() {
            a.energy += b.energy;
            b.energy = 0.0;
        }
        dirty.add_pair(x, y, jx, jy);
        world.chunks.stale[i] = 1;
        world.chunks.stale[j] = 1;
    }
}

/// The gas in cell `i` is heavier than the gas in `below`, the cell under
/// it. Let it sink by trading places, whole, with gas from the run of
/// lighter gas beneath it: the lowest cell there that is clearly light
/// (closer in density to the lightest gas in the run than to the parcel
/// coming down).
///
/// Trading with the adjacent cell only would let the heavy gas trickle down
/// through the light gas one cell at a time, mixing the two. Reaching for
/// the far end of the light gas moves it up as one body instead. (Weather
/// models do much the same and call it convective adjustment.) There is
/// still no wind: gas only rises and sinks within its own column.
fn sink(world: &mut World, i: usize, below: usize) {
    let w = world.chunks.w;
    let d = &world.derived;
    // "Lighter" has to mean lighter by more than the noise in the density
    // field, or the parcel would sink straight through still air.
    let heavy = d.gas_rho[i] * (1.0 - world.params.mix_tol);
    let mut lightest = d.gas_rho[below];
    let mut end = below;
    for _ in 1..world.params.gas_sink_reach {
        let next = end + w;
        // Stop at the floor, at anything that is not open gas, and at gas
        // that is no lighter than the parcel coming down.
        if next >= world.cells.len() || d.flags[next] & OPEN == 0 || d.gas_rho[next] >= heavy {
            break;
        }
        end = next;
        lightest = lightest.min(d.gas_rho[end]);
    }
    let clearly_light = 0.5 * (heavy + lightest);
    let mut target = end;
    while target > below && d.gas_rho[target] > clearly_light {
        target -= w;
    }
    // Equal volumes trade places. Both cells are open, so that is nearly all
    // of the gas in each. A whole cell's gas moves, so use the exact (f64)
    // exchange.
    let (va, vb) = (d.gas_vol[i], d.gas_vol[target]);
    let v_min = va.min(vb);
    let mut p = Pair::new(world.cells[i], world.cells[target], &world.subs);
    if exchange_gas(&mut p, v_min / va, v_min / vb) {
        let (a, b) = p.finish();
        world.cells[i] = a;
        world.cells[target] = b;
        // The snapshot follows the gas, so the rest of this step sees each
        // parcel where it now is.
        let d = &mut world.derived;
        d.temp.swap(i, target);
        d.cond.swap(i, target);
        d.heat_cap.swap(i, target);
        d.pressure.swap(i, target);
        d.gas_rho.swap(i, target);
        d.gas_pv.swap(i, target);
        world.chunks.touch_index(i);
        world.chunks.touch_index(target);
    }
}
