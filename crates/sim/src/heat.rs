//! Heat conduction between neighbouring cells.

use crate::derive::Derived;
use crate::world::World;
use crate::{CELL_CM, CHUNK, DT};
use rayon::prelude::*;

/// No pair may exchange more than this fraction of the heat that would bring
/// the two cells to the same temperature. With four neighbours this keeps
/// the explicit scheme stable for any conductivity.
const MAX_EQUALISE: f32 = 0.2;

/// Heat through each of `n` cell faces this step, in joules, from the cell
/// on side `a` to the one on side `b` (negative: the other way). `a` is the
/// left or upper cell of each face.
///
/// Written without branches so the compiler turns it into vector
/// instructions, eight faces at a time. A face where the sums are zero
/// computes garbage (0/0) on the way and is set to zero by the last line.
#[inline(always)]
fn flows(out: &mut [f32], d: &Derived, a: usize, b: usize, n: usize, geom: f32, tol: f32) {
    let out = &mut out[..n];
    let (ta, tb) = (&d.temp[a..a + n], &d.temp[b..b + n]);
    let (ka, kb) = (&d.cond[a..a + n], &d.cond[b..b + n]);
    let (ca, cb) = (&d.heat_cap[a..a + n], &d.heat_cap[b..b + n]);
    for x in 0..n {
        let dt = ta[x] - tb[x];
        let (k_sum, c_sum) = (ka[x] + kb[x], ca[x] + cb[x]);
        // Fourier's law, with the two half-cells in series (harmonic mean).
        let k = 2.0 * (ka[x] * kb[x]) / k_sum;
        let q = (k * geom) * dt;
        let q_max = (MAX_EQUALISE * dt) * ((ca[x] * cb[x]) / c_sum);
        let q = if q.abs() > q_max.abs() { q_max } else { q };
        let live = (dt.abs() >= tol) & (k_sum > 0.0) & (c_sum > 0.0);
        out[x] = if live { q } else { 0.0 };
    }
}

/// The bits of `v` without its sign: zero exactly when `v` is +0.0 or -0.0.
/// OR-ing these together over a run of numbers tells whether any is nonzero
/// without a branch per number.
#[inline(always)]
fn nonzero_bits(v: f32) -> u32 {
    v.to_bits() << 1
}

