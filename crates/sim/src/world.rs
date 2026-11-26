//! The world: the cell grid, chunk sleeping, and the step loop.

use crate::cell::{air, Cell, AMBIENT_P, ROOM_T};
use crate::derive::Derived;
use crate::substance::Substances;
use crate::CHUNK;
use std::time::{Duration, Instant};

/// Tunable rates. Everything here is a modelling choice, not table data.
#[derive(Clone, Debug)]
pub struct Params {
    /// Multiplier on heat conduction. 1.0 is real time.
    pub heat_speed: f32,
    /// Largest liquid volume (fraction of a cell) that can seep into or out
    /// of a powder cell per step. 0.1 cm/s is typical for coarse sand.
    pub seep_vol: f32,
    /// Gas flows this much slower when either cell is powder.
    pub pore_gas_flow: f32,
    /// Largest share of a cell's gas that may leave through one face in a
    /// step. Limits how fast gas can rush into a vacuum or out of a blast.
    pub gas_max_flow: f32,
    /// Pressure-driven flux through a face below this share of the giving
    /// cell's gas is treated as rounding error and not moved. Keep it tiny:
    /// the flux along a path has to be applied at every face or at none, or
    /// gas piles up where the path is cut and the pressure pass never rests.
    /// What decides whether gas flows at all is `rel_tol`.
    pub gas_flow_tol: f32,
    /// Gas interdiffusion per step, in cells^2 (0.2 cm^2/s is typical).
    pub gas_diffusion: f32,
    /// Relative gas density differences below this do not mix or overturn.
    pub mix_tol: f32,
    /// How many cells a heavy gas parcel can sink through lighter gas in one
    /// move.
    pub gas_sink_reach: usize,
    /// Liquid fill-level differences below this do not flow sideways.
    pub level_tol: f32,
    /// How many cells sideways a liquid can level with in one step.
    pub liquid_reach: usize,
    /// Temperature differences below this (K) do not conduct.
    pub temp_tol: f32,
    /// Relative gas pressure differences below this are left alone.
    pub rel_tol: f32,
    /// Speed limit in cells per step.
    pub max_speed: f32,
    /// Grain diameter in cm, for drag on falling grains.
    pub grain_cm: f32,
    /// Debugging aid: after each derive pass, recompute every active cell
    /// from scratch and panic if the kept values disagree. Slow; for tests.
    pub verify_derived: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            heat_speed: 1.0,
            seep_vol: 0.1 * crate::DT / crate::CELL_CM,
            pore_gas_flow: 0.25,
            gas_max_flow: 0.5,
            gas_flow_tol: 1.0e-6,
            gas_diffusion: 0.2 * crate::DT / (crate::CELL_CM * crate::CELL_CM),
            gas_sink_reach: 32,
            level_tol: 0.005,
            liquid_reach: 8,
            temp_tol: 0.01,
            rel_tol: 2.0e-3,
            mix_tol: 1.0e-3,
            max_speed: 16.0,
            grain_cm: 0.1,
            verify_derived: false,
        }
    }
}

/// Which chunks are simulated. A chunk sleeps until something in it, or
/// right next to it, changes.
pub struct Chunks {
    /// Grid size in cells.
    pub w: usize,
    pub h: usize,
    /// Grid size in chunks.
    pub cw: usize,
    pub ch: usize,
    /// Chunks simulated this step.
    pub active: Vec<bool>,
    /// Chunks to simulate next step.
    next: Vec<bool>,
    /// Per cell: its contents changed since its derived values were last
    /// worked out. Set by `touch`; cleared by the derive pass.
    pub stale: Vec<u8>,
}

impl Chunks {
    fn new(w: usize, h: usize) -> Self {
        let (cw, ch) = (w / CHUNK, h / CHUNK);
        Chunks { w, h, cw, ch, active: vec![false; cw * ch], next: vec![true; cw * ch], stale: vec![1; w * h] }
    }

    /// Record that the cell at (x, y) changed: mark it for the derive pass
    /// and wake its chunk and any chunk it borders. Every change to what a
    /// cell holds must end up here (or set `stale` itself), or the cell's
    /// derived values go on describing what it used to hold.
    #[inline]
    pub fn touch(&mut self, x: usize, y: usize) {
        self.stale[y * self.w + x] = 1;
        let (cx, cy) = (x / CHUNK, y / CHUNK);
        let (lx, ly) = (x % CHUNK, y % CHUNK);
        let x0 = if lx == 0 && cx > 0 { cx - 1 } else { cx };
        let x1 = if lx == CHUNK - 1 && cx + 1 < self.cw { cx + 1 } else { cx };
        let y0 = if ly == 0 && cy > 0 { cy - 1 } else { cy };
        let y1 = if ly == CHUNK - 1 && cy + 1 < self.ch { cy + 1 } else { cy };
        for j in y0..=y1 {
            for i in x0..=x1 {
                self.next[j * self.cw + i] = true;
            }
        }
    }

