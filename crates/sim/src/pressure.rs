//! Fast pressure equalisation for gas.
//!
//! Sound crosses the whole world in well under a step, so pressure in any
//! connected body of gas should even out almost at once. Letting gas flow
//! only between neighbours spreads pressure like slow diffusion instead: a
//! heated pocket stays at twice ambient for minutes.
//!
//! This pass works on plain numbers, not on matter. It takes each cell's
//! gas pressure and relaxes that field towards uniform by pairing cells at
//! several distances: 1, 8 and 64 cells apart along a row or column. A
//! distant pair is only used when every cell between the two is open gas,
//! so the exchange stands for gas shifting along a real, unobstructed path,
//! and the amount is recorded as a flux through every cell face on that
//! path.
//!
//! The gas pass then moves real gas once per face, by the total flux through
//! it. Each cell passes a little gas to the next, so composition and heat
//! only ever move to a neighbour; nothing jumps. Because a flux is only
//! ever applied across a face between two cells that are not walls, gas
//! cannot leak out of a sealed space however the relaxation behaves.
//!
//! Most of the world is calm most of the time, so the work is done per
//! chunk and only where needed. Each chunk keeps the lowest and highest
//! pressure in it. A chunk starts relaxing when the spread across it and
//! its right and lower neighbours exceeds `Params::rel_tol`, and then keeps
//! going until its pairs agree to a quarter of that. Between those bouts it
//! costs almost nothing. The price is a dead band: pressure differences up
//! to about `rel_tol` (0.2%) are left standing.

use crate::derive::{Derived, FIXED, OPEN, POWDER};
use crate::world::{Off, World};
use rayon::prelude::*;
use crate::{CHUNK, EPS_VOL};

/// Distances of the long pairs. Each step runs one of these, in turn, and
/// then the adjacent pairs, so a full cycle takes two steps (1/30 s). That
/// is still far quicker than anything else in the simulation and costs less
/// than half of running every distance every step.
const LONG: [usize; 2] = [8, 64];

/// Limit on what long pairs may push through one cell face in a step, as a
/// share of a cell's gas. Every cell on a row can be the start of a long
/// pair, and up to `r` of those paths cross any one face, so each pair is
/// capped at this share divided by its distance `r`.
///
/// The limit exists because the gas pass moves real gas one face at a time,
/// each cell handing on a share of what it holds at that moment, which is
/// only accurate while the share is small. So long pairs do the gentle,
/// far-reaching evening out, and sharp local differences are left to
/// adjacent pairs.
const LONG_FACE_SHARE: f32 = 0.25;

/// The adjacent-pair pass is repeated for a chunk, up to this many times,
/// while some pair in it still moved more than `REPEAT_ABOVE` of its
/// pressure. This smooths the worst of a sharp local difference (gas
/// touching something very hot, a burst) within the step.
const MAX_LOCAL_PASSES: usize = 6;
const REPEAT_ABOVE: f32 = 0.004;

/// How far each pair moves towards equal pressure. Pairs are relaxed one
/// after another in place, which is stable for any value up to 1.
const RELAX: f32 = 0.8;

/// A relaxing chunk keeps going until its pairs agree to this share of
/// `Params::rel_tol`.
const SETTLE: f32 = 0.25;

/// Working arrays for the pressure pass.
pub struct Scratch {
    /// Gas pressure of each cell as the relaxation proceeds, Pa.
    p: Vec<f32>,
    /// Flux of `pV` this step from each cell to its right neighbour. Only
    /// meaningful in chunks where `has_flux` is set.
    pub fx: Vec<f32>,
    /// Flux of `pV` this step from each cell to the cell below.
    pub fy: Vec<f32>,
    /// Number of open cells in a row starting here and going right.
    run_right: Vec<u8>,
    /// Number of open cells in a column starting here and going down.
    run_down: Vec<u8>,
    /// Per chunk: the flags versions (own, right neighbour, lower
    /// neighbour) the runs above were worked out from. Kept between steps,
    /// so runs are only redone when walls or open space have moved.
    run_key: Vec<[u64; 3]>,
    /// Per chunk: the longest run in `run_right` and in `run_down`, so a
    /// chunk with no run long enough for this step's distance is skipped.
    run_max: Vec<[u8; 2]>,
    /// Per chunk: the flags version at which `fx` and `fy` were last
    /// cleared from edge to edge. See the note on clearing in `solve`.
    zero_key: Vec<u64>,
    /// Per chunk: lowest and highest gas pressure, as of the last step the
    /// chunk was active. Kept between steps; a sleeping chunk's cells do
    /// not change, so its entry stays true.
    p_min: Vec<f32>,
    p_max: Vec<f32>,
    /// Per chunk: lowest and highest gas density, kept the same way.
    pub rho_min: Vec<f32>,
    pub rho_max: Vec<f32>,
    /// Per chunk: in the middle of evening out its pressure. Kept between
    /// steps.
    relaxing: Vec<bool>,
    /// Per chunk: `fx` and `fy` hold this step's fluxes here.
    pub has_flux: Vec<bool>,
}

