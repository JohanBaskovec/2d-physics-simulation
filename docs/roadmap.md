# Roadmap and status

Last updated 2026-11-26.

## Build order

| Step | Content | Status |
|---|---|---|
| 1 | Grid, renderer, powder/liquid/gas movement, heat conduction, sleeping chunks, benchmark | Done 2026-11-26 |
| 2 | Phase changes; sourced table for all 94 elements plus compounds | Not started |
| 3 | Gas pressure done properly; liquid pressure at depth | Gas pressure done 2026-11-26, at a performance cost (see below). Liquid pressure not started. |
| 4 | Walls and digging; unsupported walls collapsing | Not started |
| 5 | Compounds, reactions, dissolving | Not started |
| 6 | Multithreading (checkerboard chunks); larger or streaming worlds | Not started |

The user chose to do gas pressure (step 3) before phase changes (step 2).

**Open decision:** what to do about performance, which got worse with correct gas pressure. Options are under "Performance after gas pressure" below.

## Benchmark results, step 1

Machine: AMD Ryzen 7 4700U, single thread, `cargo run --release -p sim --example bench`, 2026-11-26. Budget is 16.7 ms per step.

| Scene | Size | Step time | Active chunks |
|---|---|---|---|
| playground, first 5 s | 640×384 | 3.9 ms | 16 of 60 |
| playground, after 1 min | 640×384 | 8.1 ms | 28 of 60 |
| playground, after 1 min | 1024×512 | 9.0 ms | 34 of 128 |
| stress (`scene::demo`) | 640×384 | 25–30 ms | 60 of 60 |
| stress | 1024×512 | 55–65 ms | 128 of 128 |
| gas noise (worst case) | 640×384 | 34.5 ms | 60 of 60 |
| still air | any | under 0.1 ms | 0 |

Cost per active cell is 100–140 ns in busy scenes and about 60 ns in the playground.

Slot count, stress scene at 640×384: 8 slots 25–30 ms, 6 slots 23–27 ms, 4 slots 20–24 ms. Slot count is not the bottleneck.

Pass breakdown, stress scene after 30 s at 640×384: derive 4.4, heat 2.5, gas 16.7, grains 0.8, liquid 4.3 ms.

Optimisation history, same scene: 50–85 ms at first; 32–40 ms after the cheap gas transfer and liquid rejects; 25–30 ms after staggered diffusion, dirty rectangles and the at-rest skip.

## Performance after gas pressure

Same machine and command, 2026-11-26, after `pressure.rs` was added.

| Scene | Size | Before | After | Active chunks after |
|---|---|---|---|---|
| playground, first 5 s | 640×384 | 3.9 ms | 16.3 ms | 47 of 60 |
| playground, after 1 min | 640×384 | 8.1 ms | 22.3 ms | 58 of 60 |
| playground, first 5 s | 1024×512 | 4.1 ms | 17.7 ms | 54 of 128 |
| playground, after 1 min | 1024×512 | 9.0 ms | 38.8 ms | 105 of 128 |
| stress | 640×384 | 25–30 ms | 40–46 ms | 60 of 60 |
| stress | 1024×512 | 55–65 ms | 86–99 ms | 128 of 128 |
| gas noise | 640×384 | 34.5 ms | 51.2 ms | 60 of 60 |
| still air | any | under 0.1 ms | under 0.1 ms | 0 |

The viewer's starting scene no longer holds 60 fps on this machine: about 17 ms per step at first, 39 ms after a minute at 1024×512.

Why it is slower:

- **More of the world is awake, and that part is physically right.** The world is a sealed box. A warm object raises the pressure everywhere, so gas really does have to shift in every chunk. The old rule hid this by spreading pressure slowly. With a 0.2% dead band the whole 1024×512 world is involved within about 20 s of a 700 K bar being present.
- **An awake cell costs 80–90 ns**, of which gas is about 55. Profile of the playground: `gas::pair` 35% (moving gas across faces with a flux), `derive` 21%, `pressure::solve` 19%, heat 7%.

Options, not yet chosen:

1. Multithreading (step 6). Every pass is per chunk already; 8 cores would bring the playground back under budget.
2. Make awake-but-quiet cells cheaper: derive only cells that changed, merge the several per-chunk scans in the gas pass into one.
3. Give the world open edges held at ambient pressure, so a heat source does not pressurise everything. This changes what the world is, so it is the user's call.
4. A wider dead band. 1% would cut the work but leaves pressure visibly uneven; not recommended.

## Known problems

Gas:

- Performance, as above.
- Gas has no velocity: no wind that keeps blowing, no flow around obstacles. Light gas rises only within its own column. In the helium test about two thirds reaches the top band; the rest is left as a trail that then only diffuses.
- Pressure has a dead band of about 0.2% per pair, up to about 2% across a large region, and single cells touching something very hot can be about 4% off for a step.
- Compression does not heat gas and expansion does not cool it.

Liquids:

- No pressure at depth, so connected vessels do not equalise through a pipe at the bottom.
- No momentum: liquids level quickly but do not slosh or make waves.
- A tall column drains from its exposed edge, not by being pushed out at the base.

Other:

- `substances.csv` holds 8 placeholder rows typed from memory. Values marked `~` in the file header are rough guesses.
- No phase changes: a substance stays in whatever form it was painted.
- Piles settle at 45°; real dry sand is about 34°.
- Powder rendering speckle is tied to position, so it shimmers while grains fall.
- Clippy reports a few style lints (`is_multiple_of`, index loops); no errors.
- Stale mark in `World::stamp` can make a powder cell skip one step every 255 steps; it touches its chunk so it is picked up the next step.

## Tests

`crates/sim/tests/physics.rs`, 16 tests: cell size, dry sand density, sleeping in still air, the sand-into-water worked example, sand pile and sleep, water levelling, mercury under water, sand in water versus on mercury, helium rising, heat flow, dry versus wet sand conductivity, full-cell blocking, pressure evening out within a second, a sealed box keeping its pressure, air beside hot iron staying near ambient, and conservation in a mixed scene (matter and energy to 2e-5 relative over 1500 steps).

The feature builds `slots-6` and `slots-4` are only exercised by the benchmark; the tests have not been run with them.
