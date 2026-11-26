//! Step time of the cave scene. Run with RAYON_NUM_THREADS=n to compare.
use sim::{scene, Substances, World};
fn main() {
    let mut world = World::new(1024, 512, Substances::builtin());
    scene::caves(&mut world);
    for _ in 0..200 { world.step(); }
    let n = 400;
    let mut t = [0.0f64; 6];
    for _ in 0..n {
        world.step();
        let s = world.stats;
        for (k, d) in [s.derive, s.heat, s.gas, s.grains, s.liquid, s.total()].iter().enumerate() { t[k] += d.as_secs_f64() * 1e3 / n as f64; }
    }
    println!("threads {:>2}: total {:5.2} ms | derive {:.2} heat {:.2} gas {:.2} grains {:.2} liquid {:.2}", rayon::current_num_threads(), t[5], t[0], t[1], t[2], t[3], t[4]);
}