    /// Wake the chunk holding (x, y) and any chunk that cell borders,
    /// without marking the cell. For changes to energy only, which the heat
    /// pass folds into the derived values itself.
    pub fn touch_chunk_of(&mut self, x: usize, y: usize) {
        let was = self.stale[y * self.w + x];
        self.touch(x, y);
        self.stale[y * self.w + x] = was;
    }

    #[inline]
    pub fn touch_index(&mut self, i: usize) {
        self.touch(i % self.w, i / self.w);
    }

    pub fn wake_all(&mut self) {
        self.next.fill(true);
    }

    fn advance(&mut self) {
        std::mem::swap(&mut self.active, &mut self.next);
        self.next.fill(false);
    }

    pub fn active_count(&self) -> usize {
        self.active.iter().filter(|&&a| a).count()
    }

    /// True if any chunk is due to be simulated next step.
    pub fn any_pending(&self) -> bool {
        self.next.iter().any(|&a| a)
    }
}

/// Bounding box of the cells a pass changed while working through one chunk.
/// Cheaper than calling [`Chunks::touch`] for every change in a hot loop.
pub struct Dirty {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Dirty {
    pub fn new() -> Self {
        Dirty { x0: usize::MAX, y0: usize::MAX, x1: 0, y1: 0 }
    }

    /// Record a change to the cells (x, y) and (jx, jy), where the second is
    /// the right or lower neighbour of the first.
    #[inline(always)]
    pub fn add_pair(&mut self, x: usize, y: usize, jx: usize, jy: usize) {
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(jx);
        self.y1 = self.y1.max(jy);
    }

    /// True if nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.x0 > self.x1
    }

    /// The box as `[x0, y0, x1, y1]`, if anything was recorded, and reset.
    /// For tasks that cannot reach the chunk list and hand their boxes back.
    pub fn take(&mut self) -> Option<[usize; 4]> {
        let out = (!self.is_empty()).then_some([self.x0, self.y0, self.x1, self.y1]);
        *self = Dirty::new();
        out
    }

    /// Wake every chunk the box overlaps or borders, then reset. The box
    /// must be no larger than a chunk plus one cell, so its corners reach
    /// every chunk it covers.
    pub fn flush(&mut self, chunks: &mut Chunks) {
        if self.x0 <= self.x1 {
            chunks.touch(self.x0, self.y0);
            chunks.touch(self.x1, self.y0);
            chunks.touch(self.x0, self.y1);
            chunks.touch(self.x1, self.y1);
        }
        *self = Dirty::new();
    }
}

impl Default for Dirty {
    fn default() -> Self {
        Dirty::new()
    }
}

/// Part of a per-cell array, indexed with the same cell indices as the
/// whole array. Lets a task that was handed a few rows of the world keep
/// using world coordinates.
pub struct Off<'a, T> {
    pub s: &'a mut [T],
    /// Index in the whole array of this part's first element.
    pub base: usize,
}

impl<T> Off<'_, T> {
    /// Two different elements at once, `i` before `j`.
    #[inline(always)]
    pub fn two(&mut self, i: usize, j: usize) -> (&mut T, &mut T) {
        let (head, tail) = self.s.split_at_mut(j - self.base);
        (&mut head[i - self.base], &mut tail[0])
    }

    /// The elements from `a` up to `b`.
    #[inline(always)]
    pub fn range(&mut self, a: usize, b: usize) -> &mut [T] {
        &mut self.s[a - self.base..b - self.base]
    }

    /// Two runs of `n` elements at once, the one at `i` wholly before the
    /// one at `j`.
    #[inline(always)]
    pub fn two_runs(&mut self, i: usize, j: usize, n: usize) -> (&mut [T], &mut [T]) {
        let (head, tail) = self.s.split_at_mut(j - self.base);
        (&mut head[i - self.base..i - self.base + n], &mut tail[..n])
    }
}

impl<T> std::ops::Index<usize> for Off<'_, T> {
    type Output = T;
    #[inline(always)]
    fn index(&self, i: usize) -> &T {
        &self.s[i - self.base]
    }
}

impl<T> std::ops::IndexMut<usize> for Off<'_, T> {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut T {
        &mut self.s[i - self.base]
    }
}

/// Time spent in each pass of the last step.
#[derive(Clone, Copy, Debug, Default)]
pub struct StepStats {
    pub active_chunks: usize,
    pub derive: Duration,
    pub heat: Duration,
    pub gas: Duration,
    pub grains: Duration,
    pub liquid: Duration,
}

impl StepStats {
    pub fn total(&self) -> Duration {
        self.derive + self.heat + self.gas + self.grains + self.liquid
    }
}

pub struct World {
    pub cells: Vec<Cell>,
    pub chunks: Chunks,
    pub derived: Derived,
    pub(crate) press: crate::pressure::Scratch,
    pub subs: Substances,
    pub params: Params,
    pub stats: StepStats,
    /// Heat arriving in each cell during the heat pass, before it is
    /// applied. Zero outside that pass.
    pub(crate) heat_gain: Vec<f32>,
    /// Marks cells already moved this step (low byte of the step counter).
    pub(crate) stamp: Vec<u8>,
    pub(crate) step_count: u64,
    pub(crate) rng: Rng,
}

