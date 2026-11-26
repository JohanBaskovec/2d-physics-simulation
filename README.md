# 2D physics simulation

A grid-based 2D physics engine in Rust: real substances with temperature, density and pressure, falling-sand powders, liquids and gases. Work in progress; see `docs/roadmap.md` for status.

## Run

```
cargo run --release -p viewer
```

| Input | Action |
|---|---|
| Left mouse | paint |
| Right mouse | erase to air |
| 1–9 | choose substance |
| Tab | paint as wall, powder, liquid or gas |
| `-` `=` | brush size |
| `[` `]` | paint temperature |
| Mouse wheel | zoom |
| WASD, arrows, middle-drag | pan |
| Space | pause |
| `.` | single step |
| V | material, temperature or pressure view |
| R | reset scene |
| C | clear to air |
| H | hide help |

Viewer options: `--open` (open-air scene instead of caves), `--empty`, `--stress`, `--size W H` (multiples of 64), `--screenshot PATH N`.

## Develop

```
cargo test --release --workspace
cargo run --release -p sim --example bench
```

- `docs/design.md`: how the engine works and why.
- `docs/roadmap.md`: build order, benchmark results, known problems.
- `CLAUDE.md`: working notes for AI-assisted development.
