//! Heat conduction between neighbouring cells.

use crate::world::{Dirty, World};
use crate::{CELL_CM, CHUNK, DT};

/// No pair may exchange more than this fraction of the heat that would bring
/// the two cells to the same temperature. With four neighbours this keeps
/// the explicit scheme stable for any conductivity.
const MAX_EQUALISE: f32 = 0.2;

pub fn run(world: &mut World) {
    let World { cells, chunks, derived: d, params, heat_gain: gain, .. } = world;
    let (w, h) = (chunks.w, chunks.h);
    // Fourier's law across one cell face: Q = k * (A / L) * dT * dt, where
    // A / L is (1 cm^2 / 1 cm), converted to metres.
    let geom = CELL_CM * 0.01 * DT * params.heat_speed;
    let tol = params.temp_tol;

    // Per chunk: some heat flowed from one of its cells this step.
    let mut flowed = vec![false; chunks.cw * chunks.ch];

    // First work out every flow from the start-of-step temperatures, so the
    // order faces are visited in does not matter. Then apply them.
    for cy in 0..chunks.ch {
        for cx in 0..chunks.cw {
            if !chunks.active[cy * chunks.cw + cx] {
                continue;
            }
            let mut dirty = Dirty::new();
            for y in cy * CHUNK..(cy + 1) * CHUNK {
                for x in cx * CHUNK..(cx + 1) * CHUNK {
                    let i = y * w + x;
                    let (ti, ki, ci) = (d.temp[i], d.cond[i], d.heat_cap[i]);
                    // Each cell handles the faces to its right and below, so
                    // every face is visited exactly once.
                    for (j, jx, jy) in [(i + 1, x + 1, y), (i + w, x, y + 1)] {
                        if jx >= w || jy >= h {
                            continue;
                        }
                        let dt = ti - d.temp[j];
                        if dt.abs() < tol {
                            continue;
                        }
                        let (kj, cj) = (d.cond[j], d.heat_cap[j]);
                        let k_sum = ki + kj;
                        let c_sum = ci + cj;
                        if k_sum <= 0.0 || c_sum <= 0.0 {
                            continue;
                        }
                        // Two half-cells in series: harmonic mean.
                        let k = 2.0 * ki * kj / k_sum;
                        let q_max = MAX_EQUALISE * dt * ci * cj / c_sum;
                        let mut q = k * geom * dt;
                        if q.abs() > q_max.abs() {
                            q = q_max;
                        }
                        if q == 0.0 {
                            continue;
                        }
                        gain[i] -= q;
                        gain[j] += q;
                        dirty.add_pair(x, y, jx, jy);
                    }
                }
            }
            flowed[cy * chunks.cw + cx] = !dirty.is_empty();
            dirty.flush(chunks);
        }
    }

    // Apply, and bring the snapshot up to date. The gas passes that follow
    // need this step's heating: gas that was just heated has to expand now,
    // and gas that moves on has to carry its new temperature with it.
    // A flow can cross into the chunk to the right or below, so those are
    // visited too.
    for cy in 0..chunks.ch {
        for cx in 0..chunks.cw {
            let c = cy * chunks.cw + cx;
            let here = flowed[c];
            let left = cx > 0 && flowed[c - 1];
            let up = cy > 0 && flowed[c - chunks.cw];
            if !(here || left || up) {
                continue;
            }
            for y in cy * CHUNK..(cy + 1) * CHUNK {
                let row = y * w + cx * CHUNK;
                for i in row..row + CHUNK {
                    let q = gain[i];
                    if q == 0.0 {
                        continue;
                    }
                    gain[i] = 0.0;
                    cells[i].energy += q;
                    let (t, c) = (d.temp[i], d.heat_cap[i]);
                    if c > 0.0 && t > 0.0 {
                        let t_new = t + q / c;
                        // Pressure is proportional to temperature while the
                        // gas stays put.
                        let scale = t_new / t;
                        d.temp[i] = t_new;
                        d.pressure[i] *= scale;
                        d.gas_pv[i] *= scale;
                    }
                }
            }
        }
    }
}
