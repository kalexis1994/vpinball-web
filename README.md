# vpinball-web

A port of the [Visual Pinball][vpx] *player* to Rust + WebAssembly + WebGPU
(`wgpu`), with the [PinMAME][pinmame] side of it — the machines' own firmware —
ported alongside.

**▶ [Play it](https://kalexis1994.github.io/vpinball-web/)**

The goal is to run VPX tables in the browser with good performance on modest
devices. The game only: no editor, no VR, no legacy backends.

See [docs/state-of-the-port.md](docs/state-of-the-port.md) for where the port
stands — what is emulated, how it was verified, and what is missing — and
[docs/porting-plan.md](docs/porting-plan.md) for the scope, the analysis of
the original code and the order of work.

[vpx]: https://github.com/vpinball/vpinball
[pinmame]: https://github.com/vpinball/pinmame

## Bring your own table

The deployed page ships **no tables and no ROMs**, and neither does this
repository. A `.vpx` is a hundred megabytes and somebody else's work; a ROM is
copyrighted firmware. What is deployed is the machine that runs them.

Open the page, go to **Content**, and add a `.vpx` by button or by dropping it
in. It is parsed in a worker and kept in your browser's IndexedDB, so it is
there next time. If the table needs a ROM the menu says which one — `f14_l1.zip`
for F-14 Tomcat — and you add the zip the same way.

Nothing is uploaded anywhere. There is no server: the table is parsed, the
machine is emulated and the playfield is drawn in the tab.

## Layout

```
crates/
  vpw-math      Re-export of glam + Visual Pinball units (VPU, VPT)
  vpw-physics   Physics engine (port of src/physics/)
  vpw-render    WebGPU renderer (replaces src/renderer/)
  vpw-table     Reading .vpx: metadata, ROM detection and geometry
  vpw-bus       Memory bus trait, shared by the CPUs
  vpw-m6800     Motorola MC6800/MC6808 CPU (the one in Williams System 11)
  vpw-m6809     Motorola MC6809 CPU (the CS sound board, and the WPC families)
  vpw-arm7      ARM7TDMI CPU (the one on Stern's sound board)
  vpw-at91      Atmel AT91 SoC around that ARM, and the sound board built on it
  vpw-audio     Audio chips: DAC, CVSD HC55516 and YM2151 (FM)
  vpw-pia6821   Motorola MC6821 PIA: all of System 11's I/O
  vpw-s11       Williams System 11 machine: main board and the two sound boards
  vpw-wpc       Williams Pinball Controller: the hardware of the 90s
  vpw-ws        Sega/Stern Whitestar machine: 6809E, the DMD and the AT91 board
  vpw-bench     The sound board with plain C exports, to measure wasm vs native
  vpw-vbscript  VBScript interpreter: the language a table's rules are in
  vpw-game      A table being played: physics, script and the wire between
  vpw-player    The only wasm<->JS boundary: player + vpw-table bindings
web/            Vite + React front end: table menu and game view
```

## The table menu

The menu lists the local library and lets you add `.vpx` files by button or by
drag & drop. Adding a table parses it in a Web Worker (real tables go past
100 MB) and stores it in IndexedDB: the `.vpx` as a Blob, the metadata in its
own store.

From each table we extract the name, the author, the version, the counts of
items/images/sounds, the embedded screenshot if it has one, and **whether it
needs a PinMAME ROM and which one** — for instance `f14_l1.zip` for F-14
Tomcat. That fact is not in the VPX format: it comes from parsing the table's
VBScript. See [`crates/vpw-table/src/rom.rs`](crates/vpw-table/src/rom.rs).

To inspect a table from the terminal:

```bash
cargo run -p vpw-table --example dump -- table.vpx
cargo run -p vpw-table --example script -- table.vpx cGameName
```

## Loading and drawing a table

`vpw-table` pulls the meshes, the materials and the textures out of the `.vpx`,
and `vpw-render` uploads them to the GPU and draws them. To see it without a
browser there is a headless render that writes a PNG:

```bash
cargo run --release -p vpw-render --example shot -- table.vpx output.png 720 1280
```

It also prints the scene counts:

```
meshes         147
vertices       80833
triangles      96829
textures       22
draw calls     26 (one per mesh would be 147, 5.7x fewer)
```

Two environment variables help work out who is covering whom when something
does not show up: `VPW_BATCHES=1` lists the batches with their material and
their texture, `VPW_ONLY=x` draws only those matching `x` and `VPW_EXCEPT=x`
excludes them.

### Where we depart from the original engine

The original keeps one object per part, with its matrix and its draw call,
because the editor can mutate anything at any moment. A player does not need
that: after loading, the table is immutable except for the few parts the script
animates. So:

- **The transform is baked** at load time. There is no model matrix per draw
  and no per-vertex multiplication in the shader.
- **The whole table fits in a single pair of buffers**; each batch is a range
  of indices.
- **Batches are grouped by material and texture**, which is what is expensive
  to change. The original sorts by compatibility rules with old versions —
  `RenderPass.cpp:80` says so in as many words.

That is right for the three thousand parts of a table that stand still and
wrong for the dozen that do not, so those go down **a separate path**: a flipper,
a gate, a spinner, a trigger, a bumper's ring, the plunger and the balls keep
their vertices in a local frame and get one model matrix and one draw call each
(`crates/vpw-render/src/dynamic.rs`). About a dozen draw calls against a table's
twenty-six.

The shader is split accordingly: `material.wgsl` holds everything downstream of
the vertex stage — which is where the lighting lives, and it exists once — and
`table_vs.wgsl` / `dynamic_vs.wgsl` hold one vertex stage each. Forcing a model
matrix onto the static path too would put a matrix multiply on every one of a
table's eighty thousand baked vertices in order to serve about a dozen.

The **lighting, on the other hand, does follow the original**, because it is
what gives tables the look they have: it is `lightLoop` from `Material.fxh`
with its energy conservation, the two scene lights with the ashikhmin/blinn
BRDF, and the environment map with its convolved irradiance.

With one departure: **the general illumination is light**. The original draws
every bulb as a screen-space halo that mostly *modulates* the pixel under it —
its own comment calls that "a very crude approximation of real lighting"
(`light.cpp:830`) — so a playfield the table's own lighting leaves black stays
black under thirty lit bulbs, where the real machine's GI string pours real
light onto the wood and glows in a dark arcade. Here the brightest lit bulbs
are also fed to the material loop as point lights, with the same centre, range
and falloff their halos use, so what the author tuned keeps meaning the same
thing — plus their first bounce off the glass and the plastics, flat and in
their average colour, because direct light ends at each bulb's falloff and a
real machine keeps no black patches. `gi_diffuse` in `material.wgsl` is the
whole of it, and `crates/vpw-render/tests/gi.rs` is the proof.

## Build

Requires the wasm target and a `wasm-bindgen-cli` of the same version as the
workspace's `wasm-bindgen` dependency.

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127 --locked
```

Build the wasm and bring up the front end:

```bash
./build.sh                 # or ./build.sh debug
npm --prefix web install
npm --prefix web run dev
```

Open <http://localhost:8091>. WebGPU is used where the browser grants it
(Chrome/Edge 113+, Safari 18+, Firefox 141+), and **WebGL2 where it does not**:
the renderer was built inside WebGL2's limits on purpose, and wgpu's GL backend
is the same room through an older door. That includes the door with no lock on
it — WebGPU requires a secure context and plain HTTP over the LAN is not one,
but WebGL2 predates the requirement, so a phone pointed at the dev server's
LAN address just works. `?gpu=gl` on the URL forces the GL backend, which is
how that path gets exercised on a desktop that would never take it; the
console names the door that was opened either way.

On entering a table the console prints fps and physics ticks per second.

## Playing

| key | what it does |
|---|---|
| `Z` | left flipper |
| `M` | right flipper |
| space | **held**, pulls the plunger; on release, it shoots |
| `←` `↑` `→` | nudge the cabinet |
| `Enter` | new ball, and clears the tilt |
| `C` | switch the camera: in front of the machine, or straight down |
| `B` | mark: saves the last 30 s of telemetry |
| `Esc` | back to the menu |

The physics runs at a fixed 1000 Hz, decoupled from the frame rate, exactly as
in the original. The HUD shows both numbers: a physics rate below 1000 means the
simulation is falling behind and the table is running in slow motion, which
looks like lag but is not.

## Where the player runs

The player — the wasm module, the game loop, the renderer and the audio
production — runs in a **Web Worker**, drawing on an `OffscreenCanvas`, so the
simulation does not compete with React, garbage collection and the browser for
the main thread, and parsing a 100 MB table freezes nothing. The page keeps
what only a page can do: the DOM, the input events, the `AudioContext` a
browser will only start from a user gesture, and IndexedDB. The samples flow
worker → audio thread on a `MessageChannel` of their own, so a busy page
cannot starve the sound either.

Not every browser grants a GPU inside a worker, so the choice is made per
browser, on evidence: the page asks a fresh worker to actually obtain a WebGPU
adapter — or, failing that, a real WebGL2 context — before transferring the
canvas, because a transfer cannot be taken back; and it falls back to running
the whole player on the main thread, which stays a first-class path.
`?host=main` on the URL forces the fallback, which is how the path only some
browsers take gets exercised on one that does not need it.

The table's own script runs: its parts answer to their names, the physics'
events reach its handlers, and its timers tick on table time. And the machine
underneath it runs too — the ROM boots, takes a coin, serves a ball and keeps
score, with its lamps, its solenoids and its sound. What is still missing is in
[docs/what-is-missing.md](docs/what-is-missing.md).

## Telemetry

Some bugs only show up while somebody is playing, and by the time they are
noticed the interesting part is over. So the player keeps a rolling half hour
of what the machine did and throws the rest away; pressing `B` marks the moment
and takes the thirty seconds before it out of the record.

What comes out is one JSON file, delivered twice: the browser downloads it, and
the dev server writes the same bytes to `web/debug-assets/telemetry/` so that
whoever is debugging can just read it. It holds two tracks, because there are
two kinds of fact. Motion — where each ball was and how fast, where the
flippers were pointing — is sampled every 20 ms, which is about one ball radius
at the speed of a hard shot. Everything else is an **edge**: a coil going live,
a switch closing, a sound the script asked for, a ball appearing or draining.
Those are timestamped to the millisecond, because a coil pulse is thirty of
them and sampling would miss it.

Half an hour of the continuous track is about eleven megabytes, which is what
it costs to have the recorder already running when the thing you are chasing
happens. See [`crates/vpw-game/src/telemetry.rs`](crates/vpw-game/src/telemetry.rs).

Nudging is modelled on `physics/cabinet/`: a 25 ms impulse, a 113 kg cabinet as
a damped oscillator, and a plumb bob with a ring around it. Three warnings and
the fourth one ends the ball.

## The script engine

Every VPX table is a VBScript program, and `vpw-vbscript` is an interpreter for
it — written here rather than borrowed, for the reasons in
[docs/porting-plan.md](docs/porting-plan.md) §3.1. The short version: a table's
script crosses into the table's objects on nearly every line, and the
alternatives put a C bridge or the JS↔wasm boundary on that path.

It is checked against the real thing. The tests parse all seventy scripts
Visual Pinball ships, `core.vbs` included, and two of them load and run
published tables end to end:

```bash
# Parse the script inside a table
cargo run --release -p vpw-vbscript --example parse_table -- table.vpx

# And run it, with the libraries it depends on
cargo run --release -p vpw-vbscript --example run_table -- table.vpx ../vpinball/scripts/core.vbs
```

The bugs that found were ones no invented test would have: a table whose script
is stored with bare carriage returns as line endings, and `SetLamp (118), 0` — a
call whose first argument is parenthesised, which reads as an array index if you
are not careful.

## Running a table's rules

`vpw-game` is the wire between the three halves: it answers the script's names
with the table's real parts, turns the physics' events into calls to
`Bumper1_Hit` and friends, and runs the parts' timers on table time.

The surface it has to cover was **measured, not guessed**: parsing four
published tables and `core.vbs` and counting every `x.Member` gives a bounded
list with a very long tail, and what is implemented is the head of it.

```bash
# Load a table, play it, and say what happened
cargo run --release -p vpw-game --example play -- table.vpx ../vpinball/scripts 10
```

It prints what the table said on start-up, how many script calls ran, and which
members the table asked for that this port does not model — which is the
shopping list for whoever works on the binding next.

Two things a table needs that are not in the `.vpx`:

- **Its libraries.** A table's script pulls in `core.vbs` and its machine's
  library *by name, at run time*, through Visual Pinball's `GetTextFile`. The
  web app bundles Visual Pinball's `scripts/` directory and hands it over
  before loading a table.
- **The player's own globals.** `VPBuildVersion`, the key codes, `Setting`.
  One missing number there is the difference between a table that loads and one
  that reports it cannot find `core.vbs` — `s11.vbs` reads `VPBuildVersion` to
  decide *how to load its own libraries*.

## The System 11 emulator

`vpw-s11` is a complete Williams System 11 machine: three CPUs (the main 6808
at 1 MHz and the two sound boards, another 6808 at 1 MHz and a 6809 at 2 MHz),
six PIAs, the two displays, the switch matrix, the lamp matrix, the solenoids,
the CMOS and the three audio sources (DAC, speech through CVSD and FM synthesis
with the YM2151).

Not every System 11 wires the display the same way: the 11B and 11C send the
segment data **inverted** and have the two alphanumeric rows, so you have to
tell it which one it is with `set_display_config`.

```bash
# Watch a real ROM boot
cargo run --release -p vpw-s11 --example boot -- /path/f14_l1.zip 20

# Boot hundreds of sets at once and see which ones run clean
cargo run --release -p vpw-s11 --example batch --     crates/vpw-s11/data/rom_sets.tsv /path/roms 12

# Sweep the sound numbers of each board
cargo run --release -p vpw-s11 --example sound_test -- /path/f14_l1.zip
cargo run --release -p vpw-s11 --example sound_cs_test -- /path/f14_l1.zip
```

## The WPC emulator

`vpw-wpc` is the platform of the 90s: Twilight Zone, The Addams Family,
Terminator 2 and a hundred more. It shares the CPU with the System 11 sound
board —a 6809— but the resemblance ends there: instead of six PIAs there is an
ASIC that concentrates all the I/O, the ROM is paged in 16 KB banks to reach a
megabyte, and the display is a 128x32 dot matrix.

```bash
# Boot a game and see the dot display in the terminal
cargo run --release -p vpw-wpc --example wpc_boot -- /path/t2_l8.zip 5 dmd

# Boot the 468 sets in the manifest
cargo run --release -p vpw-wpc --example wpc_batch --     crates/vpw-wpc/tests/data/rom_sets.tsv /path/roms 4
```

## The Whitestar emulator

`vpw-ws` is Sega and Stern's platform from 1995 to 2004: a 6809E main board, a
second 6809 driving the 128×32 dot matrix through a CRTC, and — on the Stern
games — a sound board that is a computer of its own: an Atmel AT91 with an
ARM7TDMI at 40 MHz (`vpw-arm7` and `vpw-at91`) that boots a two-megabyte BIOS
and does all its mixing in software. There is no path to a single note of Lord
of the Rings that does not go through emulating that processor.

```bash
# Boot a game and watch for the signs of life: bank switching, the switch
# strobe, the lamp sweep
cargo run --release -p vpw-ws --example ws_boot -- /path/lotr.zip 5

# Boot just the sound board and watch the remap and the first samples
cargo run --release -p vpw-at91 --example at91_boot -- /path/lotr.zip 10

# Ask it for one sound after another and list which commands answer
cargo run --release -p vpw-at91 --example sweep -- /path/lotr.zip
```

## Tests

```bash
cargo test
```

The tests that boot a real ROM are skipped unless they are told where the zip
is (ROMs are not in the repo):

```bash
# One game in depth: displays, sound, POST
VPW_S11_ROM=/path/f14_l1.zip cargo test --release -p vpw-s11

# The 297 sets in the manifest, breadth-first
VPW_ROM_DIR=/path/to/the/roms cargo test --release -p vpw-s11
```

`crates/vpw-s11/data/rom_sets.tsv` lists the System 9, System 11 and Data
East sets with each one's ROM layout and display wiring. It came out of
`s11games.c` and `degames.c` from PinMAME: the three families share the same
memory map, so they run on the same emulator.

To watch a ROM boot by hand:

```bash
cargo run --release -p vpw-s11 --example boot -- /path/f14_l1.zip 20
```

## What this is ported from

Two projects, and this would not exist without either.

- **[Visual Pinball][vpx]** — the player: the physics, the renderer, the table
  format and the script host. Nearly every file here cites the one it came from,
  by name and line, because "it behaves the same" is a claim and a citation is
  evidence. GPL-3.0-or-later.
- **[PinMAME][pinmame]** — the machines: the CPUs, the PIAs, the sound chips,
  the display wiring and the per-game data that says how each one is put
  together. BSD-3-Clause.

Neither is vendored. The citations are line references into their repositories,
and the tests that check against them expect a checkout next door.

## Licence

GPL-3.0-or-later, inherited from Visual Pinball. The parts ported from PinMAME
are BSD-3-Clause, which that permits.

The one third-party asset is the display typeface the menus are set in:
**Orbitron** by The League of Moveable Type, SIL Open Font License 1.1, bundled
as a 12 KB variable font at `web/src/fonts/` with its licence beside it. It is
carried rather than fetched so the page keeps working with no signal.
