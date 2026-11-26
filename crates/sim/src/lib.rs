//! 2D grid-based physics simulation.
//!
//! The world is a grid of fixed-size cells (1 cm x 1 cm x 1 cm). Each cell
//! stores only conserved quantities: moles of each substance it holds and its
//! total energy. Temperature, pressure, density and conductivity are derived
//! from those once per step.
//!
//! Step order (see [`World::step`]): derive -> heat -> gas (pressure solve, then
//! gas movement) -> grains -> liquids.

pub mod cell;
pub mod derive;
pub mod gas;
pub mod grains;
pub mod heat;
pub mod liquid;
pub mod pressure;
pub mod render;
pub mod scene;
pub mod substance;
pub mod world;
pub mod xfer;

pub use cell::{Cell, Form};
pub use substance::{SubId, Substances};
pub use world::{Params, World};

/// Number of (substance, form) slots per cell. Compile-time constant.
#[cfg(feature = "slots-4")]
pub const SLOTS: usize = 4;
#[cfg(all(feature = "slots-6", not(feature = "slots-4")))]
pub const SLOTS: usize = 6;
#[cfg(not(any(feature = "slots-4", feature = "slots-6")))]
pub const SLOTS: usize = 8;

/// Side of a cell in centimetres. Cells are cubes, so a cell is 1 cm^3.
pub const CELL_CM: f32 = 1.0;
/// Cell volume in cubic metres, for the ideal gas law.
pub const CELL_M3: f32 = 1.0e-6;
/// Simulation time step in seconds.
pub const DT: f32 = 1.0 / 60.0;
/// Gravity in cells per step squared (981 cm/s^2).
pub const GRAVITY: f32 = 981.0 * DT * DT / CELL_CM;
/// Molar gas constant, J/(mol K).
pub const R_GAS: f32 = 8.314_462;
/// Standard atmosphere in pascals.
pub const ATM: f32 = 101_325.0;
/// Fraction of a cell that grains fill when packed. The rest is pore space.
pub const PACKING: f32 = 0.6;
/// Chunk side in cells. Chunks are the unit of sleeping.
pub const CHUNK: usize = 64;
/// Amounts below this (moles) are swept along with any transfer so that
/// traces do not hold slots forever. Nothing is ever deleted.
pub const TRACE_MOL: f32 = 1.0e-9;
/// Volumes below this fraction of a cell count as "no space".
pub const EPS_VOL: f32 = 1.0e-6;