impl Scratch {
    pub fn new(cells: usize, chunks: usize) -> Self {
        Scratch {
            p: vec![0.0; cells],
            fx: vec![0.0; cells],
            fy: vec![0.0; cells],
            run_right: vec![0; cells],
            run_down: vec![0; cells],
            run_key: vec![[u64::MAX; 3]; chunks],
            run_max: vec![[0; 2]; chunks],
            zero_key: vec![u64::MAX; chunks],
            p_min: vec![0.0; chunks],
            p_max: vec![0.0; chunks],
            rho_min: vec![0.0; chunks],
            rho_max: vec![0.0; chunks],
            relaxing: vec![false; chunks],
            has_flux: vec![false; chunks],
        }
    }

    /// No chunk is awake: none is in the middle of relaxing either.
    pub fn rest(&mut self) {
        self.relaxing.fill(false);
    }

    /// True if gas density is the same, to within `tol`, throughout chunk
    /// `c` and the chunks to its right and below (`right`, `down`).
    pub fn density_uniform(&self, c: usize, right: Option<usize>, down: Option<usize>, tol: f32) -> bool {
        let (mut lo, mut hi) = (self.rho_min[c], self.rho_max[c]);
        for n in [right, down].into_iter().flatten() {
            lo = lo.min(self.rho_min[n]);
            hi = hi.max(self.rho_max[n]);
        }
        hi - lo <= tol * hi
    }
}

/// Record the range of gas pressure and density in every active chunk.
/// Runs after the heat pass, so it sees this step's heating.
pub fn summarise(world: &mut World) {
    let World { chunks, derived: d, press: s, .. } = world;
    let (w, cw) = (chunks.w, chunks.cw);
    let d: &crate::derive::Derived = d;
    let active: &[bool] = &chunks.active;
    // One task per row of chunks; each writes only its own chunks' entries.
    s.p_min
        .par_chunks_mut(cw)
        .zip(s.p_max.par_chunks_mut(cw))
        .zip(s.rho_min.par_chunks_mut(cw))
        .zip(s.rho_max.par_chunks_mut(cw))
        .enumerate()
        .for_each(|(cy, (((p_min, p_max), rho_min), rho_max))| {
            for cx in 0..cw {
                if !active[cy * cw + cx] {
                    continue;
                }
                let (mut p_lo, mut p_hi) = (f32::MAX, f32::MIN);
                let (mut r_lo, mut r_hi) = (f32::MAX, f32::MIN);
                for ly in 0..CHUNK {
                    // Outside this range the row is all wall.
                    let [from, to] = d.row_span[(cy * cw + cx) * CHUNK + ly];
                    let row = (cy * CHUNK + ly) * w + cx * CHUNK;
                    for i in row + from as usize..row + to as usize {
                        // Cells with no room for gas have no gas pressure.
                        if d.gas_vol[i] > EPS_VOL && d.flags[i] & FIXED == 0 {
                            p_lo = p_lo.min(d.pressure[i]);
                            p_hi = p_hi.max(d.pressure[i]);
                            r_lo = r_lo.min(d.gas_rho[i]);
                            r_hi = r_hi.max(d.gas_rho[i]);
                        }
                    }
                }
                if p_lo > p_hi {
                    // No gas space at all in this chunk.
                    (p_lo, p_hi, r_lo, r_hi) = (0.0, 0.0, 0.0, 0.0);
                }
                p_min[cx] = p_lo;
                p_max[cx] = p_hi;
                rho_min[cx] = r_lo;
                rho_max[cx] = r_hi;
            }
        });
}

