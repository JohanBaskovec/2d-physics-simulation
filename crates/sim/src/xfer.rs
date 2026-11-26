//! Moving matter between two neighbouring cells.
//!
//! Every movement rule works on a [`Pair`]: private copies of the two cells
//! that are edited and then written back. Each transfer moves moles of one
//! slot and the enthalpy those moles carry, so matter and energy are
//! conserved by construction. A transfer into a cell with no slot for the
//! substance is refused and nothing changes ("block, never destroy").

use crate::cell::{flows, Cell, Form};
use crate::substance::Substances;
use crate::{EPS_VOL, SLOTS, TRACE_MOL};

/// Two cells being edited together.
///
/// Energies are tracked in f64 while the pair is open. A cell can lose
/// nearly all its matter in one operation (water falling out of a cell and
/// leaving only air), and in f32 the leftover energy would be the small
/// difference of two large numbers, which shows up as temperature noise.
#[derive(Clone, Copy)]
pub struct Pair<'s> {
    pub a: Cell,
    pub b: Cell,
    ea: f64,
    eb: f64,
    ta: f64,
    tb: f64,
    pub subs: &'s Substances,
}

impl<'s> Pair<'s> {
    pub fn new(a: Cell, b: Cell, subs: &'s Substances) -> Self {
        Pair {
            ea: a.energy as f64,
            eb: b.energy as f64,
            ta: a.temperature64(subs),
            tb: b.temperature64(subs),
            a,
            b,
            subs,
        }
    }

    /// Move `n` moles of slot `s` from `a` to `b`. Returns false, with
    /// nothing changed, if `b` has no slot for it.
    pub fn a_to_b(&mut self, s: usize, n: f32) -> bool {
        mv(&mut self.a, &mut self.b, &mut self.ea, &mut self.eb, self.ta, s, n, self.subs)
    }

    /// Move `n` moles of slot `s` from `b` to `a`.
    pub fn b_to_a(&mut self, s: usize, n: f32) -> bool {
        mv(&mut self.b, &mut self.a, &mut self.eb, &mut self.ea, self.tb, s, n, self.subs)
    }

    /// Close the pair and return the two edited cells.
    pub fn finish(mut self) -> (Cell, Cell) {
        // A cell with no matter cannot hold energy; hand any rounding
        // residue to the other cell so the total is unchanged.
        if self.a.is_empty() {
            self.eb += self.ea;
            self.ea = 0.0;
        }
        if self.b.is_empty() {
            self.ea += self.eb;
            self.eb = 0.0;
        }
        self.a.energy = self.ea as f32;
        self.b.energy = self.eb as f32;
        (self.a, self.b)
    }
}

#[allow(clippy::too_many_arguments)]
fn mv(
    src: &mut Cell,
    dst: &mut Cell,
    e_src: &mut f64,
    e_dst: &mut f64,
    t_src: f64,
    s: usize,
    n: f32,
    subs: &Substances,
) -> bool {
    let have = src.amount[s];
    if have <= 0.0 || n <= 0.0 {
        return true;
    }
    let (id, form) = (src.id[s], src.form[s]);
    let Some(d) = dst.slot_for(id, form) else { return false };
    // If what would be left behind is only a trace, take it all. This is the
    // rule that frees slots: the trace goes to a neighbour that now holds the
    // same substance, and nothing is deleted.
    let all = have - n < TRACE_MOL;
    let n = if all { have } else { n };
    let p = subs.props(id, form);
    let e = n as f64 * (p.h0 as f64 + p.cp as f64 * t_src);
    if all {
        src.clear_slot(s);
    } else {
        src.amount[s] = have - n;
    }
    dst.id[d] = id;
    dst.form[d] = form;
    dst.amount[d] += n;
    *e_src -= e;
    *e_dst += e;
    true
}

/// Gas amounts per slot (zero for slots that are not gas).
fn gas_amounts(c: &Cell) -> [f32; SLOTS] {
    let mut g = [0.0; SLOTS];
    for s in 0..SLOTS {
        if c.form[s] == Form::Gas {
            g[s] = c.amount[s];
        }
    }
    g
}

