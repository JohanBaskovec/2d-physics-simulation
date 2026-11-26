# 2D physics simulation

A grid-based 2D physics engine in Rust, meant to be used later for a game set underground (a sealed world of rock and caves). Target: all 94 natural elements in three phases with real densities and temperatures, falling-sand powders, diggable walls, basic chemistry, dissolving, 60 fps, world larger than a screen.

Read these before changing anything:

- `docs/design.md`: the settled design and the rules every pass must obey.
- `docs/roadmap.md`: build order, what is done, benchmark results, open problems.

## Commands

`cargo` is installed in `~/.cargo/bin` and may not be on PATH: `export PATH="$HOME/.cargo/bin:$PATH"`.

```
cargo test --release --workspace                 # 16 behaviour tests, about 6 s
cargo run --release -p viewer                    # interactive viewer
cargo run --release -p viewer                    # default scene is caves; --open for open air
cargo run --release -p viewer -- --stress        # benchmark scene, every chunk busy
cargo run --release -p viewer -- --screenshot /tmp/shot.png 120   # run 120 frames, save PNG, quit
cargo run --release -p sim --example bench       # everything, about 6 min; too long for one 600 s call
cargo run --release -p sim --example bench -- caves   # one scene by name: demo, caves, playground, "gas noise", "still air"
cargo run --release -p sim --example bench --features slots-6     # or slots-4
cargo clippy --release --workspace --all-targets
```

Always use `--release`. Debug builds are too slow to judge behaviour.

`--screenshot` is how to check rendering without the user: run it, then read the PNG. It opens a real window on the user's screen for a few seconds. If the window is hidden the run can stall; use `timeout` and retry once.

## Layout

```
crates/sim/                 the engine, no dependencies
  data/substances.csv       substance table (placeholder rows, see roadmap)
  src/lib.rs                constants: SLOTS, DT, GRAVITY, PACKING, CHUNK, tolerances
  src/cell.rs               Cell (64 bytes), Form, Kind, per-cell derived values
  src/substance.rs          CSV loader, FormProps (per substance and form)
  src/xfer.rs               moving matter between two cells: Pair, exchange_*, move_fast
  src/world.rs              World, Params, Chunks (sleeping), Dirty, step loop, Rng
  src/derive.rs             per-step snapshot arrays (temp, cond, pressure, flags, ...)
  src/heat.rs gas.rs grains.rs liquid.rs    the four passes
  src/pressure.rs           scalar pressure relaxation, called from the gas pass
  src/render.rs             cells to RGBA, three views
  src/scene.rs              caves (viewer default, the game's setting), playground (open air), demo (stress), gas_noise (worst case)
  examples/bench.rs         benchmark
  tests/physics.rs          behaviour tests
crates/viewer/              macroquad front end
```

Step order in `World::step`: derive, heat, gas, grains, liquid. Inside the gas pass: chunk summaries, buoyancy (`overturn`), pressure relaxation (`pressure::solve`), then one transfer per cell face.

## Rules that must not be broken

- **Conservation.** Cells store moles and energy only. Every movement goes through `xfer` and moves moles together with the enthalpy they carry. Nothing is created or destroyed except by the editing calls (`World::set`, `fill_rect`, `fill_circle`).
- **Block, never destroy.** If a cell has no slot for a substance, the transfer is refused and the matter stays where it was. This applies to reactions and phase changes too.
- **Empty slot invariant.** An unused slot has `id == 0`, `form == Form::None`, `amount == 0.0`. Sums over slots rely on this.
- **Data, not code.** The engine never names a substance. Scenes and tests look substances up by symbol; passes only read table rows.
- **Wake what you change.** Any change to a cell must call `Chunks::touch` (or go through `Dirty`), or the chunk may sleep with work left undone. A cell that is skipped but still has work pending must also touch.
- **Snapshot versus real state.** `world.derived` is computed once at the start of a step and goes stale as passes edit cells. Use it for cheap rejects and for picking partners; do the actual transfer on the real cells.

## Conventions

- Units are real: cells are 1 cm cubes, amounts in moles, energy in joules, temperature in kelvin, pressure in pascals, density in g/cm3. y points down.
- Tunable rates live in `Params` with a comment saying what the number means physically. Table data lives in the CSV.
- Comments explain why, and name the simplification when physics is being approximated.
- Add a behaviour test in `tests/physics.rs` for every new rule. Conservation is checked by `mixed_scene_conserves_matter_and_energy`; extend that scene when adding substances or forms.
- Measure before and after any change to a pass with the benchmark. `perf record` works on this machine for profiling.
- There is no git history to fall back on unless the user has committed. Before an experiment that rewrites a pass, copy the files to `/tmp` so it can be backed out exactly.
- Order matters between passes: anything that moves or heats gas before `pressure::solve` must also update the `derived` snapshot (see `heat.rs` and `gas::sink`), or the solver acts on stale pressures and produces spikes.

## Working with the user

- Settle design in conversation first. The user gives an explicit go-ahead before coding a new step, and corrects design details precisely; follow them to the letter and say so when deviating.
- Keep `docs/roadmap.md` and `docs/design.md` up to date in the same change as the code. The user expects documentation without being asked.
- Do not commit unless asked.