/// What one task needs to relax the chunks of one row of chunks. The
/// per-cell arrays cover that row and the one below it, because a pair can
/// end in the next chunk down; the per-chunk arrays likewise.
struct Relax<'a> {
    p: Off<'a, f32>,
    fx: Off<'a, f32>,
    fy: Off<'a, f32>,
    run_right: Off<'a, u8>,
    run_down: Off<'a, u8>,
    run_key: Off<'a, [u64; 3]>,
    run_max: Off<'a, [u8; 2]>,
    relaxing: Off<'a, bool>,
    d: &'a Derived,
    active: &'a [bool],
    /// Distance of this step's long pairs.
    long: usize,
    tol: f32,
    pore_gas_flow: f32,
    w: usize,
    h: usize,
    cw: usize,
    ch: usize,
}

pub fn solve(world: &mut World) {
    let long = LONG[world.step_count as usize % LONG.len()];
    let World { chunks, derived: d, press: s, params, .. } = world;
    let (w, h, cw, ch) = (chunks.w, chunks.h, chunks.cw, chunks.ch);
    let d: &Derived = d;
    let active: &[bool] = &chunks.active;
    let band = w * CHUNK;
    let right_of = |cx: usize, cy: usize| (cx + 1 < cw).then(|| cy * cw + cx + 1);
    let below = |cx: usize, cy: usize| (cy + 1 < ch).then(|| (cy + 1) * cw + cx);

    // Decide which chunks relax this step, and so where fluxes can appear:
    // in a relaxing chunk and in the chunks to its right and below, where
    // its pairs can end.
    s.has_flux.fill(false);
    for cy in 0..ch {
        for cx in 0..cw {
            let c = cy * cw + cx;
            if !active[c] {
                s.relaxing[c] = false;
                continue;
            }
            if !s.relaxing[c] {
                let (mut lo, mut hi) = (s.p_min[c], s.p_max[c]);
                for n in [right_of(cx, cy), below(cx, cy)].into_iter().flatten() {
                    // A chunk with no gas space reports 0..0; leave it out.
                    if s.p_max[n] > 0.0 || s.p_min[n] < 0.0 {
                        lo = lo.min(s.p_min[n]);
                        hi = hi.max(s.p_max[n]);
                    }
                }
                s.relaxing[c] = hi - lo > params.rel_tol * hi;
            }
            if s.relaxing[c] {
                s.has_flux[c] = true;
                for n in [right_of(cx, cy), below(cx, cy)].into_iter().flatten() {
                    s.has_flux[n] = true;
                }
            }
        }
    }
    let Scratch { p, fx, fy, run_right, run_down, run_key, run_max, zero_key, relaxing, has_flux, .. } = s;
    let has_flux: &[bool] = has_flux;

    // Start from this step's pressures and no flux. Only the part of each
    // row that is not wall is used, so only that part is prepared. That
    // relies on the flux of a wall cell being zero already (the running
    // sums read the cell to the left and the cell above without asking what
    // they are): nothing ever writes flux into a wall, and when the walls
    // in a chunk have moved, the whole chunk is cleared once.
    p.par_chunks_mut(band)
        .zip(fx.par_chunks_mut(band))
        .zip(fy.par_chunks_mut(band))
        .zip(zero_key.par_chunks_mut(cw))
        .enumerate()
        .for_each(|(cy, (((p, fx), fy), zero_key))| {
            for cx in 0..cw {
                let c = cy * cw + cx;
                if !has_flux[c] {
                    continue;
                }
                let whole = zero_key[cx] != d.flags_version[c];
                zero_key[cx] = d.flags_version[c];
                for ly in 0..CHUNK {
                    let [from, to] = if whole { [0, CHUNK as u8] } else { d.row_span[c * CHUNK + ly] };
                    let row = ly * w + cx * CHUNK;
                    let (a, b) = (row + from as usize, row + to as usize);
                    p[a..b].copy_from_slice(&d.pressure[cy * band + a..cy * band + b]);
                    fx[a..b].fill(0.0);
                    fy[a..b].fill(0.0);
                }
            }
        });

    // Relax, one task per row of chunks, in two rounds: even rows, then odd
    // rows. A chunk's pairs reach into the chunk to its right (same task)
    // and the chunk below (the next row), so two tasks of one round are
    // always a whole row apart and never touch the same cell. The outcome
    // does not depend on the number of threads.
    let tol = SETTLE * params.rel_tol;
    for round in 0..2usize.min(ch) {
        p[round * band..]
            .par_chunks_mut(2 * band)
            .zip(fx[round * band..].par_chunks_mut(2 * band))
            .zip(fy[round * band..].par_chunks_mut(2 * band))
            .zip(run_right[round * band..].par_chunks_mut(2 * band))
            .zip(run_down[round * band..].par_chunks_mut(2 * band))
            .zip(run_key[round * cw..].par_chunks_mut(2 * cw))
            .zip(run_max[round * cw..].par_chunks_mut(2 * cw))
            .zip(relaxing[round * cw..].par_chunks_mut(2 * cw))
            .enumerate()
            .for_each(|(k, (((((((p, fx), fy), run_right), run_down), run_key), run_max), relaxing))| {
                let cy = round + 2 * k;
                let (base, cbase) = (cy * band, cy * cw);
                let mut t = Relax {
                    p: Off { s: p, base },
                    fx: Off { s: fx, base },
                    fy: Off { s: fy, base },
                    run_right: Off { s: run_right, base },
                    run_down: Off { s: run_down, base },
                    run_key: Off { s: run_key, base: cbase },
                    run_max: Off { s: run_max, base: cbase },
                    relaxing: Off { s: relaxing, base: cbase },
                    d,
                    active,
                    long,
                    tol,
                    pore_gas_flow: params.pore_gas_flow,
                    w,
                    h,
                    cw,
                    ch,
                };
                for cx in 0..cw {
                    if t.relaxing[cbase + cx] {
                        relax_chunk(&mut t, cx, cy);
                    }
                }
            });
    }

    // Running sums along rows and down columns: the flux through a face is
    // everything that entered paths before it minus everything that left.
    // Each row of chunks does its own rows first, summing down from its own
    // top edge only.
    fx.par_chunks_mut(band).zip(fy.par_chunks_mut(band)).enumerate().for_each(|(cy, (fx, fy))| {
        for cx in 0..cw {
            let c = cy * cw + cx;
            if !has_flux[c] {
                continue;
            }
            let from_left = cx > 0 && has_flux[c - 1];
            for ly in 0..CHUNK {
                let row = ly * w + cx * CHUNK;
                // Left of `from` is wall, where the sum would restart
                // anyway, and right of `to` nothing is read.
                let [from, to] = d.row_span[c * CHUNK + ly];
                let mut sum = if from == 0 && from_left { fx[row - 1] } else { 0.0 };
                for i in row + from as usize..row + to as usize {
                    // Nothing flows through a wall, so restart there. This
                    // also stops rounding from building up along a row.
                    if d.flags[cy * band + i] & FIXED != 0 {
                        sum = 0.0;
                        fx[i] = 0.0;
                        fy[i] = 0.0;
                        continue;
                    }
                    sum += fx[i];
                    fx[i] = sum;
                    if ly > 0 {
                        fy[i] += fy[i - w];
                    }
                }
            }
        }
    });
    // Then carry what comes down through each top edge on into the chunk
    // below, as far as the first wall in each column. This has to go from
    // the top of the world down, but it only visits columns of open space
    // that cross a chunk boundary with a flux in them.
    for cy in 1..ch {
        for cx in 0..cw {
            let c = cy * cw + cx;
            if !has_flux[c] || !has_flux[c - cw] {
                continue;
            }
            let top = cy * band + cx * CHUNK;
            let mut carry = [0.0f32; CHUNK];
            carry.copy_from_slice(&fy[top - w..top - w + CHUNK]);
            for ly in 0..CHUNK {
                let [from, to] = d.row_span[c * CHUNK + ly];
                let (from, to) = (from as usize, to as usize);
                carry[..from].fill(0.0);
                carry[to..].fill(0.0);
                let mut left = 0u32;
                let row = top + ly * w;
                for k in from..to {
                    if d.flags[row + k] & FIXED != 0 {
                        carry[k] = 0.0;
                    } else {
                        fy[row + k] += carry[k];
                    }
                    left |= carry[k].to_bits() << 1;
                }
                if left == 0 {
                    break;
                }
            }
        }
    }
}