/// Let heavier fluid in the upper cell (`a`) trade places with lighter fluid
/// in the lower cell (`b`), volume for volume.
///
/// Liquid above falls into gas space below while the displaced gas comes up,
/// and a denser liquid above swaps with a lighter liquid below. `max_vol`
/// caps the volume moved down (used for slow seepage through powder).
/// Returns the volume moved down, as a fraction of a cell.
pub fn exchange_vertical(p: &mut Pair, max_vol: f32) -> f32 {
    let subs = p.subs;
    let mut budget = max_vol;
    let mut moved = 0.0f32;

    for _ in 0..2 * SLOTS {
        if budget <= EPS_VOL {
            break;
        }
        // Densest flowing slot in the upper cell.
        let mut top: Option<(usize, f32)> = None;
        for s in 0..SLOTS {
            if flows(p.a.form[s]) {
                let rho = subs.props(p.a.id[s], p.a.form[s]).density;
                if top.map_or(true, |(_, r)| rho > r) {
                    top = Some((s, rho));
                }
            }
        }
        let Some((i, rho_i)) = top else { break };
        let vm_i = subs.props(p.a.id[i], p.a.form[i]).vm;
        let vol_i = p.a.amount[i] * vm_i;
        let space = p.b.free_space(subs);
        let save = *p;

        if space > EPS_VOL {
            // Fall into gas space; the gas that occupied it comes up.
            let v = vol_i.min(space).min(budget);
            let n_i = if v >= vol_i { p.a.amount[i] } else { v / vm_i };
            let frac = if v >= space { 1.0 } else { v / space };
            let gas = gas_amounts(&p.b);
            let mut ok = p.a_to_b(i, n_i);
            for s in 0..SLOTS {
                ok = ok && p.b_to_a(s, gas[s] * frac);
            }
            if !ok {
                *p = save;
                break;
            }
            moved += v;
            budget -= v;
        } else {
            // Lower cell is full: swap with its lightest flowing slot, if
            // that is lighter than what we are bringing down.
            let mut bottom: Option<(usize, f32)> = None;
            for s in 0..SLOTS {
                if flows(p.b.form[s]) {
                    let rho = subs.props(p.b.id[s], p.b.form[s]).density;
                    if rho < rho_i && bottom.map_or(true, |(_, r)| rho < r) {
                        bottom = Some((s, rho));
                    }
                }
            }
            let Some((j, _)) = bottom else { break };
            let vm_j = subs.props(p.b.id[j], p.b.form[j]).vm;
            let vol_j = p.b.amount[j] * vm_j;
            let v = vol_i.min(vol_j).min(budget);
            let n_i = if v >= vol_i { p.a.amount[i] } else { v / vm_i };
            let n_j = if v >= vol_j { p.b.amount[j] } else { v / vm_j };
            if !(p.a_to_b(i, n_i) && p.b_to_a(j, n_j)) {
                *p = save;
                break;
            }
            moved += v;
            budget -= v;
        }
    }
    moved
}

