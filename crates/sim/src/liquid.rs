//! Liquid movement. Liquid amounts are continuous: a cell can be partly
//! full. Liquids fall into gas space, sort by density, and level sideways.
//!
//! Not in this version: pressure at depth. Liquids here are incompressible
//! and level only across their surface, so two connected vessels do not
//! equalise through a pipe at the bottom yet.

use crate::cell::{flows, Cell, Form, Kind};
use crate::derive::{FIXED, FLOWING, POWDER};
use crate::substance::Substances;
use crate::world::World;
use crate::xfer::{equalize_lateral, exchange_vertical, Pair};
use crate::{CHUNK, EPS_VOL, GRAVITY, SLOTS};

pub fn run(world: &mut World) {
    let w = world.chunks.w;
    let flip = world.step_count & 1 == 1;
    // Bottom row first, so liquid that falls is not moved twice in a step.
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
                    // The snapshot flag is a cheap first test. Liquid that
                    // only arrived in a cell this step has already moved.
                    if world.derived.flags[i] & FLOWING != 0
                        && world.cells[i].has_flowing()
                        && !at_rest(world, x, y)
                    {
                        move_liquid(world, x, y);
                    }
                }
            }
        }
    }
}

/// Cheap test for a cell deep inside still liquid: it is full, and the
/// cells below and to both sides are walls or full cells holding exactly the
/// same substances. Nothing can move in such a cell, and most liquid cells
/// in a settled pool are like this, so they are skipped. Reads the
/// start-of-step snapshot; a cell that changed this step is picked up next
/// step.
fn at_rest(world: &World, x: usize, y: usize) -> bool {
    let (w, h) = (world.chunks.w, world.chunks.h);
    let d = &world.derived;
    let i = y * w + x;
    if d.gas_vol[i] > EPS_VOL {
        return false;
    }
    let me = &world.cells[i];
    let blocked = |j: usize| {
        d.flags[j] & FIXED != 0
            || (d.gas_vol[j] <= EPS_VOL && world.cells[j].id == me.id && world.cells[j].form == me.form)
    };
    (y + 1 >= h || blocked(i + w)) && (x == 0 || blocked(i - 1)) && (x + 1 >= w || blocked(i + 1))
}

fn move_liquid(world: &mut World, x: usize, y: usize) {
    let (w, h) = (world.chunks.w, world.chunks.h);
    let i = y * w + x;
    let mut fell = false;

    if y + 1 < h
        && world.derived.flags[i + w] & FIXED == 0
        && may_sink(&world.cells[i], &world.cells[i + w], &world.subs)
    {
        if world.cells[i].is_powder() || world.cells[i + w].is_powder() {
            // Through pores: slow seepage, no free fall.
            fell = fall_once(world, i, world.params.seep_vol) > 0.0;
        } else {
            // Free fall: follow the liquid down for as many cells as its
            // speed allows. A resting liquid always gets one attempt, which
            // is also how we learn whether it is supported.
            let vy = (world.cells[i].vy() + GRAVITY).min(world.params.max_speed);
            let steps = world.rng.round(vy).max(1);
            let mut cur = i;
            for _ in 0..steps {
                let below = cur + w;
                if below >= w * h {
                    break;
                }
                if world.derived.flags[below] & FIXED != 0 || world.cells[below].is_powder() {
                    break;
                }
                if fall_once(world, cur, f32::INFINITY) <= 0.0 {
                    break;
                }
                fell = true;
                if world.cells[cur].has_flowing() {
                    break; // only part of it fitted below
                }
                // The liquid has left this cell and takes its speed with it.
                world.cells[cur].vel = [0, 0];
                cur = below;
            }
            if fell {
                world.cells[cur].set_vy(vy);
            }
        }
    }

    // Liquid that still sits here on something it cannot sink into is
    // supported. That includes the part left over after topping up the cell
    // below, so a tall column drains from every exposed row at once.
    let supported = world.cells[i].has_flowing()
        && (y + 1 >= h || {
            let below = &world.cells[i + w];
            below.kind(&world.subs) != Kind::Fluid || below.free_space(&world.subs) < 0.01
        });
    if supported {
        // Come to rest and level with the neighbours.
        if !fell && world.cells[i].vel != [0, 0] {
            world.cells[i].vel = [0, 0];
        }
        let first = if world.rng.coin() { -1i32 } else { 1 };
        for dx in [first, -first] {
            let Some(j) = level_target(world, x, y, dx) else { continue };
            let (a, b) = (world.cells[i], world.cells[j]);
            let limit = if a.is_powder() || b.is_powder() {
                world.params.seep_vol
            } else {
                f32::INFINITY
            };
            let mut p = Pair::new(a, b, &world.subs);
            if equalize_lateral(&mut p, limit, world.params.level_tol) {
                let (a, b) = p.finish();
                world.cells[i] = a;
                world.cells[j] = b;
                world.chunks.touch_index(i);
                world.chunks.touch_index(j);
            }
        }
    }
}