/// Relax the pairs of one chunk: this step's long pairs, then adjacent ones.
fn relax_chunk(t: &mut Relax, cx: usize, cy: usize) {
    let (w, h, cw, ch, d, tol) = (t.w, t.h, t.cw, t.ch, t.d, t.tol);
    let c = cy * cw + cx;
    let right = (cx + 1 < cw).then(|| c + 1);
    let down = (cy + 1 < ch).then(|| c + cw);
    let (x0, y0) = (cx * CHUNK, cy * CHUNK);
    let mut ops = 0usize;

    // Lengths of open runs, read from the flags. The scan starts a chunk's
    // width beyond the chunk so runs that continue into the next chunk are
    // counted; anything further out is assumed closed, which can only make
    // a run look shorter. The flags of this chunk and of the ones to its
    // right and below are all the runs depend on, so they are kept until
    // one of those changes.
    let key = [d.flags_version[c], right.map_or(0, |n| d.flags_version[n]), down.map_or(0, |n| d.flags_version[n])];
    if t.run_key[c] != key {
        t.run_key[c] = key;
        let (mut longest_right, mut longest_down) = (0u8, 0u8);
        let x_end = (x0 + 2 * CHUNK).min(w);
        for y in y0..y0 + CHUNK {
            let mut run = 0u8;
            for x in (x0..x_end).rev() {
                let i = y * w + x;
                run = if d.flags[i] & OPEN != 0 { run.saturating_add(1) } else { 0 };
                if x < x0 + CHUNK {
                    t.run_right[i] = run;
                    longest_right = longest_right.max(run);
                }
            }
        }
        let y_end = (y0 + 2 * CHUNK).min(h);
        let mut col = [0u8; CHUNK];
        for y in (y0..y_end).rev() {
            for (k, run) in col.iter_mut().enumerate() {
                let i = y * w + x0 + k;
                *run = if d.flags[i] & OPEN != 0 { run.saturating_add(1) } else { 0 };
                if y < y0 + CHUNK {
                    t.run_down[i] = *run;
                    longest_down = longest_down.max(*run);
                }
            }
        }
        t.run_max[c] = [longest_right, longest_down];
    }

    // Long pairs. `fx` and `fy` collect, for now, the amount entering a
    // path at its first face and leaving at its last; the running sums in
    // `solve` turn that into a flux through every face. A long pair may end
    // in the next chunk, and its whole path must be in chunks that are
    // simulated this step.
    let r = t.long;
    let long_cap = LONG_FACE_SHARE / r as f32;
    let right_active = right.is_some_and(|n| t.active[n]);
    let down_active = down.is_some_and(|n| t.active[n]);
    let cols = if right_active { CHUNK } else { CHUNK.saturating_sub(r) };
    let rows = if down_active { CHUNK } else { CHUNK.saturating_sub(r) };
    let spans = &d.row_span[c * CHUNK..(c + 1) * CHUNK];
    // No run in the chunk is long enough: nothing to look for.
    let [longest_right, longest_down] = t.run_max[c];
    let rows_h = if longest_right as usize > r { CHUNK } else { 0 };
    let rows_v = if longest_down as usize > r { rows } else { 0 };
    for y in y0..y0 + rows_h {
        let base = y * w + x0;
        let p = t.p.range(base, base + cols + r);
        let fx = t.fx.range(base, base + cols + r);
        let run = &t.run_right.range(base, base + cols)[..];
        let [from, to] = spans[y - y0];
        for k in from as usize..(to as usize).min(cols) {
            // A run longer than the distance means every cell from here to
            // the partner is open.
            if run[k] as usize > r {
                let (a, b) = (p[k], p[k + r]);
                let diff = a - b;
                if diff.abs() > tol * a.max(b) {
                    let cap = long_cap * a.min(b);
                    let delta = (0.5 * RELAX * diff).clamp(-cap, cap);
                    p[k] = a - delta;
                    p[k + r] = b + delta;
                    fx[k] += delta;
                    fx[k + r] -= delta;
                    ops += 1;
                }
            }
        }
    }
    for y in y0..y0 + rows_v {
        let (i0, j0) = (y * w + x0, (y + r) * w + x0);
        let (pa, pb) = t.p.two_runs(i0, j0, CHUNK);
        let (fa, fb) = t.fy.two_runs(i0, j0, CHUNK);
        let run = &t.run_down.range(i0, i0 + CHUNK)[..];
        let [from, to] = spans[y - y0];
        for k in from as usize..to as usize {
            if run[k] as usize > r {
                let (a, b) = (pa[k], pb[k]);
                let diff = a - b;
                if diff.abs() > tol * a.max(b) {
                    let cap = long_cap * a.min(b);
                    let delta = (0.5 * RELAX * diff).clamp(-cap, cap);
                    pa[k] = a - delta;
                    pb[k] = b + delta;
                    fa[k] += delta;
                    fb[k] -= delta;
                    ops += 1;
                }
            }
        }
    }

    // Adjacent pairs: every cell that is not a wall, open or not. The first
    // pass covers the chunk. Repeats only cover the box around the pairs
    // that moved a lot, since sharp differences are local (gas touching
    // something hot, a burst).
    let (mut bx0, mut by0, mut bx1, mut by1) = (x0, y0, x0 + CHUNK, y0 + CHUNK);
    for _ in 0..MAX_LOCAL_PASSES {
        let (mut nx0, mut ny0, mut nx1, mut ny1) = (usize::MAX, usize::MAX, 0, 0);
        for y in by0..by1 {
            let [from, to] = spans[y - y0];
            for x in bx0.max(x0 + from as usize)..bx1.min(x0 + to as usize) {
                let i = y * w + x;
                let flags = d.flags[i];
                if flags & FIXED != 0 {
                    continue;
                }
                for (j, in_world, vertical) in [(i + 1, x + 1 < w, false), (i + w, y + 1 < h, true)] {
                    if !in_world {
                        continue;
                    }
                    let delta = if flags & d.flags[j] & OPEN != 0 {
                        relax_open(&mut t.p, i, j, tol)
                    } else {
                        relax_any(&mut t.p, &d.gas_vol, &d.flags, i, j, tol, t.pore_gas_flow)
                    };
                    if delta != 0.0 {
                        let f = if vertical { &mut t.fy } else { &mut t.fx };
                        f[i] += delta;
                        f[j] -= delta;
                        ops += 1;
                        if delta.abs() >= REPEAT_ABOVE * t.p[i].max(t.p[j]) {
                            nx0 = nx0.min(x);
                            ny0 = ny0.min(y);
                            nx1 = nx1.max(x);
                            ny1 = ny1.max(y);
                        }
                    }
                }
            }
        }
        if nx0 > nx1 {
            break;
        }
        // Two cells of margin, kept inside this chunk.
        bx0 = nx0.saturating_sub(2).max(x0);
        by0 = ny0.saturating_sub(2).max(y0);
        bx1 = (nx1 + 3).min(x0 + CHUNK);
        by1 = (ny1 + 3).min(y0 + CHUNK);
    }

    // Nothing left to even out here: rest until the spread builds up again.
    if ops == 0 {
        t.relaxing[c] = false;
    }
}

