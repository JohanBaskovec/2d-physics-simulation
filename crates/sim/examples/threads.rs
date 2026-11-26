//! Step time of the cave scene, by pass. Run with RAYON_NUM_THREADS=n to
//! compare thread counts. Optional arguments: width height.
use sim::{scene, Substances, World};

fn main() {
    let args: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (w, h) = if args.len() == 2 { (args[0], args[1]) } else { (1024, 512) };
    let mut world = World::new(w, h, Substances::builtin());
    scene::caves(&mut world);
    for _ in 0..200 {
        world.step();
    }
    let n = 400;
    let mut t = [0.0f64; 10];
    for _ in 0..n {
        world.step();
        let s = world.stats;
        let g = s.gas_parts;
        let parts = [s.total(), s.derive, s.heat, s.gas, s.grains, s.liquid, g[0], g[1], g[2], g[3]];
        for (k, d) in parts.iter().enumerate() {
            t[k] += d.as_secs_f64() * 1e3 / n as f64;
        }
    }
    // Drawing one 640 x 360 view (the viewer's window at zoom 2).
    let mut pixels = vec![0u8; 640 * 360 * 4];
    let t0 = std::time::Instant::now();
    for _ in 0..50 {
        sim::render::draw(&world, sim::render::View::Material, 0, 0, 640, 360, &mut pixels);
    }
    let draw_ms = t0.elapsed().as_secs_f64() * 1e3 / 50.0;
    println!(
        "threads {:>2}: draw {draw_ms:.2} ms | total {:5.2} ms | derive {:.2} heat {:.2} gas {:.2} (summary {:.2} buoyancy {:.2} pressure {:.2} faces {:.2}) grains {:.2} liquid {:.2}",
        rayon::current_num_threads(), t[0], t[1], t[2], t[3], t[6], t[7], t[8], t[9], t[4], t[5]
    );
}
