# Design

This records what was decided and why. Decisions marked **(user)** were specified by the user and should not be changed without asking.

## Approach

A grid-based continuum simulation in the family of The Powder Toy, Noita and Oxygen Not Included. Every substance is a row of data and one shared set of rules applies to all of them. Molecular dynamics was rejected: it only works at nanometre scale and needs a force model per element.

Store conserved quantities, derive the rest. A cell holds moles of each substance and its total energy. Temperature, pressure, density and conductivity are computed from those every step, so mass and energy cannot drift.

## The cell

Cells are 1 cm cubes, so volumes and densities are in real units. The cell size is the resolution: a droplet or a grain does not exist individually, only as an amount inside a cell.

Layout, 64 bytes with 8 slots (one cache line):

| Field | Size | Meaning |
|---|---|---|
| `amount: [f32; 8]` | 32 | moles per slot |
| `energy: f32` | 4 | joules, for the whole cell |
| `id: [u16; 8]` | 16 | substance per slot, 0 = empty |
| `vel: [i16; 2]` | 4 | bulk velocity, 1/256 cell per step |
| `form: [Form; 8]` | 8 | rigid, granular, liquid or gas, per slot |

- **Slot count is a compile-time constant, 8 (user).** Two extras were too few: damp sand needs water, nitrogen and oxygen in its pores; burning iron filings need nitrogen, oxygen and solid iron oxide; air plus CO2 plus water vapour is four. Benchmarked against 6 and 4: 8 costs only about 15–20% more, so it stays.
- **A slot is a substance in a form (user agreed).** Phase is per slot, so a boiling cell holds liquid water and steam side by side and boiling moves moles between the two slots.
- **Overflow: block, never destroy (user).** A substance with no free slot cannot enter and stays in the neighbouring cell. Reactions and phase changes whose products have no slot do not happen that step.
- **Traces (user).** When what is left of a slot after a transfer is below `TRACE_MOL`, the remainder goes along with the transfer, which frees the slot. A trace with no neighbour holding the same substance keeps its slot; it is never deleted.
- There is no room left in the cell. Per-step bookkeeping goes in separate arrays (`World::stamp`, `Derived`).

## Energy and temperature

Molar enthalpy of a substance in a form is `h0 + cp * T`, measured from the solid at 0 K. The offsets include the latent heats:

- solid: `h0 = 0`
- liquid: `h0 = (cp_s - cp_l) * T_melt + L_fus`
- gas: `h0 = h0_liquid + (cp_l - cp_g) * T_boil + L_vap`

Cell temperature is `(E - sum(n * h0)) / sum(n * cp)`. Moving moles from one form to another at constant cell energy therefore absorbs or releases latent heat with no extra state, which gives melting and boiling plateaus for free.

Rules for the phase-change pass, not written yet **(user)**:

- A blocked phase change is safe. Energy is the stored quantity, so the temperature just overshoots the transition point (superheating) and the change catches up when a slot frees. The code must say this where it happens.
- When a slot converts completely within one step, change its form byte in place, so a complete conversion never needs a free slot.
- Boiling point shifts with local pressure (Clausius–Clapeyron). Helium only solidifies under pressure; carbon and arsenic sublime. These need the pressure model.

Energy is carried as enthalpy, so gas does not heat on compression or cool on expansion. This is a known simplification.

## Cell behaviour (Kind)

Decided from what the cell holds **(user)**:

- **Powder**: any granular slot. Grains fill at most 60% of the cell (`PACKING`); the rest is pore space holding fluid. Dry sand is 1.6 g per cell and saturated sand 2.0 g, both realistic.
- **Fixed**: rigid volume above 50% and no grains. Stays in place and is sealed to flow.
- **Fluid**: everything else, including vacuum. Liquids and gases flow per slot by amount. Rigid pieces below 50% are carried along with the liquid (the code treats loose rigid slots like liquid slots).

Liquid and gas amounts are continuous; a cell can be partly full. Only grains are whole-cell **(user)**. Suspensions (grains below 60% in liquid) are out of version 1 **(user)**; they can come back later as fine solid carried in a liquid cell.

Two granular substances in one cell share the 60% limit, move as one cell and never separate. Accepted for version 1.

## Movement rules

All movement trades equal volumes, so cells never overfill and gas is never squeezed.