/// Move two open cells towards equal pressure. Open cells are all gas, so
/// their volumes are taken as one cell each. Returns the `pV` that goes from
/// `i` to `j`, or zero if the two are already close enough.
#[inline(always)]
fn relax_open(p: &mut Off<f32>, i: usize, j: usize, tol: f32) -> f32 {
    let diff = p[i] - p[j];
    if diff.abs() <= tol * p[i].max(p[j]) {
        return 0.0;
    }
    let delta = 0.5 * RELAX * diff;
    p[i] -= delta;
    p[j] += delta;
    delta
}

/// The same for any two adjacent cells that are not walls: partly filled
/// cells, where the gas volumes differ, and powder, where flow is slowed.
#[inline(always)]
fn relax_any(p: &mut Off<f32>, vol: &[f32], flags: &[u8], i: usize, j: usize, tol: f32, pore_flow: f32) -> f32 {
    let (vi, vj) = (vol[i], vol[j]);
    if flags[j] & FIXED != 0 || vi <= EPS_VOL || vj <= EPS_VOL {
        return 0.0;
    }
    let diff = p[i] - p[j];
    if diff.abs() <= tol * p[i].max(p[j]) {
        return 0.0;
    }
    let mut delta = RELAX * diff * vi * vj / (vi + vj);
    if (flags[i] | flags[j]) & POWDER != 0 {
        delta *= pore_flow;
    }
    p[i] -= delta / vi;
    p[j] += delta / vj;
    delta
}
