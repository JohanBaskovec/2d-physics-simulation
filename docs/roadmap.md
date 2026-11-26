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
| 6 | Multithreading; larger or streaming worlds | First stage done 2026-11-26 with little gain; see "Multithreading, first stage" |

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
3. Open world edges held at ambient pressure (or vacuum, as in Oxygen Not Included). Ruled out by the user on 2026-11-26: the game is underground, so gas has nowhere to escape. Real gravity could not hold gas in anyway: air pressure only falls 0.06% over 5 m.
4. A wider dead band. 1% would cut the work but leaves pressure visibly uneven; not recommended.

## Cave scene (the game is underground)

The user's game is set underground, so the world is sealed by rock and open edges are off the table. `scene::caves` is the representative scene: solid rock with one chamber per 128×128 tile, some linked by tunnels. Five chambers in six hold something (water and sand, mercury and gold, helium, a 700 K bar, a 1500 K bar). It is the viewer's default; `--open` gives the old playground.

Measured 2026-11-26, same machine, `bench -- caves`:

| Size | First 5 s | 5 to 20 s | After 1 min | Awake chunks |
|---|---|---|---|---|
| 640×384 | 16.8 ms | 12.3 ms | 10.7 ms | about 50 of 60 |
| 1024×512 | 26.0 ms | 26.0 ms | 26.4 ms | about 105 of 128 |

The guess that rock would make this cheap was wrong:

- **Cost goes by awake chunk, not by awake gas cell.** A chunk is 64×64 and is simulated whole if anything in it is active. Chambers and tunnels touch most chunks, so most chunks are awake even though most cells are rock.
- **A rock cell in an awake chunk is not free.** It is still derived (about 16 ns) and scanned by each pass. An awake chunk costs 0.21–0.25 ms whatever it holds, so the 16.7 ms budget is about 65–80 awake chunks, roughly one screen at zoom 2.
- **Helium and hot bars never settle**, so their chambers stay awake for the whole run. Water and sand chambers do go to sleep.

### After recomputing only changed cells (2026-11-26)

| Scene | Size | Before | After |
|---|---|---|---|
| caves, after 1 min | 640×384 | 10.7 ms | 6.5 ms |
| caves, after 1 min | 1024×512 | 26.4 ms | 17.5 ms |
| playground, after 1 min | 640×384 | 22.3 ms | 19.7 ms |
| playground, after 1 min | 1024×512 | 38.8 ms | 33.9 ms |

The derive pass went from 7.1 ms to 1.8 ms in the 1024×512 cave scene. An awake chunk in the cave scene now costs about 0.17 ms, so the budget is about 100 awake chunks.

What is left in that scene, per step: gas 10.6 ms (pressure relaxation 4.4, moving gas across faces 5.3, chunk summaries 0.9), heat 3.5, derive 1.8, liquid 1.0, grains 0.6. About 102 of 108 awake chunks have real pressure work every step, because half the chambers hold helium or a hot bar and many are linked by tunnels. This scene is busier than a normal moment in a game.

Three further changes were tried on the gas pass and gained nothing measurable in this scene: keeping open-run lengths until the flags change (kept, it is sound), confining repeated local smoothing to a box around the pairs that moved a lot (kept), and not re-triggering relaxation while chunk pressure sums are unchanged (removed: no gain, and it could miss two compensating changes).

What would help next, cheapest first: multithreading; a tighter inner loop for the face transfers and the adjacent-pair pass; smaller chunks, 32×32 (needs the pressure pass reworked, which assumes pairs of 64 fit in a chunk).

### Multithreading, first stage (2026-11-26)

Uses rayon. Parallel over rows of chunks ("bands"): derive, heat, the gas chunk summaries, and the gas face transfers. Still serial: buoyancy, the pressure relaxation, powders, liquids.

| Cave scene, after 1 min | Single-threaded | 8 threads |
|---|---|---|
| 640×384 | 6.5 ms | 7.5 ms |
| 1024×512 | 17.5 ms | 15.4 ms |

By thread count at 1024×512 (`examples/threads.rs`, same session, single-threaded build 18.8 ms): 1 thread 20.6 ms, 2 threads 18.0, 4 threads 17.6, 8 threads 16.7.

Findings:

- **The passes do not scale with cores.** Derive goes 2.07 to 1.19 ms and heat 4.24 to 2.07 ms from 1 to 8 threads; gas barely moves (12.3 to 10.8 ms) even though its face transfers, about half of it, are parallel.
- **Most likely cause: memory, not arithmetic.** Each pass streams over several whole-world arrays (about 40 MB of derived and scratch data at 1024×512, against 8 MB of cache), and there are around eight such sweeps per step. Cores share one memory bus. This also explains why three earlier attempts to trim arithmetic in the gas pass gained nothing. Not yet proven with a hardware counter.
- **The smaller world got slower**, because it fits in cache better, so thread overhead and the heat pass's doubled arithmetic (each face is now evaluated from both sides so cells can be written independently) are not paid back.
- **The step must run on a worker thread** (`rayon::scope` in `World::step`). Run from the caller's thread, every parallel section put that thread to sleep, and the serial passes after it ran at half speed on this laptop.

What this points to next: keep a chunk's data in cache while doing all the gas work on it (process chunk by chunk through summaries, relaxation and transfers, instead of pass by pass over the world), and shrink what is streamed (pack the derived values per cell, drop arrays that can be recomputed). Parallelising the remaining serial passes should come after that, since it would hit the same wall.

The user chose to keep this stage (committed 2026-11-26) and continue with the memory work.

Superseded list from before that work: make solid cells nearly free inside awake chunks (skip derive for cells that did not change; skip rows or runs of rock in the gas passes); smaller chunks, 32×32 (needs the pressure pass reworked, which assumes pairs of 64 fit in a chunk); multithreading.

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

`crates/sim/tests/physics.rs`, 17 tests: cell size, dry sand density, sleeping in still air, the sand-into-water worked example, sand pile and sleep, water levelling, mercury under water, sand in water versus on mercury, helium rising, heat flow, dry versus wet sand conductivity, full-cell blocking, pressure evening out within a second, a sealed box keeping its pressure, air beside hot iron staying near ambient, kept derived values matching a full recompute in the cave scene, and conservation in a mixed scene (matter and energy to 2e-5 relative over 1500 steps).

The feature builds `slots-6` and `slots-4` are only exercised by the benchmark; the tests have not been run with them.
