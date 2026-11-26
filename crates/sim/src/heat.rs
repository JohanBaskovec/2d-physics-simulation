//! Heat conduction between neighbouring cells.

use crate::derive::Derived;
use crate::world::World;
use crate::{CELL_CM, CHUNK, DT};
use rayon::prelude::*;

/// No pair may exchange more than this fraction of the heat that would bring
/// the two cells to the same temperature. With four neighbours this keeps
/// the explicit scheme stable for any conductivity.
const MAX_EQUALISE: f32 = 0.2;

/// Heat that flows this step from cell `i` to its neighbour `j`, in joules
/// (negative: from `j` to `i`).
///
/// Each face is evaluated from both sides, once by each cell, so this must
/// give exactly opposite results when `i` and `j` are swapped. Every
/// product and sum below is written so that it does.
#[inline(always)]
fn flow(d: &Derived, i: usize, j: usize, geom: f32, tol: f32) -> f32 {
    let dt = d.temp[i] - d.temp[j];
    if dt.abs() < tol {
        return 0.0;
    }
    let (ki, kj) = (d.cond[i], d.cond[j]);
    let (ci, cj) = (d.heat_cap[i], d.heat_cap[j]);
    let (k_sum, c_sum) = (ki + kj, ci + cj);
    if k_sum <= 0.0 || c_sum <= 0.0 {
        return 0.0;
    }
    // Fourier's law, with the two half-cells in series (harmonic mean).
    let k = 2.0 * (ki * kj) / k_sum;
    let q = (k * geom) * dt;
    let q_max = (MAX_EQUALISE * dt) * ((ci * cj) / c_sum);
    if q.abs() > q_max.abs() {
        q_max
    } else {
        q
    }
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

    // First work out, for every cell, the heat it gains from its four
    // neighbours, all from the start-of-step temperatures. Each cell is
    // written by one task only, so rows of chunks run in parallel.
    //
    // A face is live when the chunk of its left or upper cell is active.
    // That decides which cells have anything to add up: all of an active
    // chunk, plus the first column or row of a sleeping chunk that has an
    // active chunk to its left or above.
    let d_ref: &Derived = d;
    let results: Vec<(Vec<bool>, Vec<[usize; 4]>)> = gain
        .par_chunks_mut(band)
        .enumerate()
        .map(|(cy, gain)| {
            let base = cy * band;
            let mut gained = vec![false; cw];
            let mut boxes = Vec::new();
            for cx in 0..cw {
                let c = cy * cw + cx;
                let here = active[c];
                let left = cx > 0 && active[c - 1];
                let up = cy > 0 && active[c - cw];
                if !(here || left || up) {
                    continue;
                }
                let (x0, y0) = (cx * CHUNK, cy * CHUNK);
                let (xs, ys) = if here {
                    (x0..x0 + CHUNK, y0..y0 + CHUNK)
                } else if left && !up {
                    (x0..x0 + 1, y0..y0 + CHUNK)
                } else if up && !left {
                    (x0..x0 + CHUNK, y0..y0 + 1)
                } else {
                    // Both: the first row and the first column. Walk the
                    // whole chunk and let the face tests below skip the rest.
                    (x0..x0 + CHUNK, y0..y0 + CHUNK)
                };
                // Bounding box of this chunk's cells that gave heat through
                // a face they own (right or below), to wake chunks with.
                let (mut bx0, mut by0, mut bx1, mut by1) = (usize::MAX, usize::MAX, 0, 0);
                for y in ys {
                    for x in xs.clone() {
                        let i = y * w + x;
                        let mut q = 0.0f32;
                        if here {
                            let mut own = false;
                            if x + 1 < w {
                                let f = flow(d_ref, i, i + 1, geom, tol);
                                own |= f != 0.0;
                                q -= f;
                            }
                            if y + 1 < h {
                                let f = flow(d_ref, i, i + w, geom, tol);
                                own |= f != 0.0;
                                q -= f;
                            }
                            if own {
                                bx0 = bx0.min(x);
                                by0 = by0.min(y);
                                bx1 = bx1.max((x + 1).min(w - 1));
                                by1 = by1.max((y + 1).min(h - 1));
                            }
                        }
                        // The faces to the left and above belong to the
                        // neighbouring cell, in this chunk or the next one
                        // over, and are live if that cell's chunk is active.
                        if x > 0 && (if x > x0 { here } else { left }) {
                            q -= flow(d_ref, i, i - 1, geom, tol);
                        }
                        if y > 0 && (if y > y0 { here } else { up }) {
                            q -= flow(d_ref, i, i - w, geom, tol);
                        }
                        if q != 0.0 {
                            gain[i - base] = q;
                            gained[cx] = true;
                        }
                    }
                }
                if bx0 <= bx1 {
                    boxes.push([bx0, by0, bx1, by1]);
                }
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