/// Cheap test for "could anything in `a` sink into `b`, the cell below?".
/// It may say yes when the answer is no, never the reverse. This keeps the
/// inside of a still pool from paying for a full exchange every step.
fn may_sink(a: &Cell, b: &Cell, subs: &Substances) -> bool {
    if b.free_space(subs) > EPS_VOL {
        return true;
    }
    // `b` is full, so only a denser-for-lighter swap is possible. Two cells
    // holding the same single flowing substance have nothing to swap.
    let same_layout = a.id == b.id && a.form == b.form;
    let flowing = a.form.iter().filter(|&&f| flows(f)).count();
    !(same_layout && flowing <= 1)
}

/// Fraction of the cell's non-grain volume that is filled with liquid.
fn fill(c: &Cell, subs: &Substances) -> f32 {
    let (mut flowing, mut grains) = (0.0f32, 0.0f32);
    for s in 0..SLOTS {
        let v = c.slot_volume(subs, s);
        if flows(c.form[s]) {
            flowing += v;
        } else if c.form[s] == Form::Granular {
            grains += v;
        }
    }
    flowing / (1.0 - grains).max(EPS_VOL)
}

/// Pick the cell that the liquid at (x, y) levels with in direction `dx`.
///
/// Levelling only with the adjacent cell spreads liquid like diffusion,
/// which takes minutes to flatten a pool a metre wide. Real liquid levels at
/// about sqrt(g * depth), around a cell per step. So the liquid looks along
/// its row, up to `liquid_reach` cells, and levels with the emptiest cell it
/// can reach through open, supported cells. There is no momentum in this
/// version, so liquids level quickly but do not slosh.
///
/// The search reads the start-of-step snapshot, which is cheap but can be a
/// little out of date. That is safe: it only picks a partner, and the
/// levelling itself works on the real cells.
fn level_target(world: &World, x: usize, y: usize, dx: i32) -> Option<usize> {
    let (w, h) = (world.chunks.w, world.chunks.h);
    let d = &world.derived;
    let i = y * w + x;
    let from_powder = world.cells[i].is_powder();
    let my_fill = fill(&world.cells[i], &world.subs);
    let mut best: Option<(usize, f32)> = None;
    for r in 1..=world.params.liquid_reach as i32 {
        let nx = x as i32 + dx * r;
        if nx < 0 || nx >= w as i32 {
            break;
        }
        let j = y * w + nx as usize;
        if d.flags[j] & FIXED != 0 {
            break;
        }
        let f = d.fill[j];
        if f < my_fill && best.map_or(true, |(_, bf)| f < bf) {
            best = Some((j, f));
        }
        // Through pores liquid only seeps to the adjacent cell.
        if from_powder || d.flags[j] & POWDER != 0 {
            break;
        }
        // Do not reach across a drop: liquid arriving here would fall.
        if y + 1 < h && d.flags[j + w] & (FIXED | POWDER) == 0 && d.gas_vol[j + w] > 0.01 {
            break;
        }
    }
    best.map(|(j, _)| j)
}

/// Exchange fluid between cell `i` and the cell below it. Returns the volume
/// that moved down.
fn fall_once(world: &mut World, i: usize, max_vol: f32) -> f32 {
    let j = i + world.chunks.w;
    let mut p = Pair::new(world.cells[i], world.cells[j], &world.subs);
    let moved = exchange_vertical(&mut p, max_vol);
    if moved > 0.0 {
        let (a, b) = p.finish();
        world.cells[i] = a;
        world.cells[j] = b;
        world.chunks.touch_index(i);
        world.chunks.touch_index(j);
    }
    moved
}
