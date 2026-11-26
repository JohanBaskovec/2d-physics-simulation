//! Turns cells into pixels. No graphics dependency: this fills an RGBA
//! buffer that any front end can display.

use crate::cell::Form;
use crate::world::World;
use crate::{ATM, SLOTS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Substance colours blended by volume.
    Material,
    /// False colour by temperature.
    Temperature,
    /// False colour by gas pressure.
    Pressure,
}

impl View {
    pub fn next(self) -> View {
        match self {
            View::Material => View::Temperature,
            View::Temperature => View::Pressure,
            View::Pressure => View::Material,
        }
    }
}

/// Fill `out` (RGBA, `vw * vh * 4` bytes) with the cells of the rectangle
/// whose top-left corner is (x0, y0). Cells outside the world are black.
pub fn draw(world: &World, view: View, x0: i32, y0: i32, vw: usize, vh: usize, out: &mut [u8]) {
    assert!(out.len() >= vw * vh * 4);
    let (w, h) = (world.width() as i32, world.height() as i32);
    for py in 0..vh {
        for px in 0..vw {
            let (x, y) = (x0 + px as i32, y0 + py as i32);
            let rgb = if x < 0 || y < 0 || x >= w || y >= h {
                [0, 0, 0]
            } else {
                let i = y as usize * w as usize + x as usize;
                // Read the cell itself, not the per-step snapshot: the
                // snapshot is a step old for cells that have just changed.
                let c = &world.cells[i];
                match view {
                    View::Material => material(world, i, x as u32, y as u32),
                    View::Temperature => heat_ramp(c.temperature(&world.subs)),
                    View::Pressure => pressure_ramp(c.pressure(&world.subs)),
                }
            };
            let o = (py * vw + px) * 4;
            out[o..o + 3].copy_from_slice(&rgb);
            out[o + 3] = 255;
        }
    }
}

fn material(world: &World, i: usize, x: u32, y: u32) -> [u8; 3] {
    let c = &world.cells[i];
    let subs = &world.subs;
    let mut rgb = [0.0f32; 3];
    let mut filled = 0.0f32;
    let (mut gas_rgb, mut gas_n) = ([0.0f32; 3], 0.0f32);
    let mut grains = false;
    for s in 0..SLOTS {
        if c.form[s] == Form::None {
            continue;
        }
        let col = subs.get(c.id[s]).color;
        if c.form[s] == Form::Gas {
            gas_n += c.amount[s];
            for k in 0..3 {
                gas_rgb[k] += c.amount[s] * col[k] as f32;
            }
        } else {
            let mut v = c.slot_volume(subs, s);
            if c.form[s] == Form::Granular {
                grains = true;
            } else if c.form[s] == Form::Liquid && c.is_powder() {
                // Liquid in pores only darkens the grains a little.
                v *= 0.5;
            }
            filled += v;
            for k in 0..3 {
                rgb[k] += v * col[k] as f32;
            }
        }
    }
    // Whatever volume is left shows the gas, dimmed by how thin it is.
    let rest = (1.0 - filled).max(0.0);
    if grains && filled > 0.0 {
        // Draw powder at full strength; its pores are too small to see.
        for v in &mut rgb {
            *v /= filled;
        }
    } else if gas_n > 0.0 {
        let opacity = (c.pressure(subs) / ATM).clamp(0.0, 1.0);
        for k in 0..3 {
            rgb[k] += rest * opacity * gas_rgb[k] / gas_n;
        }
    }
    if grains {
        // Speckle so powder reads as grains and walls as smooth.
        let n = hash(x, y) as f32 / 255.0;
        let shade = 0.82 + 0.26 * n;
        for v in &mut rgb {
            *v *= shade;
        }
    }
    [rgb[0].min(255.0) as u8, rgb[1].min(255.0) as u8, rgb[2].min(255.0) as u8]
}

fn hash(x: u32, y: u32) -> u8 {
    let mut v = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA6B);
    v ^= v >> 15;
    v = v.wrapping_mul(0x2C1B_3C6D);
    (v >> 24) as u8
}

/// Black for vacuum, then blue (cold) through grey (room temperature) to
/// red, yellow and white (hot). Logarithmic so 100 K and 3000 K both show.
fn heat_ramp(t: f32) -> [u8; 3] {
    if t <= 0.0 {
        return [0, 0, 0];
    }
    const STOPS: [(f32, [f32; 3]); 6] = [
        (50.0, [20.0, 20.0, 120.0]),
        (200.0, [40.0, 110.0, 230.0]),
        (293.15, [110.0, 110.0, 110.0]),
        (600.0, [220.0, 50.0, 30.0]),
        (1500.0, [250.0, 210.0, 40.0]),
        (4000.0, [255.0, 255.0, 255.0]),
    ];
    ramp(&STOPS, t)
}

/// Black for vacuum, blue below one atmosphere, grey at one, red above.
fn pressure_ramp(p: f32) -> [u8; 3] {
    if p <= 0.0 {
        return [0, 0, 0];
    }
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.01 * ATM, [20.0, 20.0, 120.0]),
        (0.5 * ATM, [40.0, 110.0, 230.0]),
        (ATM, [110.0, 110.0, 110.0]),
        (2.0 * ATM, [220.0, 50.0, 30.0]),
        (20.0 * ATM, [250.0, 230.0, 60.0]),
    ];
    ramp(&STOPS, p)
}

fn ramp(stops: &[(f32, [f32; 3])], v: f32) -> [u8; 3] {
    let last = stops.len() - 1;
    if v <= stops[0].0 {
        return stops[0].1.map(|c| c as u8);
    }
    if v >= stops[last].0 {
        return stops[last].1.map(|c| c as u8);
    }
    let k = (1..=last).find(|&k| v < stops[k].0).unwrap_or(last);
    let (v0, c0) = stops[k - 1];
    let (v1, c1) = stops[k];
    let f = (v.ln() - v0.ln()) / (v1.ln() - v0.ln());
    [0, 1, 2].map(|i| (c0[i] + f * (c1[i] - c0[i])) as u8)
}