/// Level liquids between two side-by-side cells.
///
/// Each liquid is levelled separately (so a dense liquid spreads underneath
/// a light one), and whatever net volume enters a cell pushes the same
/// volume of that cell's gas back the other way. Differences in fill level
/// below `tol` are left alone, which is what stops a puddle from spreading
/// into an endlessly thin film. Returns true if anything moved.
pub fn equalize_lateral(p: &mut Pair, max_vol: f32, tol: f32) -> bool {
    let subs = p.subs;
    // Grains take up room; liquid levels are compared within what is left.
    let cap_a = 1.0 - p.a.form_volume(subs, Form::Granular);
    let cap_b = 1.0 - p.b.form_volume(subs, Form::Granular);
    if cap_a <= EPS_VOL || cap_b <= EPS_VOL {
        return false;
    }
    let weight = cap_a * cap_b / (cap_a + cap_b);

    // One entry per flowing substance present on either side:
    // (slot in a, slot in b, volume to move a->b (negative: b->a), molar volume)
    let mut flow = [(usize::MAX, usize::MAX, 0.0f32, 0.0f32); 2 * SLOTS];
    let mut count = 0;
    let (mut pos, mut neg) = (0.0f32, 0.0f32);
    let mut consider = |sa: usize, sb: usize, va: f32, vb: f32, vm: f32| {
        let diff = va / cap_a - vb / cap_b;
        if diff.abs() > tol {
            let d = diff * weight;
            flow[count] = (sa, sb, d, vm);
            count += 1;
            if d > 0.0 {
                pos += d;
            } else {
                neg -= d;
            }
        }
    };
    for s in 0..SLOTS {
        if flows(p.a.form[s]) {
            let vm = subs.props(p.a.id[s], p.a.form[s]).vm;
            let sb = p.b.find(p.a.id[s], p.a.form[s]);
            let vb = sb.map_or(0.0, |sb| p.b.amount[sb] * vm);
            consider(s, sb.unwrap_or(usize::MAX), p.a.amount[s] * vm, vb, vm);
        }
    }
    for s in 0..SLOTS {
        if flows(p.b.form[s]) && p.a.find(p.b.id[s], p.b.form[s]).is_none() {
            let vm = subs.props(p.b.id[s], p.b.form[s]).vm;
            consider(usize::MAX, s, 0.0, p.b.amount[s] * vm, vm);
        }
    }
    if count == 0 {
        return false;
    }

    // Net inflow to either side cannot exceed the gas space it has.
    let space_a = p.a.free_space(subs);
    let space_b = p.b.free_space(subs);
    let (mut scale_pos, mut scale_neg) = (1.0f32, 1.0f32);
    if pos - neg > space_b {
        scale_pos = ((space_b + neg) / pos).clamp(0.0, 1.0);
    }
    if neg - pos > space_a {
        scale_neg = ((space_a + pos) / neg).clamp(0.0, 1.0);
    }
    let total = pos * scale_pos + neg * scale_neg;
    if total > max_vol {
        scale_pos *= max_vol / total;
        scale_neg *= max_vol / total;
    }
    let net = pos * scale_pos - neg * scale_neg;
    if pos * scale_pos + neg * scale_neg <= EPS_VOL {
        return false;
    }

    let save = *p;
    let gas_a = gas_amounts(&p.a);
    let gas_b = gas_amounts(&p.b);
    let mut ok = true;
    for &(sa, sb, d, vm) in &flow[..count] {
        if d > 0.0 {
            let v = d * scale_pos;
            let whole = v >= p.a.amount[sa] * vm;
            ok = ok && p.a_to_b(sa, if whole { p.a.amount[sa] } else { v / vm });
        } else {
            let v = -d * scale_neg;
            let whole = v >= p.b.amount[sb] * vm;
            ok = ok && p.b_to_a(sb, if whole { p.b.amount[sb] } else { v / vm });
        }
    }
    if net > EPS_VOL {
        let frac = (net / space_b).min(1.0);
        for s in 0..SLOTS {
            ok = ok && p.b_to_a(s, gas_b[s] * frac);
        }
    } else if net < -EPS_VOL {
        let frac = (-net / space_a).min(1.0);
        for s in 0..SLOTS {
            ok = ok && p.a_to_b(s, gas_a[s] * frac);
        }
    }
    if !ok {
        *p = save;
        return false;
    }
    true
}

/// Move a fraction of every gas in each cell to the other: `fa` of `a`'s gas
/// goes to `b` and `fb` of `b`'s gas goes to `a`. A gas that finds no slot on
/// the far side stays where it is; the others still move.
/// Returns true if anything moved.
pub fn exchange_gas(p: &mut Pair, fa: f32, fb: f32) -> bool {
    let gas_a = gas_amounts(&p.a);
    let gas_b = gas_amounts(&p.b);
    let mut any = false;
    for s in 0..SLOTS {
        if gas_a[s] > 0.0 && fa > 0.0 {
            any |= p.a_to_b(s, gas_a[s] * fa);
        }
    }
    for s in 0..SLOTS {
        if gas_b[s] > 0.0 && fb > 0.0 {
            any |= p.b_to_a(s, gas_b[s] * fb);
        }
    }
    any
}

/// Move `n` moles of slot `s` from `src` straight into `dst`, carrying
/// enthalpy at temperature `t_src`. The cheap path for small gas flows.
///
/// Unlike [`Pair`] this works in f32 and takes the source temperature from
/// the caller (the start-of-step snapshot). That is accurate as long as only
/// part of the cell's contents leaves; use a `Pair` when a cell may be
/// emptied. Returns false, with nothing changed, if `dst` has no slot.
#[inline]
pub fn move_fast(src: &mut Cell, dst: &mut Cell, s: usize, n: f32, t_src: f32, subs: &Substances) -> bool {
    let have = src.amount[s];
    let (id, form) = (src.id[s], src.form[s]);
    let Some(d) = dst.slot_for(id, form) else { return false };
    let all = have - n < TRACE_MOL;
    let n = if all { have } else { n };
    let p = subs.props(id, form);
    let e = n * (p.h0 + p.cp * t_src);
    if all {
        src.clear_slot(s);
    } else {
        src.amount[s] = have - n;
    }
    dst.id[d] = id;
    dst.form[d] = form;
    dst.amount[d] += n;
    src.energy -= e;
    dst.energy += e;
    true
}
