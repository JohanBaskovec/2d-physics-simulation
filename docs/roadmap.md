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
| 6 | Multithreading; larger or streaming worlds | Most passes parallel as of 2026-11-26; see "Performance work, second round". Buoyancy, powders and liquids are still serial. |

The user chose to do gas pressure (step 3) before phase changes (step 2).

Performance got worse with correct gas pressure and has since been brought back under budget for the cave scene; the history is below, newest last. Current numbers are under "Performance work, second round".

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

Findings at the time:

- **The passes did not scale with cores.** Derive went 2.07 to 1.19 ms and heat 4.24 to 2.07 ms from 1 to 8 threads; gas barely moved (12.3 to 10.8 ms).
- **The step must run on a worker thread** (`rayon::scope` in `World::step`). Run from the caller's thread, every parallel section put that thread to sleep, and the serial passes after it ran at half speed on this laptop.
- **Wrong diagnosis, corrected below.** This section first blamed memory traffic (40 MB of arrays against 8 MB of cache) and proposed cache blocking. That was a guess; the counters say otherwise.

The user chose to keep this stage (committed 2026-11-26).

### Performance work, second round (2026-11-26)

Measured with `perf stat` before changing anything:

| Run | Clock | Instructions per cycle |
|---|---|---|
| 1 thread | 4.16 GHz | 2.85 |
| 8 separate single-threaded copies at once | 3.37 GHz each | 2.2 each |
| 8 threads, first-stage build | 2.19 GHz average, 2.4 cores in use | 2.82 |

- **The step is limited by arithmetic, not memory.** 2.85 instructions per cycle and 2% first-level cache misses is a processor that is busy, not waiting. The step executed 225 million instructions.
- **Threads gained little for two other reasons.** Half the step was still serial, and helper cores that sit idle between short parallel sections run at a low clock (and take up to 0.4 ms to wake). Eight fully busy cores on this laptop are worth about five single cores.
- **Two threads give almost everything eight do**, and on the small world two are faster than eight. Not understood beyond the point above; the thread count is left at rayon's default.

What was changed, each measured:

| Change | Effect on the 1024×512 cave scene, 1 thread |
|---|---|
| Heat: each face worked out once, in loops with no branches, which the compiler vectorises | heat 3.9 to 1.5 ms |
| Gas passes skip rock, using a per-row range of non-wall cells (`Derived::row_span`) | pressure 4.3 to 2.6 ms, summaries 0.95 to 0.5 ms |
| Gas exchange between two cells laid out alike (air next to air, 91% of exchanges) done in one short loop (`xfer::trade_gas_aligned`) | faces 5.6 to 4.2 ms |
| Pressure relaxation in parallel, two alternating rounds of chunk rows | pressure 2.6 to 1.9 ms at 8 threads |
| Face transfers in strips of 16 rows instead of whole chunk rows | faces 3.3 to 2.2 ms at 8 threads |
| A step with nothing awake returns at once | still air back to 0.00 ms |
| Renderer draws pixel rows in parallel | one 640×360 view 4.5 to 0.9 ms |

Old and new builds run back to back (`examples/threads.rs`, cave scene after 200 steps):

| Size | Before, 1 thread | Before, 8 threads | After, 1 thread | After, 2 threads | After, 8 threads |
|---|---|---|---|---|---|
| 640×384 | 7.6 ms | 7.1 ms | 5.2 ms | 4.0 ms | 4.8 ms |
| 1024×512 | 19.1 ms | 15.3 ms | 14.2 ms | 9.4 ms | 9.1 ms |

Benchmark (`bench`, 8 threads, after 1 min):

| Scene | Size | Mean | p99 | Before this round |
|---|---|---|---|---|
| caves | 640×384 | 3.7 ms | 4.8 ms | 7.5 ms |
| caves | 1024×512 | 8.6 ms | 12.6 ms | 15.4 ms |
| playground | 640×384 | 8.9 ms | 14.1 ms | 19.7 ms (single-threaded build) |
| playground | 1024×512 | 14.9 ms | 18.7 ms | 33.9 ms (single-threaded build) |

The cave scene now fits the 16.7 ms budget at 1024×512 with room to spare. The open-air playground fits on average at that size but its slowest steps do not.

What is left, 1024×512 caves at 8 threads: gas 5.2 ms (faces 2.2, pressure 1.9, buoyancy 0.9, summaries 0.2), liquid 1.3, derive 0.95, grains 0.9, heat 0.8.

Not verified: frame rate in the viewer after these changes. One screenshot run before the renderer change showed 50 fps with an 11.5 ms step (the step is slower there than in the benchmark because cores idle between frames). Later runs stalled because the window was hidden.

What would help next:

- **Parallelise buoyancy, powders and liquids.** They are serial, about 3 ms together, and run slower when other cores have just been busy. Buoyancy needs a random number stream per chunk to stay independent of thread count.
- **Trim the face transfer further.** It still executes about 500 instructions per exchange.
- **Cells that are not laid out alike** (helium mixing into air) take the slow exchange path. Keeping slots in a fixed order per substance would make every exchange the quick kind.
- **AVX2.** Building with `-C target-cpu=x86-64-v3` gained about 4% overall (heat 1.6 to 1.25 ms). Not adopted; it restricts which processors the program runs on.

## Known problems

Gas:

- Performance: see "Performance work, second round". Buoyancy, powders and liquids are still single-threaded.
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

`crates/sim/tests/physics.rs`, 19 tests: cell size, dry sand density, sleeping in still air, the sand-into-water worked example, sand pile and sleep, water levelling, mercury under water, sand in water versus on mercury, helium rising, heat flow, dry versus wet sand conductivity, full-cell blocking, pressure evening out within a second, a sealed box keeping its pressure, air beside hot iron staying near ambient, kept derived values matching a full recompute in the cave scene, the quick gas exchange matching the general one, results identical for 1 and 5 threads, and conservation in a mixed scene (matter and energy to 2e-5 relative over 1500 steps).

The feature builds `slots-6` and `slots-4` are only exercised by the benchmark; the tests have not been run with them.