- **Grains** move as whole cells: straight down, else diagonally (45° piles), swapping with the fluid cell they enter. Speed accumulates under gravity with buoyancy, capped by a drag-limited terminal speed for a 1 mm grain. Speeds below one cell per step are handled by random rounding. A powder cell rises if the fluid cell above is heavier than the whole powder cell.
- **Pore exchange (user's worked example).** After the swap, denser fluid above trades places with lighter pore fluid. A sand cell (60% sand, 40% air) falling into a full water cell gives a lower cell of 60% sand with 40% water and an upper cell holding the other 60% of the water plus the pore air. Tested exactly.
- **Liquids** fall into gas space below while the displaced gas comes up; a denser liquid above swaps with a lighter one below. Falling liquid follows its velocity for several cells per step.
- **Levelling.** Supported liquid levels with the emptiest cell within `liquid_reach` (8) cells on its row, per liquid, so dense liquids spread under light ones. Differences below `level_tol` do not flow, which stops puddles spreading into endless films.
- **Seepage.** Liquid through powder is capped at `seep_vol` per step (0.1 cm/s, coarse sand). Gas through powder is slowed by `pore_gas_flow`.
- **Gas** has three rules, in this order each step: buoyancy, pressure, diffusion. Buoyancy: a heavy parcel above lighter gas trades places, whole, with the lowest clearly-light cell in the run beneath it (`gas::sink`). Pressure: see the next section. Diffusion: each pair of neighbours trades a small equal volume once every 8 steps.
- **Heat** conducts across cell faces by Fourier's law with the harmonic mean of the two conductivities, capped at 20% of full equalisation per pair for stability.

## Gas pressure

Pressure in connected gas should even out almost instantly (sound crosses the world in well under a step). Flow between neighbours only is slow diffusion: it left 2.3 atm standing next to a 1500 K bar and cost the most of any pass.

`pressure.rs` relaxes the pressure field on plain numbers, then the gas pass moves real gas once per cell face.

- **Pairs at three distances.** Adjacent cells every step, plus cells 8 or 64 apart on alternate steps. A distant pair is only used if every cell between the two is open gas (`OPEN` flag, run lengths computed from this step's flags), so it stands for gas shifting along a real straight path.
- **Flux along the path.** A pair's exchange is recorded at its first and last face; a running sum along each row and column turns that into a flux through every face on the path. Each cell then hands a little gas to the next, so composition and heat never jump.
- **No leaks.** Flux is only applied across a face between two cells that are not `Fixed`, and the running sum restarts at walls. A sealed box keeps its pressure to 0.1% over 600 steps (tested).
- **Caps.** A long pair may move at most `0.25 / distance` of the lower pressure per step, because up to `distance` paths cross one face and a cell can only hand on a small share of its gas accurately. Sharp local differences are left to adjacent pairs, which repeat up to 6 times per chunk while anything moved more than 0.4%.
- **Per chunk, with a dead band.** Each chunk records its lowest and highest pressure and gas density. It starts relaxing when the pressure spread across it and its right and lower neighbours exceeds `rel_tol` (0.2%), and stops when its pairs agree to a quarter of that. Chunks with no flux and uniform density skip the face loop entirely.
- **Order matters.** Heat is applied first and updates the snapshot (temperature, pressure, pV); buoyancy runs next and swaps snapshot entries along with the gas; only then is pressure relaxed. Each of these was found by a spike of 10–20% when it was missing.

Results: a 16×16 block at 3 atm evens out to within 2% in one second; air beside a 1500 K bar stays within about 4% of ambient for single cells and about 1% otherwise.

Tried and backed out: keeping fluxes pending across steps and applying a face only once it had collected 0.5% of a cell's gas. It made pressure less accurate (11% spikes) and the step slower, because chunks never finished relaxing.

Still simplified: gas has no momentum (no wind that keeps blowing, no sound), light gas rises only within its own column, compression does not heat gas, and the weight of the gas column is ignored.

## Deviations from what was first agreed

- **Powder conductivity is a geometric mean, not volume-weighted.** The volume-weighted mix made dry sand conduct at about 60% of solid quartz. The geometric mean gives 0.2–0.8 W/(m K) dry and 1–2.5 wet, which is the insulation effect the design wanted. Other cells use the volume-weighted mean.
- **Liquid levelling reaches 8 cells.** Neighbour-only levelling is diffusion and took over a minute to flatten an 80-cell basin.
- **Gas buoyancy is a whole-parcel swap with a random chance.** Swapping a small fraction each step only blurred the boundary and light gas stalled.

## Performance design

- The world is a flat grid split into 64×64 logical chunks. A chunk is simulated only if something in it or bordering it changed last step.
- `Derived` holds temperature, conductivity, heat capacity, gas pressure, gas volume, gas density, gas pV, fill level and flags **(user: derive once so the heat pass does not walk eight slots per neighbour lookup)**.
- **Only changed cells are re-derived.** `Chunks::touch` marks a cell stale, and the derive pass recomputes marked cells only. For an unchanged cell (rock around a cave, the inside of a pool) the one thing that moves is temperature, and the heat pass keeps that current as it applies each step's heat. A cell that is conducting heat is therefore still fully simulated; what is skipped is recomputing properties that depend on contents that did not change. Each chunk is recomputed in full every 64 steps so that temperatures carried forward by addition cannot drift.
- **Safety net.** `Params::verify_derived` recomputes every active cell after each derive pass and panics on a mismatch. Two tests run with it on. A change to a cell that is not marked gives silently wrong physics, so any new code that edits cells must go through `touch` or set `stale`, and should be covered by a verified test.
- The heat pass collects all flows first (`World::heat_gain`) and applies them afterwards, so results do not depend on visiting order and the snapshot can be brought up to date for the gas pass.
- Gas uses a cheap f32 transfer (`move_fast`) for partial flows and the exact f64 `Pair` when a cell may be emptied. The f64 path exists because a cell that loses almost all its matter would otherwise get its leftover energy from the difference of two large f32 numbers.
- Liquid cells deep inside a still pool are skipped by a snapshot test (`liquid::at_rest`).
- **Rock is stepped over.** `Derived::row_span` holds, for every row of every chunk, the range of columns outside which the row is all wall. It is redone when a flag in the chunk changes. The gas passes (summaries, pressure, face transfers) only walk that range. Heat cannot use it, since rock conducts. The pressure pass relies on the flux arrays being zero in wall cells: nothing writes flux into a wall, and a chunk whose walls moved is cleared whole once.
- **Heat is written for the compiler to vectorise.** Each task goes down its rows keeping three lines of face flows (right faces of this row, bottom faces of this row and of the row above) and works every face out once, with no branches. The faces on a task's top edge are also worked out by the task above, from the same numbers in the same order, so both sides agree exactly and energy is conserved.
- **Quick gas exchange.** When two cells hold the same substances in the same slots (air next to air), `xfer::trade_gas_aligned` moves all gases both ways in one short loop with no slot search. It gives up, changing nothing, if the layouts differ or a slot would drop below a trace, and the general path takes over.
- **Threads.** Derive, heat, gas summaries, pressure relaxation, gas face transfers and rendering run in parallel using rayon; buoyancy, powders and liquids are serial. A task may only write its own rows. Passes that reach into the rows below (pressure relaxation: one row of chunks; face transfers: strips of 16 rows) run in two alternating rounds, even then odd, so two tasks in a round never touch the same cell. The flux running sums are done per row of chunks and then carried across chunk boundaries in a short serial pass. Results are identical for any number of threads (tested bit for bit).
- **A world at rest costs nothing.** If no chunk is awake, `World::step` returns before waking any thread.
- **Where the time goes** is arithmetic, not memory traffic (measured: 2.85 instructions per cycle). See the roadmap for numbers.

## Planned but not designed in detail

- **Walls and digging.** A wall cell is 100% solid and a powder cell 60%, so digging one wall cell makes about 1.7 cells of powder. Crumble only when there is room for the extra.
- **Chemistry.** A reaction table keyed by substance pair: products, activation temperature, rate, heat released. Many entries generated by rule (alkali metal + water). Powders react where blocks only rust because a powder shares its cell with the gas.
- **Dissolving.** Solute is a slot in a liquid cell, capped by a solubility table; evaporation leaves it behind. Most pure elements do not dissolve in water, so this mostly applies to compounds and a few gases.
- **Data gaps.** Astatine and francium have only predicted bulk properties.