impl World {
    /// A closed box of the given size, filled with room-temperature air.
    /// Width and height must be multiples of [`CHUNK`].
    pub fn new(w: usize, h: usize, subs: Substances) -> World {
        assert!(w > 0 && h > 0 && w % CHUNK == 0 && h % CHUNK == 0, "size must be a multiple of {CHUNK}");
        let ambient = Cell::gas(&subs, &air(&subs), ROOM_T, AMBIENT_P);
        World {
            cells: vec![ambient; w * h],
            chunks: Chunks::new(w, h),
            derived: Derived::new(w * h, (w / CHUNK) * (h / CHUNK)),
            press: crate::pressure::Scratch::new(w * h, (w / CHUNK) * (h / CHUNK)),
            subs,
            params: Params::default(),
            stats: StepStats::default(),
            heat_gain: vec![0.0; w * h],
            stamp: vec![0; w * h],
            step_count: 0,
            rng: Rng(0x9E37_79B9_7F4A_7C15),
        }
    }

    pub fn width(&self) -> usize {
        self.chunks.w
    }

    pub fn height(&self) -> usize {
        self.chunks.h
    }

    pub fn steps(&self) -> u64 {
        self.step_count
    }

    #[inline]
    pub fn index(&self, x: usize, y: usize) -> usize {
        y * self.chunks.w + x
    }

    pub fn cell(&self, x: usize, y: usize) -> &Cell {
        &self.cells[self.index(x, y)]
    }

    /// Replace a cell. This is an editing action, not physics: it creates or
    /// removes matter.
    pub fn set(&mut self, x: usize, y: usize, cell: Cell) {
        let i = self.index(x, y);
        self.cells[i] = cell;
        self.chunks.touch(x, y);
    }

    /// Replace every cell in a rectangle (clipped to the world).
    pub fn fill_rect(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, cell: Cell) {
        for y in y0..y1.min(self.height()) {
            for x in x0..x1.min(self.width()) {
                self.set(x, y, cell);
            }
        }
    }

    /// Replace every cell within `r` of (cx, cy) (clipped to the world).
    pub fn fill_circle(&mut self, cx: i32, cy: i32, r: i32, cell: Cell) {
        for y in (cy - r).max(0)..=(cy + r).min(self.height() as i32 - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(self.width() as i32 - 1) {
                if (x - cx).pow(2) + (y - cy).pow(2) <= r * r {
                    self.set(x as usize, y as usize, cell);
                }
            }
        }
    }

    /// A cell of ambient air, for erasing.
    pub fn ambient(&self) -> Cell {
        Cell::gas(&self.subs, &air(&self.subs), ROOM_T, AMBIENT_P)
    }

    /// Advance the simulation by one time step.
    pub fn step(&mut self) {
        // Run the whole step on one of the worker threads. If the caller's
        // thread ran it, that thread would be put to sleep at every parallel
        // section and woken after it, and on a laptop a core that has just
        // been asleep runs slowly for a while. Measured: the single-threaded
        // passes took twice as long that way.
        rayon::scope(|_| self.step_on_worker());
    }

    fn step_on_worker(&mut self) {
        self.chunks.advance();
        self.stats.active_chunks = self.chunks.active_count();

        let t0 = Instant::now();
        crate::derive::run(self);
        if self.params.verify_derived {
            crate::derive::verify(self);
        }
        let t1 = Instant::now();
        crate::heat::run(self);
        let t2 = Instant::now();
        crate::gas::run(self);
        let t3 = Instant::now();
        crate::grains::run(self);
        let t4 = Instant::now();
        crate::liquid::run(self);
        let t5 = Instant::now();

        self.stats.derive = t1 - t0;
        self.stats.heat = t2 - t1;
        self.stats.gas = t3 - t2;
        self.stats.grains = t4 - t3;
        self.stats.liquid = t5 - t4;
        self.step_count += 1;
    }

    /// Total moles of one substance across all forms and cells.
    pub fn total_moles(&self, id: crate::SubId) -> f64 {
        let mut n = 0.0f64;
        for c in &self.cells {
            for s in 0..crate::SLOTS {
                if c.id[s] == id {
                    n += c.amount[s] as f64;
                }
            }
        }
        n
    }

    /// Total energy in the world, joules.
    pub fn total_energy(&self) -> f64 {
        self.cells.iter().map(|c| c.energy as f64).sum()
    }
}

/// Small deterministic random number generator (xorshift64*).
pub struct Rng(pub u64);

impl Rng {
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    #[inline]
    pub fn coin(&mut self) -> bool {
        self.next_u32() & 1 == 0
    }

    /// Round `v` to an integer at random so the average equals `v`.
    /// This is how speeds below one cell per step are represented.
    #[inline]
    pub fn round(&mut self, v: f32) -> usize {
        let v = v.max(0.0);
        let whole = v.floor();
        whole as usize + (self.unit() < v - whole) as usize
    }
}
