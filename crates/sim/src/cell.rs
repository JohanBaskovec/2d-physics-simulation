//! The cell: the only place simulation state lives.

use crate::substance::{SubId, Substances};
use crate::{ATM, CELL_M3, PACKING, R_GAS, SLOTS};

/// The form of the matter in one slot. Phase lives here, per slot, so a
/// boiling cell holds liquid water and steam side by side.
///
/// Rules for the phase-change pass (not written yet; it comes with the full
/// element table):
/// - A change of form moves moles from one slot to another at constant cell
///   energy. The latent heat is already in the enthalpy offsets
///   (`FormProps::h0`), so the temperature plateau needs no extra state.
/// - If the cell has no free slot for the new form, the change is blocked.
///   That is safe because energy, not temperature, is what is stored: the
///   temperature simply overshoots the transition point (superheating or
///   supercooling) and the change catches up once a slot frees. Nothing is
///   lost or created.
/// - When a slot converts completely within one step, its form byte is
///   changed in place, so a complete conversion never needs a free slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Form {
    /// Empty slot.
    None = 0,
    /// Solid that holds its shape (a wall, or ice forming in water).
    Rigid = 1,
    /// Solid in grains. Packs to [`PACKING`] of the cell at most.
    Granular = 2,
    Liquid = 3,
    Gas = 4,
}

/// How a whole cell behaves, decided by what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Rigid volume above 50%: stays in place and is sealed to flow.
    Fixed,
    /// Holds grains: moves as a whole cell, fluids seep through its pores.
    Powder,
    /// Everything else, including vacuum. Liquids and gases flow per slot,
    /// and any rigid part below 50% is carried along with the liquid.
    Fluid,
}

/// One grid cell. With 8 slots this is exactly 64 bytes, one cache line.
///
/// Invariant: an empty slot has `id == 0`, `form == Form::None` and
/// `amount == 0.0`, so sums over all slots need no "is it empty" branch.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(any(feature = "slots-4", feature = "slots-6")), repr(C, align(64)))]
#[cfg_attr(any(feature = "slots-4", feature = "slots-6"), repr(C))]
pub struct Cell {
    /// Moles of each slot's substance.
    pub amount: [f32; SLOTS],
    /// Total energy of the cell's contents in joules (see `FormProps::h0`).
    pub energy: f32,
    pub id: [SubId; SLOTS],
    /// Velocity of the cell's bulk in 1/256 cell per step: `[x, y]`, y down.
    pub vel: [i16; 2],
    pub form: [Form; SLOTS],
}

#[cfg(not(any(feature = "slots-4", feature = "slots-6")))]
const _: () = assert!(std::mem::size_of::<Cell>() == 64);

impl Cell {
    /// A cell with nothing in it: vacuum.
    pub const EMPTY: Cell = Cell {
        amount: [0.0; SLOTS],
        energy: 0.0,
        id: [0; SLOTS],
        vel: [0; 2],
        form: [Form::None; SLOTS],
    };

    // ---- constructors -------------------------------------------------

    /// Add `moles` of a substance at temperature `t`. Returns false, leaving
    /// the cell unchanged, if no slot is available.
    pub fn add_at(&mut self, subs: &Substances, id: SubId, form: Form, moles: f32, t: f32) -> bool {
        let Some(s) = self.slot_for(id, form) else { return false };
        let p = subs.props(id, form);
        self.id[s] = id;
        self.form[s] = form;
        self.amount[s] += moles;
        self.energy += moles * (p.h0 + p.cp * t);
        true
    }

    /// A cell completely filled with one solid or liquid.
    pub fn filled(subs: &Substances, id: SubId, form: Form, t: f32) -> Cell {
        debug_assert!(matches!(form, Form::Rigid | Form::Liquid));
        let mut c = Cell::EMPTY;
        c.add_at(subs, id, form, 1.0 / subs.props(id, form).vm, t);
        c
    }

    /// A gas mixture filling the cell at pressure `p` (Pa). `mix` is a list
    /// of (substance, mole fraction).
    pub fn gas(subs: &Substances, mix: &[(SubId, f32)], t: f32, p: f32) -> Cell {
        let mut c = Cell::EMPTY;
        c.add_gas(subs, mix, t, p, 1.0);
        c
    }

    /// A packed powder cell whose pores hold the given gas mixture.
    pub fn powder(subs: &Substances, id: SubId, t: f32, pore_gas: &[(SubId, f32)], p: f32) -> Cell {
        let mut c = Cell::EMPTY;
        c.add_at(subs, id, Form::Granular, PACKING / subs.props(id, Form::Granular).vm, t);
        c.add_gas(subs, pore_gas, t, p, 1.0 - PACKING);
        c
    }

    fn add_gas(&mut self, subs: &Substances, mix: &[(SubId, f32)], t: f32, p: f32, volume: f32) {
        let total = p * volume * CELL_M3 / (R_GAS * t);
        for &(id, frac) in mix {
            self.add_at(subs, id, Form::Gas, total * frac, t);
        }
    }

    // ---- slots --------------------------------------------------------

    pub fn find(&self, id: SubId, form: Form) -> Option<usize> {
        (0..SLOTS).find(|&s| self.id[s] == id && self.form[s] == form)
    }

    /// The slot already holding this substance and form, or a free one.
    /// `None` means the cell is full: the substance is blocked from entering.
    pub fn slot_for(&self, id: SubId, form: Form) -> Option<usize> {
        self.find(id, form)
            .or_else(|| (0..SLOTS).find(|&s| self.form[s] == Form::None))
    }

