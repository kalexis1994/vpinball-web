# vpinball-web

### ▶ **[Play it now — kalexis1994.github.io/vpinball-web](https://kalexis1994.github.io/vpinball-web/)**

Real pinball tables, running in your browser. No install, no account, no
server: the machine is emulated and the playfield is drawn in the tab you
already have open.

---

## About

A pinball machine from the eighties is two things at once — a physical table
with a steel ball loose on it, and a computer with the rules of the game
burnt into a chip. This runs both. The physics of
[Visual Pinball][vpx] and the firmware emulation of [PinMAME][pinmame] are
ported to Rust and compiled to WebAssembly, drawn with WebGPU, so a table
authored for a desktop simulator plays on a phone through a URL.

**What you get**

- **The whole machine.** Not a video game *about* pinball: the table's own
  script runs, the game's original ROM runs on an emulated CPU, and the
  score, the lamps, the sounds and the rules are the machine's own.
- **Your tables.** Bring any `.vpx` file and its ROM. They stay in your
  browser and nothing is ever uploaded. A table that ships no picture of
  itself gets photographed the first time you play it, so the shelf stays
  readable.
- **On a phone, properly.** Touch flippers, a plunger you drag, an overhead
  view where the screen is the glass over the playfield, and a renderer that
  trades quality for frames on its own when a device needs it.
- **A head that looks like one.** A `.vpx` stops at the playfield — Visual
  Pinball draws the backglass from a separate file, or on a second monitor —
  so the head is painted here, from the colours of the table's own artwork.
  F-14 comes out red and blue and South Park blue and yellow, and neither is
  named anywhere.
- **Offline.** Once opened, the player is cached; the tables already live in
  your browser. It works on a plane.

**What you need**

A reasonably current browser — Chrome, Edge, Firefox or Safari — on a
desktop, a tablet or a phone. WebGPU is used where it exists and WebGL2 where
it does not.

## Bring your own table

The deployed page ships **no tables and no ROMs**, and neither does this
repository. A `.vpx` is a hundred megabytes and somebody else's work; a ROM is
copyrighted firmware. What is deployed is the machine that runs them.

Open the page, go to **Content**, and add a `.vpx` by button or by dropping it
in. It is parsed in a worker and kept in your browser's IndexedDB, so it is
there next time. If the table needs a ROM the menu says which one — `f14_l1.zip`
for F-14 Tomcat — and you add the zip the same way. A zip with several tables
and their ROMs in it can be dropped whole; you are shown what was found before
anything is kept.

Nothing is uploaded anywhere. There is no server: the table is parsed, the
machine is emulated and the playfield is drawn in the tab.

## Playing it

| key | what it does |
|-----|--------------|
| `Z` / `M` | left and right flipper |
| space | hold to pull the plunger, let go to shoot |
| ← ↑ → | nudge the cabinet — too hard and it tilts |
| `5`, then `1` | drop a coin in, then start the game |
| `Enter` | serve a new ball, and clear the tilt |
| `C` | switch between the front view and the overhead one |
| `Esc` | pause, with resume and quit |

Every one of those can be moved, and a **gamepad** works too — the shoulders
are the flippers, because that is where a cabinet keeps them.

On a phone the same controls are on screen: the flippers are the two buttons
at the bottom corners, the plunger is dragged down and released, and the coin
and start buttons are where a cabinet keeps them.

## Settings

Four tabs, so that looking for the flipper keys does not mean scrolling past
the room lighting.

**Controls** — the keyboard and the gamepad, in rooms of their own. Each has
its own bindings and its own defaults, because they are different instruments
and moving the left flipper on the pad says nothing about where it is on the
keyboard. Press a row, then press the key or the button you want on it.

**Graphics** — where a slow machine is made fast:

- **Renderer** — *Full 3D* is the real thing, lit and reflected. *Flat (2D)*
  photographs the table once and plays the photographs, keeping only the ball
  and the moving parts in 3D; it looks close and costs a fraction, which is
  what makes an older phone play at full speed.
- **Adaptive resolution** — on, the picture softens when the device falls
  behind and sharpens when it catches up. Off, it stays sharp whatever the
  frame rate does.
