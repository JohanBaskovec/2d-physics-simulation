//! Step-time benchmark.
//!
//!   cargo run --release -p sim --example bench
//!   cargo run --release -p sim --example bench --features slots-6
//!   cargo run --release -p sim --example bench --features slots-4

use sim::world::StepStats;
use sim::{scene, Cell, Substances, World, CHUNK, SLOTS};
use std::time::Duration;

const BUDGET_MS: f64 = 1000.0 / 60.0;

struct Summary {
    steps: usize,
    sum: StepStats,
    totals: Vec<Duration>,
    chunks: usize,
}

fn measure(world: &mut World, steps: usize) -> Summary {
    let mut s = Summary { steps, sum: StepStats::default(), totals: Vec::with_capacity(steps), chunks: 0 };
    for _ in 0..steps {
        world.step();
        let st = world.stats;
        s.sum.derive += st.derive;
        s.sum.heat += st.heat;
        s.sum.gas += st.gas;
        s.sum.grains += st.grains;
        s.sum.liquid += st.liquid;
        s.chunks += st.active_chunks;
        s.totals.push(st.total());
    }
    s.totals.sort();
    s
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn report(name: &str, world: &World, s: &Summary) {
    let n = s.steps as u32;
    let mean = ms(s.sum.total() / n);
    let p99 = ms(s.totals[(s.steps * 99 / 100).min(s.steps - 1)]);
    let max = ms(*s.totals.last().unwrap());
    let all_chunks = (world.width() / CHUNK) * (world.height() / CHUNK);
    let active = s.chunks as f64 / s.steps as f64;
    let ns_per_cell = if active > 0.0 { mean * 1e6 / (active * (CHUNK * CHUNK) as f64) } else { 0.0 };
    println!(
        "{name:<24} mean {mean:6.2} ms  p99 {p99:6.2}  max {max:6.2} | derive {:5.2} heat {:5.2} gas {:5.2} grains {:5.2} liquid {:5.2} | chunks {active:5.1}/{all_chunks} | {ns_per_cell:5.1} ns/active cell | {}",
        ms(s.sum.derive / n),
        ms(s.sum.heat / n),
        ms(s.sum.gas / n),
        ms(s.sum.grains / n),
        ms(s.sum.liquid / n),
        if p99 <= BUDGET_MS { "fits 60 fps" } else { "OVER 16.7 ms" },
    );
}

fn main() {
    println!(
        "slots per cell: {SLOTS}   cell size: {} bytes   single thread",
        std::mem::size_of::<Cell>()
    );
    for (w, h) in [(640, 384), (1024, 512)] {
        println!("\n{w} x {h} cells ({:.1} MB of cells)", (w * h * std::mem::size_of::<Cell>()) as f64 / 1e6);

        let mut world = World::new(w, h, Substances::builtin());
        scene::demo(&mut world);
        let s = measure(&mut world, 300);
        report("demo, first 5 s", &world, &s);
        let s = measure(&mut world, 300);
        report("demo, 5 to 10 s", &world, &s);
        for _ in 0..1200 {
            world.step();
        }
        let s = measure(&mut world, 120);
        report("demo, after 30 s", &world, &s);

        if w >= 448 && h >= 256 {
            let mut world = World::new(w, h, Substances::builtin());
            scene::playground(&mut world);
            let s = measure(&mut world, 300);
            report("playground, first 5 s", &world, &s);
            let s = measure(&mut world, 900);
            report("playground, 5 to 20 s", &world, &s);
            for _ in 0..2400 {
                world.step();
            }
            let s = measure(&mut world, 300);
            report("playground, after 1 min", &world, &s);
        }

        let mut world = World::new(w, h, Substances::builtin());
        scene::gas_noise(&mut world);
        let s = measure(&mut world, 60);
        report("gas noise (worst case)", &world, &s);

        let mut world = World::new(w, h, Substances::builtin());
        let s = measure(&mut world, 120);
        report("still air", &world, &s);
    }
}