    pub fn clear_slot(&mut self, s: usize) {
        self.id[s] = 0;
        self.form[s] = Form::None;
        self.amount[s] = 0.0;
    }

    #[inline]
    pub fn has_form(&self, f: Form) -> bool {
        self.form.contains(&f)
    }

    /// True if no slot is in use.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.form.iter().all(|&x| x == Form::None)
    }

    // ---- velocity -----------------------------------------------------

    /// Downward speed in cells per step.
    #[inline]
    pub fn vy(&self) -> f32 {
        self.vel[1] as f32 / 256.0
    }

    #[inline]
    pub fn set_vy(&mut self, v: f32) {
        self.vel[1] = (v * 256.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    }

    // ---- derived quantities ------------------------------------------

    /// Volume fraction of one slot. Zero for gas.
    #[inline]
    pub fn slot_volume(&self, subs: &Substances, s: usize) -> f32 {
        self.amount[s] * subs.props(self.id[s], self.form[s]).vm
    }

    /// Volume fraction taken by solids and liquids.
    pub fn condensed_volume(&self, subs: &Substances) -> f32 {
        (0..SLOTS).map(|s| self.slot_volume(subs, s)).sum()
    }

    /// Volume fraction left for gas (or vacuum).
    pub fn free_space(&self, subs: &Substances) -> f32 {
        (1.0 - self.condensed_volume(subs)).max(0.0)
    }

    pub fn form_volume(&self, subs: &Substances, f: Form) -> f32 {
        (0..SLOTS)
            .filter(|&s| self.form[s] == f)
            .map(|s| self.slot_volume(subs, s))
            .sum()
    }

    /// Total mass in grams. Since a cell is 1 cm^3 this is also its bulk
    /// density in g/cm^3.
    pub fn mass(&self, subs: &Substances) -> f32 {
        (0..SLOTS)
            .map(|s| self.amount[s] * subs.props(self.id[s], self.form[s]).molar_mass)
            .sum()
    }

    /// True density of the grains themselves, g/cm^3 (not the packed bulk).
    pub fn grain_density(&self, subs: &Substances) -> f32 {
        let (mut m, mut v) = (0.0, 0.0);
        for s in 0..SLOTS {
            if self.form[s] == Form::Granular {
                let p = subs.props(self.id[s], Form::Granular);
                m += self.amount[s] * p.molar_mass;
                v += self.amount[s] * p.vm;
            }
        }
        if v > 0.0 {
            m / v
        } else {
            0.0
        }
    }

    pub fn heat_capacity(&self, subs: &Substances) -> f32 {
        (0..SLOTS)
            .map(|s| self.amount[s] * subs.props(self.id[s], self.form[s]).cp)
            .sum()
    }

    /// Temperature in kelvin, computed in f64. Zero for vacuum.
    pub fn temperature64(&self, subs: &Substances) -> f64 {
        let (mut ncp, mut nh0) = (0.0f64, 0.0f64);
        for s in 0..SLOTS {
            let p = subs.props(self.id[s], self.form[s]);
            ncp += self.amount[s] as f64 * p.cp as f64;
            nh0 += self.amount[s] as f64 * p.h0 as f64;
        }
        if ncp > 0.0 {
            (self.energy as f64 - nh0) / ncp
        } else {
            0.0
        }
    }

    pub fn temperature(&self, subs: &Substances) -> f32 {
        self.temperature64(subs) as f32
    }

    /// Gas pressure in pascals.
    pub fn pressure(&self, subs: &Substances) -> f32 {
        let n: f32 = (0..SLOTS)
            .filter(|&s| self.form[s] == Form::Gas)
            .map(|s| self.amount[s])
            .sum();
        let v = self.free_space(subs).max(crate::derive::MIN_GAS_VOL);
        n * R_GAS * self.temperature(subs) / (v * CELL_M3)
    }

    #[inline]
    pub fn is_powder(&self) -> bool {
        self.has_form(Form::Granular)
    }

    /// Fixed in place: more than half the cell is rigid solid.
    #[inline]
    pub fn is_fixed(&self, subs: &Substances) -> bool {
        self.has_form(Form::Rigid)
            && !self.is_powder()
            && self.form_volume(subs, Form::Rigid) > 0.5
    }

    pub fn kind(&self, subs: &Substances) -> Kind {
        if self.is_powder() {
            Kind::Powder
        } else if self.is_fixed(subs) {
            Kind::Fixed
        } else {
            Kind::Fluid
        }
    }

    /// True if the cell holds anything that moves under the liquid rules:
    /// liquid, or loose rigid pieces carried by it.
    #[inline]
    pub fn has_flowing(&self) -> bool {
        self.form.iter().any(|&f| flows(f))
    }
}

/// Forms that move under the liquid rules. Rigid counts because these rules
/// only ever run on cells that are not `Fixed`, where rigid pieces are loose.
#[inline(always)]
pub fn flows(f: Form) -> bool {
    matches!(f, Form::Liquid | Form::Rigid)
}

/// Dry air, simplified to its two main components.
pub fn air(subs: &Substances) -> [(SubId, f32); 2] {
    [(subs.must("N2"), 0.79), (subs.must("O2"), 0.21)]
}

/// Room temperature, K.
pub const ROOM_T: f32 = 293.15;
/// Ambient pressure, Pa.
pub const AMBIENT_P: f32 = ATM;