- **Room** — the light the table stands in: its own, or a real bar in HDR
  whose lamps show up reflected in the ball and the plastics.
- **Camera** — where you look from.

**Score panel** — where the machine's display sits when its head is not in
shot: which gutter it stands in on a wide screen, and whether it docks above
or below the table on a phone, or stays out of the way.

**Audio** — the master volume, and under it the balance between the two
halves of a machine's noise: what the game *says* — its sound board, the
music and the speech and the effects — against what the table *does* when it
is hit, the bumpers and the flippers and the ball on the wood. Both go past
100%, because plenty of ROMs were mastered quietly and a table cannot be
balanced against one of those by turning the table down.

## For developers

A port of the Visual Pinball *player* to Rust + WebAssembly + WebGPU
(`wgpu`), with the PinMAME side of it — the machines' own firmware — ported
alongside. The goal is to run VPX tables in the browser with good performance
on modest devices. The game only: no editor, no VR, no legacy backends.

Nearly every file cites the one it was ported from, by name and line, because
"it behaves the same" is a claim and a citation is evidence.

See [docs/state-of-the-port.md](docs/state-of-the-port.md) for where the port
stands — what is emulated, how it was verified, and what is missing — and
[docs/porting-plan.md](docs/porting-plan.md) for the scope, the analysis of
the original code and the order of work.

[vpx]: https://github.com/vpinball/vpinball
[pinmame]: https://github.com/vpinball/pinmame

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

### The flat engine

A phone that cannot afford the scene pass can afford a photograph of it. With
*Flat (2D)* on, the table is rendered once into a set of images and then
played as images, with only the ball and the dozen moving parts still drawn in
3D on top.

The trick is that light is additive, so the photographs can be relit: one base
picture with every lamp off, plus one difference picture per lamp, and the
frame is `base + Σ level × layer`. A lamp at 40% contributes 40% of its own
picture, exactly as it would have, and the script can do whatever it likes to
the lamps without a single triangle being drawn again. The per-lamp pictures
are cropped to where that lamp actually reaches and packed into an atlas,
because a bulb over the left slingshot changes nothing on the right.

The bake is spread over a few frames rather than blocking, so the table is
playable while it is being taken. The camera is fixed while it is on: the
photographs were taken from one place, and there is no second place to move
to.

`crates/vpw-render/src/flat.rs`.

### The head, which is not in the file

A `.vpx` stops at the playfield. Visual Pinball draws the backglass from a
separate `.directb2s`, or on a second monitor, and neither is part of the
table — so the head is **built** here from the cabinet's proportions
(`crates/vpw-table/src/backbox.rs`), and the artwork on it is **painted**
(`crates/vpw-table/src/backglass.rs`).

The palette comes from the table itself, out of the playfield texture, with
each pixel weighted by `saturation² × value`. That weighting is the whole
trick: the commonest colour in a playfield photograph is a muddy mid-grey —
wood, shadow, the average of everything — so a palette built by count comes
back as four greys, and what a machine is remembered by is its vivid colours
however little of it they cover.

Then domain-warped value noise for the cloud, with the colour tied to the
cloud's own density along a single ramp rather than chosen by a field of its
own: when hue and brightness are independent, two colours meet at equal
brightness and the join is a hard edge, which reads as marbled oil rather than
as depth. Then a few broad Gaussians for the tubes behind the sheet — added
where they overpower the ink, so the hot spot washes towards the tube's colour
instead of the artwork's, which is what says *lit from behind* rather than
*painted bright*. Then a grey frame, because the glass in a real head is held
by something.

To look at one without a browser:

```bash
cargo run --release -p vpw-table --example backglass -- out.png table.vpx
```

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

## The keys in full

The player's own keys are in [Playing it](#playing-it) and every one of them
can be rebound; these are the rest, which exist for working on it rather than
for playing, and which cannot.

| key | what it does |
|---|---|
| `B` | mark: saves the last 30 s of telemetry |

The physics runs at a fixed 1000 Hz, decoupled from the frame rate, exactly as
in the original. The HUD shows both numbers when either is worth reading: a
physics rate below 1000 means the simulation is falling behind and the table is
running in slow motion, which looks like lag but is not, and a `Q` beside them
is the rung the quality ladder has dropped to.

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
