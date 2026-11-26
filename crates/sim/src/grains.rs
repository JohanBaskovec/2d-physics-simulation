//! Powder movement. Grains move as whole cells, as in a falling-sand game:
//! a powder cell trades places with the fluid cell it falls into.

use crate::cell::{Cell, Kind};
use crate::derive::POWDER;
use crate::substance::Substances;
use crate::world::World;
use crate::xfer::{exchange_vertical, Pair};
use crate::{CELL_CM, CHUNK, DT, GRAVITY};

/// Drag coefficient of a rough sphere at the speeds grains fall.
const DRAG_COEFF: f32 = 0.44;

pub fn run(world: &mut World) {
    let w = world.chunks.w;
    let flip = world.step_count & 1 == 1;
    let stamp = stamp_for(world.step_count);
    // Bottom row first, so a cell that falls is not visited again.
    for cy in (0..world.chunks.ch).rev() {
        for ly in (0..CHUNK).rev() {
            let y = cy * CHUNK + ly;
            for cx in 0..world.chunks.cw {
                if !world.chunks.active[cy * world.chunks.cw + cx] {
                    continue;
                }
                for k in 0..CHUNK {
                    let x = cx * CHUNK + if flip { CHUNK - 1 - k } else { k };
                    let i = y * w + x;
                    // The snapshot flag is a cheap first test. A cell that
                    // only became powder this step was moved here, and is done.
                    if world.derived.flags[i] & POWDER == 0 || !world.cells[i].is_powder() {
                        continue;
                    }
                    if world.stamp[i] == stamp {
                        // Already moved this step (or a stale mark from 255
                        // steps ago). Look again next step either way.
                        world.chunks.touch(x, y);
                    } else {
                        move_powder(world, x, y);
                    }
                }
            }
        }
    }
}

/// Mark for "moved during this step". Never zero, which is what a cell that
/// has not moved yet holds.
#[inline]
fn stamp_for(step: u64) -> u8 {
    (step % 255) as u8 + 1
}

/// Can grains of density `rho_p` sink into this cell?
#[inline]
fn passable(c: &Cell, rho_p: f32, subs: &Substances) -> bool {
    c.kind(subs) == Kind::Fluid && c.mass(subs) < rho_p
}

/// Speed, in cells per step, at which drag balances weight for a grain of
/// density `rho_p` falling through fluid of density `rho_f` (both g/cm^3).
fn terminal_speed(rho_p: f32, rho_f: f32, grain_cm: f32) -> f32 {
    let rho_f = rho_f.max(1.0e-9);
    let v_cm_s = (4.0 * 981.0 * grain_cm * (rho_p - rho_f).max(0.0) / (3.0 * DRAG_COEFF * rho_f)).sqrt();
    v_cm_s * DT / CELL_CM
}

fn move_powder(world: &mut World, x: usize, y: usize) {
    let (w, h) = (world.chunks.w, world.chunks.h);
    let i = y * w + x;
    let c = world.cells[i];
    let rho_p = c.grain_density(&world.subs);
    let stamp = stamp_for(world.step_count);

    // Straight down.
    if y + 1 < h && passable(&world.cells[i + w], rho_p, &world.subs) {
        let rho_f = world.cells[i + w].mass(&world.subs);
        let vy = (c.vy() + GRAVITY * (1.0 - rho_f / rho_p))
            .min(terminal_speed(rho_p, rho_f, world.params.grain_cm))
            .min(world.params.max_speed);
        let steps = world.rng.round(vy);
        let mut cur = i;
        let mut vy_end = vy;
        for _ in 0..steps {
            let below = cur + w;
            if below >= w * h || !passable(&world.cells[below], rho_p, &world.subs) {
                vy_end = 0.0;
                break;
            }
            swap_down(world, cur, below);
            cur = below;
        }
        world.cells[cur].set_vy(vy_end);
        world.stamp[cur] = stamp;
        // Still in flight even if it did not advance this step.
        world.chunks.touch_index(cur);
        return;
    }

    // Blocked below: slide diagonally if the way is open.
    if y + 1 < h {
        let first = if world.rng.coin() { -1i32 } else { 1 };
        for dx in [first, -first] {
            let nx = x as i32 + dx;
            if nx < 0 || nx >= w as i32 {
                continue;
            }
            let side = y * w + nx as usize;
            let diag = side + w;
            if world.cells[side].kind(&world.subs) == Kind::Fluid
                && passable(&world.cells[diag], rho_p, &world.subs)
            {
                swap_down(world, i, diag);
                world.cells[diag].vel = [0, 0];
                world.stamp[diag] = stamp;
                return;
            }
        }
    }

    // At rest.
    if c.vel != [0, 0] {
        world.cells[i].vel = [0, 0];
    }

    // Float: a fluid cell above that is heavier than this whole powder cell
    // sinks past it.
    if y > 0 {
        let above = world.cells[i - w];
        if above.kind(&world.subs) == Kind::Fluid
            && above.has_flowing()
            && above.mass(&world.subs) > c.mass(&world.subs)
        {
            let mut p = Pair::new(c, above, &world.subs);
            exchange_vertical(&mut p, f32::INFINITY);
            let (upper, lower) = p.finish();
            world.cells[i - w] = upper;
            world.cells[i] = lower;
            world.stamp[i - w] = stamp;
            world.chunks.touch_index(i);
            world.chunks.touch_index(i - w);
        }
    }
}

/// Move the powder cell at `from` into the fluid cell at `to`, which is on
/// the row below. The fluid cell moves up to `from`.
///
/// After the swap, fluid in the upper cell that is heavier than what fills
/// the grains' pores trades places with it. So dry sand falling into a full
/// water cell ends up as 60% sand with 40% water, and the cell above holds
/// the other 60% of the water plus the air that was in the pores.
fn swap_down(world: &mut World, from: usize, to: usize) {
    let powder = world.cells[from];
    let fluid = world.cells[to];
    let mut p = Pair::new(fluid, powder, &world.subs);
    exchange_vertical(&mut p, f32::INFINITY);
    let (upper, lower) = p.finish();
    world.cells[from] = upper;
    world.cells[to] = lower;
    world.chunks.touch_index(from);
    world.chunks.touch_index(to);
}