pub fn run(world: &mut World) {
    let World { cells, chunks, derived: d, params, heat_gain: gain, .. } = world;
    let (w, h, cw) = (chunks.w, chunks.h, chunks.cw);
    // Across one cell face: Q = k * (A / L) * dT * dt, where A / L is
    // (1 cm^2 / 1 cm), converted to metres.
    let geom = CELL_CM * 0.01 * DT * params.heat_speed;
    let tol = params.temp_tol;
    let band = w * CHUNK;
    let active: &[bool] = &chunks.active;

    // First work out, for every cell, the heat it gains through its four
    // faces, all from the start-of-step temperatures. Each cell is written
    // by one task only, so rows of chunks run in parallel.
    //
    // A face belongs to its left or upper cell and is live when that cell's
    // chunk is active. So the cells with anything to add up are all of an
    // active chunk, plus the first column or row of a sleeping chunk that
    // has an active chunk to its left or above.
    //
    // Each task goes down its rows keeping three lines of face flows: `fh`
    // (through the right face of each cell in this row; `fh[x + 1]` is the
    // face to the right of cell x, so `fh[x]` is the one to its left), `fv`
    // (through the bottom faces of this row) and `fv_up` (the row above).
    // Every face is worked out once, except the faces on a task's top edge,
    // which the task above also works out, from the same numbers in the
    // same order, so the two agree exactly and energy is conserved.
    let d_ref: &Derived = d;
    let results: Vec<(Vec<bool>, Vec<[usize; 4]>)> = gain
        .par_chunks_mut(band)
        .enumerate()
        .map(|(cy, gain)| {
            let mut gained = vec![false; cw];
            let mut boxes = Vec::new();
            let y0 = cy * CHUNK;
            let any_here = (0..cw).any(|cx| active[cy * cw + cx]);
            let any_up = cy > 0 && (0..cw).any(|cx| active[(cy - 1) * cw + cx]);
            if !any_here && !any_up {
                return (gained, boxes);
            }
            let mut fh = vec![0.0f32; w + 1];
            let mut fv = vec![0.0f32; w];
            let mut fv_up = vec![0.0f32; w];
            // Per column: nonzero if the cell in some row of this band gave
            // or took heat through a face it owns. Per chunk: the rows where
            // that happened. Together they give the box to wake chunks with.
            let mut col_any = vec![0u32; w];
            let mut rows = vec![(usize::MAX, 0usize); cw];
            if cy > 0 {
                for cx in 0..cw {
                    if active[(cy - 1) * cw + cx] {
                        let x0 = cx * CHUNK;
                        let above = (y0 - 1) * w + x0;
                        flows(&mut fv_up[x0..x0 + CHUNK], d_ref, above, above + w, CHUNK, geom, tol);
                    }
                }
            }
            for y in y0..y0 + CHUNK {
                let row = y * w;
                for cx in 0..cw {
                    let x0 = cx * CHUNK;
                    if !active[cy * cw + cx] {
                        fh[x0 + 1..x0 + CHUNK + 1].fill(0.0);
                        fv[x0..x0 + CHUNK].fill(0.0);
                        continue;
                    }
                    // The last cell of the world's last column has no right
                    // face, and the last row has no faces below.
                    let n = if x0 + CHUNK < w { CHUNK } else { CHUNK - 1 };
                    fh[x0 + CHUNK] = 0.0;
                    flows(&mut fh[x0 + 1..x0 + 1 + n], d_ref, row + x0, row + x0 + 1, n, geom, tol);
                    if y + 1 < h {
                        flows(&mut fv[x0..x0 + CHUNK], d_ref, row + x0, row + x0 + w, CHUNK, geom, tol);
                    } else {
                        fv[x0..x0 + CHUNK].fill(0.0);
                    }
                    let mut any = 0u32;
                    let (right, down) = (&fh[x0 + 1..x0 + 1 + CHUNK], &fv[x0..x0 + CHUNK]);
                    for (k, col) in col_any[x0..x0 + CHUNK].iter_mut().enumerate() {
                        let bits = nonzero_bits(right[k]) | nonzero_bits(down[k]);
                        *col |= bits;
                        any |= bits;
                    }
                    if any != 0 {
                        rows[cx].0 = rows[cx].0.min(y);
                        rows[cx].1 = y;
                    }
                }
                for cx in 0..cw {
                    let c = cy * cw + cx;
                    let here = active[c];
                    let left = cx > 0 && active[c - 1];
                    let up = cy > 0 && active[c - cw];
                    let x0 = cx * CHUNK;
                    // In a sleeping chunk only the first row (below an
                    // active chunk) and the first column (right of one)
                    // have a live face.
                    let n = if here || (up && y == y0) {
                        CHUNK
                    } else if left {
                        1
                    } else {
                        continue;
                    };
                    let out = &mut gain[row - y0 * w + x0..][..n];
                    let (from_left, to_right) = (&fh[x0..x0 + n], &fh[x0 + 1..x0 + 1 + n]);
                    let (from_up, to_down) = (&fv_up[x0..x0 + n], &fv[x0..x0 + n]);
                    let mut any = 0u32;
                    for k in 0..n {
                        let q = (from_left[k] - to_right[k]) + (from_up[k] - to_down[k]);
                        out[k] = q;
                        any |= nonzero_bits(q);
                    }
                    gained[cx] |= any != 0;
                }
                std::mem::swap(&mut fv, &mut fv_up);
            }
            for cx in 0..cw {
                let (by0, by1) = rows[cx];
                if by0 > by1 {
                    continue;
                }
                let x0 = cx * CHUNK;
                let cols = &col_any[x0..x0 + CHUNK];
                let first = cols.iter().position(|&b| b != 0).expect("a row had flow");
                let last = cols.iter().rposition(|&b| b != 0).expect("a row had flow");
                // One cell further right and down: the cells on the far
                // side of those faces.
                boxes.push([x0 + first, by0, (x0 + last + 1).min(w - 1), (by1 + 1).min(h - 1)]);
            }
            (gained, boxes)
        })
        .collect();

    // Wake chunks around every cell that exchanged heat.
    for (_, boxes) in &results {
        for &[x0, y0, x1, y1] in boxes {
            chunks.touch_chunk_of(x0, y0);
            chunks.touch_chunk_of(x1, y0);
            chunks.touch_chunk_of(x0, y1);
            chunks.touch_chunk_of(x1, y1);
        }
    }

    // Apply, and bring the snapshot up to date. The gas passes that follow
    // need this step's heating: gas that was just heated has to expand now,
    // and gas that moves on has to carry its new temperature with it.
    let gained: Vec<&Vec<bool>> = results.iter().map(|(g, _)| g).collect();
    cells
        .par_chunks_mut(band)
        .zip(gain.par_chunks_mut(band))
        .zip(d.temp.par_chunks_mut(band))
        .zip(d.pressure.par_chunks_mut(band))
        .zip(d.gas_pv.par_chunks_mut(band))
        .zip(d.heat_cap.par_chunks(band))
        .enumerate()
        .for_each(|(cy, (((((cells, gain), temp), pressure), gas_pv), heat_cap))| {
            for cx in 0..cw {
                if !gained[cy][cx] {
                    continue;
                }
                for ly in 0..CHUNK {
                    let row = ly * w + cx * CHUNK;
                    for i in row..row + CHUNK {
                        let q = gain[i];
                        if q == 0.0 {
                            continue;
                        }
                        gain[i] = 0.0;
                        cells[i].energy += q;
                        let (t, c) = (temp[i], heat_cap[i]);
                        if c > 0.0 && t > 0.0 {
                            let t_new = t + q / c;
                            // Pressure is proportional to temperature while
                            // the gas stays put.
                            let scale = t_new / t;
                            temp[i] = t_new;
                            pressure[i] *= scale;
                            gas_pv[i] *= scale;
                        }
                    }
                }
            }
        });
}
