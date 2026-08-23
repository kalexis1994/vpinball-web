# Porting plan: Visual Pinball → Rust + wasm + WebGPU

A living document. It reflects the analysis of the C++ tree in `../vpinball`
(commit `2920186`, 2026-08-16).

## 1. Scope

**In:** the *player*. Load a `.vpx`, simulate the physics, run the table's
script, render and respond to input.

**Out:** the editor, the desktop UI, VR, anaglyph/stereo 3D, hardware
backglass, DOF/solenoids, and every render backend that is not WebGPU.

## 2. Surface area of the original code

Line count (`.cpp` + `.h`) by area:

| Area           |    LOC | What we do                                                           |
|----------------|-------:|----------------------------------------------------------------------|
| `src/ui`       | 47,104 | **Discard.** It is the editor + ImGui + Win32 dialogs.               |
| `src/parts`    | 40,340 | Port ~50%. The other half is the editor's property sheets.           |
| `src/renderer` | 38,224 | Rewrite, do not port. Almost all of it is D3D/GL/bgfx plumbing + VR. |
| `src/meshes`   | 25,134 | Mechanical conversion: they are hardcoded vertex arrays.             |
| `src/core`     | 15,314 | Port the player; discard editor/undo/COM.                            |
| `src/physics`  | 11,361 | **Port it whole.** It is the heart.                                  |
| `src/utils`    |  5,909 | Port what is needed.                                                 |
| `src/input`    |  5,457 | Rewrite against DOM events + the Gamepad API.                        |
| `src/math`     |  3,393 | Replace with `glam` + VP unit conversions.                           |
| `src/audio`    |  1,871 | Rewrite against WebAudio.                                            |

Discarding the UI takes out almost a third of the tree. The real core that has
to be reproduced comes to about **50–60k LOC**.

## 3. The four hard pieces

### 3.1 VBScript — the main blocker

Every VPX table is, functionally, a VBScript program. Without a script engine
there is no game: no bumpers that score, no lights, no modes.

The original uses `IActiveScript` (Windows COM). For the standalone builds they
**did not reimplement it**: they link `libwinevbs`, that is, **Wine**'s
VBScript engine packaged as a library (see `CMakeLists.txt:122-127, 420-431`).

Options, from lower to higher risk:

1. **Compile `libwinevbs` to wasm with Emscripten** and interoperate from Rust.
   It reuses an engine already compatible with real tables. The cost is that
   Wine code has to be made to build for wasm and that the Rust↔C bridge is
   crossed on every script call into a table object (and there are an awful lot
   of those per frame).
2. **Our own VBScript interpreter in Rust.** Tables use a bounded subset (subs,
   functions, arrays, `With`, simple classes, the occasional `Eval`). More work
   up front, but total control over the binding to the table objects and over
   performance.
3. **Transpile VBScript → JS** and run it in the browser's engine. The JIT is
   free and very fast, but every access to a table object crosses the JS↔wasm
   boundary, which is exactly the hot path.

> **Decision pending.** It is the one that constrains the architecture most.
> Better settled early by measuring with a real table, not in the abstract.

### 3.2 PinMAME

Tables with a ROM emulate the original hardware via PinMAME
([vpinball/pinmame](https://github.com/vpinball/pinmame), cloned in
`../pinmame`). It is a fork of MAME: **596,080 LOC** of C. Porting it whole to
Rust is not one project, it is several.

But it does not have to be ported whole. PinMAME covers ~20 CPU architectures
and 131k LOC of drivers alone, and each table uses **one** hardware generation.
For F-14 Tomcat (Williams **System 11A**, `s11games.c:407`) the subset is:

| Piece                 | File                    |   LOC | Licence      |
|-----------------------|-------------------------|------:|--------------|
| 6808 CPU              | `src/cpu/m6800/`        | 5,916 | old MAME     |
| System 11 driver      | `src/wpc/s11.c` + `.h`  | 1,717 | BSD-3-Clause |
| Game definitions      | `src/wpc/s11games.c`    | 1,649 | BSD-3-Clause |
| Pinball core          | `src/wpc/core.c` + `.h` | 4,717 | old MAME     |
| Williams sound        | `src/wpc/wmssnd.c`+`.h` | 4,649 | old MAME     |
| PIA 6821              | `src/machine/6821pia.*` | 1,234 | BSD-3-Clause |
| DAC                   | `src/sound/dac.c`       |   195 | —            |
| CVSD (speech) HC55516 | `src/sound/hc55516.c`   | 1,404 | BSD-3-Clause |

Total: **~21,500 LOC**, and `core.c`/`wmssnd.c` cover many generations, so what
applies to System 11 alone is quite a bit less. That much really is portable.

> **Watch out for the licence.** PinMAME is migrating to BSD-3-Clause but the
> migration is not finished: the files that already migrated carry
> `// license:BSD-3-Clause` on the first line, the rest are still under the old
> MAME licence, which is **non-commercial** and therefore incompatible with
> this project's GPLv3. It is precisely the three largest files we need
> (`m6800.c`, `core.c`, `wmssnd.c`) that are still old MAME. Those have to be
> reimplemented from the hardware specification (the 6800 instruction set is
> public), not transliterated from the C.

### 3.3 The `.vpx` format

A `.vpx` is an OLE Compound File with BIFF records inside.

**This is already solved in Rust:** francisdb's
[`vpin`](https://github.com/francisdb/vpin) crate reads and writes VPX, and
even publishes a wasm build (`@francisdb/vpin-wasm`). We start there instead of
porting `pintable.cpp` and `PoleStorage.cpp` by hand.

### 3.4 Assets and shaders

`src/shaders` weighs 109 MB and `src/assets` 42 MB. None of that goes into a
web bundle as it is. The shaders have to be rewritten in WGSL (they are
HLSL/GLSL) and an asset pipeline with lazy loading has to be built.

## 4. Why this can come out faster than the original

The reason for the port is performance on weak devices. Where the real gains
are:

- **Physics at 1000 Hz.** `PHYSICS_STEPTIME` is 1 ms. On a phone that is ~17
  steps per frame at 60 fps, all single-threaded. The broadphase
  (`quadtree.cpp`, `kdtree.cpp`, `AsyncDynamicQuadTree.cpp`) is a direct
  candidate for SIMD (`wasm32` has SIMD128) and, for the narrowphase, for
  compute shaders.
- **Draw calls.** The original renderer drags 25 years of compatibility behind
  it and changes state per object. With WebGPU the frame can be built with
  stable bind groups and aggressive instancing.
- **Discarding the editor.** A great many of the original's structures exist so
  that the editor can mutate the table live. In a player, tables are immutable
  after loading: we can precompute and flatten everything at load time.
- **An explicit budget.** We measure from the first commit (see the fps/ticks
  counter in `vpw-player`) instead of chasing regressions afterwards.

## 5. Order of work

| Phase | Goal | Status |
|------|----------|--------|
| 0 | Scaffold: workspace, wasm boot, WebGPU initialised, fixed-step loop | **done** |
| 1a | Read `.vpx` with `vpin`: metadata, ROM detection, library in the menu | **done** |
| 1b | Dump the table's geometry and game items | **done**: primitives, walls, rubbers, ramps, flippers, bumpers, targets, gates, spinners, kickers and lights |
| 2 | Static render of the playfield: meshes, materials, textures, camera | **done** |
| 3 | Physics: ball, walls, playfield, gravity. No script. | **done** |
| 4 | Full physics: flippers, bumpers, slingshots, kickers, triggers, gates, spinners | **partial**: the wall and rubber geometry of a `.vpx` already stops the ball; the moving parts are missing |
| 5 | Script engine + binding of the table's objects (see 3.1) | pending |
| 6 | Input (keyboard, touch, gamepad) and audio (WebAudio) | pending |
| 7 | Lights, reflections, ball trails, DMD | **partial**: the halos of the lit lights; reflections, trails and DMD are missing |
| 8a | MC6800/6808 CPU in Rust (`vpw-m6800`) | **done** |
| 8b | PIA 6821 (`vpw-pia6821`) | **done** |
| 8c | System 11 memory map, the 6 PIAs and the periodic IRQ (`vpw-s11`) | **done** |
| 8d | 14- and 7-segment alphanumeric displays | **done** |
| 8e | I/O: switches, lamps, solenoids, CMOS | **done** |
| 8f | M6809 CPU (`vpw-m6809`) | **done** |
| 8g | Audio chips: DAC + CVSD HC55516 (`vpw-audio`) | **done** |
| 8h | XS sound board: 6808, banked ROM, DAC and speech | **done** |
| 8i | YM2151, the FM synthesiser (no LFO yet) | **done** |
| 8j | CS sound board: 6809 + YM2151 + DAC + CVSD | **done** |
| 8k | Integration: the three CPUs and the mixed audio | **done** |
| 8l | LFO and noise of the YM2151: the complete chip | **done** |
| 8m | Switch input and the return channel of the CS board | **done** |
| 8n | WPC board: 6809, ASIC, paged ROM and DMD (`vpw-wpc`) | **done** |

Phases 3 and 4 are the ones that benefit most from regression tests against the
original engine: it is deterministic code, so a ball's trajectory can be
compared tick by tick.

## 6. Licence notes

Visual Pinball is **GPLv3+**. Any code derived from `../vpinball` — including
the line-by-line ports of the physics — inherits that licence. The workspace
already declares `GPL-3.0-or-later`.
