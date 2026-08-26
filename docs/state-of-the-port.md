# State of the port

_26 August 2026. A snapshot of what is emulated, how faithfully, how it was
checked, and what is known to be missing. `what-is-missing.md` is the running
backlog; `parity.md` is the file-by-file ledger against the original. This is
the view from above both._

## What it is

A player for Visual Pinball tables, in Rust, compiled to WebAssembly and
drawn with WebGPU. A player only: no editor. A table is a `.vpx` the player
brings, a machine is a PinMAME ROM zip the player brings, and the port's job
is to be the thing that runs them — the physics, the script interpreter, the
renderer, and the emulated hardware that runs the game's own firmware.

Fifty-one thousand lines of source across eighteen crates, twenty thousand
lines of tests (1,110 tests in 88 suites), and four thousand lines of
TypeScript for the page. Thirty-two commits in this stretch.

| crate | what | src | tests |
|---|---|---|---|
| `vpw-vbscript` | a VBScript interpreter, from scratch | 7,851 | 1,220 |
| `vpw-table` | the `.vpx`: geometry, lights, materials, physics setup | 8,110 | 3,167 |
| `vpw-physics` | the ball, the collisions, the moving parts | 6,938 | 5,082 |
| `vpw-game` | binds script, physics and machine into a table that plays | 6,004 | 2,798 |
| `vpw-render` | WebGPU renderer, shaders, bloom, lights | 5,688 | 1,260 |
| `vpw-s11` | Williams System 11 board | 4,363 | 1,885 |
| `vpw-audio` | mixer, sound-board filters, the table's own sounds | 2,844 | 763 |
| `vpw-ws` | Sega/Stern Whitestar board and its DMD | 1,792 | 916 |
| `vpw-m6809`, `vpw-m6800` | the 8-bit CPUs the boards run on | 2,804 | 1,367 |
| `vpw-arm7`, `vpw-at91` | the ARM7TDMI and the AT91 SoC of the Stern sound board | 1,936 | 634 |
| `vpw-player` | the wasm surface the page talks to | 1,583 | — |
| `vpw-pia6821`, `vpw-wpc`, `vpw-bus`, `vpw-math`, `vpw-bench` | peripherals and glue | 1,454 | 1,071 |

## What is emulated

**Three generations of machine, four processors, from scratch.**

- **Williams System 11** (F-14 Tomcat is the reference table): 6808 CPU,
  PIAs, the segment displays, and the sound boards — a 6809 with a YM2151 and
  a 6808 with a DAC and speech. Boots from a blank CMOS, writes its own
  factory defaults, plays.
- **Sega/Stern Whitestar** (Lord of the Rings): 6809E main board, a second
  6809 driving the 128×32 DMD through a CRTC, and — the big one — the
  **Stern sound board**, an Atmel AT91 with an ARM7TDMI at 40 MHz that does
  all its mixing in software. There is no path to a single note of these
  games that does not go through emulating that processor. It boots the real
  two-megabyte BIOS, builds its program in RAM, remaps, and runs its sample
  interrupt at 24,242 Hz — the rate its own timer is programmed to, not one
  this assumed. Eighty-six of ninety-one sound commands produce audio.
- The game boards run the **actual firmware** from the ROM zips. Nothing
  about the rules of a game is reimplemented: the ROM decides what a switch
  means, what a lamp does, when a coil fires, and the port hands it switches
  and takes back lamps, coils, display and sound — exactly the contract a
  table's script expects from VPinMAME.

**The table side, ported from Visual Pinball.**

- The physics engine: the ball, its spin and friction, walls, rubbers,
  ramps, flippers, bumpers, slingshots, kickers, gates, spinners, triggers,
  the plunger, and the nudge/tilt model. Sixty-eight of the original's
  collision routines are ported one for one; `parity.md` tracks each.
- The VBScript interpreter is complete enough that real tables' scripts run
  unmodified, including `core.vbs` and the machine-family libraries. It was
  the item the plan called the main blocker, and it was.
- The renderer draws the original's material model — its Ashikhmin-Blinn
  BRDF, the environment map with roughness-selected mips, the bulb-light
  blend that multiplies rather than paints, Reinhard tone mapping, bloom,
  playfield reflection, transmitted light — in WGSL.
- The page: table library in IndexedDB, ROM import, a PWA that works
  offline, touch controls that are the cabinet's buttons, a headless
  screenshot and a headless table runner for everything that cannot be
  judged by eye. The player itself runs in a worker over an
  `OffscreenCanvas` where the browser grants a GPU there — probed by
  obtaining a real adapter or a real WebGL2 context, because a transferred
  canvas cannot be taken back — with the main-thread path kept whole as the
  fallback; the renderer takes WebGPU or WebGL2 by the same evidence, which
  is what lets a phone on plain-HTTP LAN play without a secure context.
  `?host=main` and `?gpu=gl` force the paths a capable browser would never
  take.

## How faithful, and how that was found out

The port cites file and line of the original at every non-obvious decision
and marks every deliberate departure. That convention turned out to be the
whole method, because **every serious bug of the last week was a place
where the original was not consulted**:

| what was wrong | where the answer was | what it looked like |
|---|---|---|
| sample ROMs mapped as four blocks | `desound.c:453` — two interleaved pairs, A0 and A21 select | music with noise all over it |
| lamp halo scaled by 0.02 | `light.cpp:798` — that factor is the bulb mesh's, not the halo's | lit inserts pixel-identical to unlit |
| bloom pinned at 1.8 | the table's own field; this one asks for 0.3 | every insert a blown-out blob |
| material colours gamma-decoded | `utils/color.h:22` — a plain divide by 255 | playfield 30% dark |
| kicker tested with a wall's flags | `collide.cpp:209` — "all false = kicker/trigger" | invisible post in the outlane |
| gravity 0.97 passed as an acceleration | `pintable.cpp:3833` — it is a multiple of `GRAVITYCONST` | every table 1.8× too floaty |
| switch numbers clipped to 1–64 | `core.c:2108` — `n + 7`, and the flippers are 81–84 | no flippers on a Whitestar |
| wire ramp channel = wire spacing | `ramp.cpp:471` — spacing **plus twenty** | the ball wedged in every habitrail |
| interrupt return address +8 | ARM ARM: the instruction plus four | firmware walked off its idle loop |

None of these crashed. Every one produced a plausible wrong result —
silence, darkness, a table that "lacks life", a ball that "gets stuck over
there" — and the only thing that found them was a **differential audit**:
agents reading the port against the reference side by side, citing both,
refusing plausibility. Five of them in parallel found more in twenty
minutes than three days of debugging from symptoms. That is now the way
this work is done, and the remaining findings of that audit are the
backlog below.

The second thing that made the difference was **being able to look without
a browser**. `vpw-render --example shot` photographs a table; `vpw-game
--example table` runs a real table with its ROM, presses its buttons, drops
a ball anywhere, dumps its display, names the shapes a stuck ball is
resting against, and writes what it hears to a WAV. Numbers about lamps
had agreed with each other for a day while the picture stayed dark; the
picture settled it in an hour.

The third is measurement discipline on this device: its clock moves by up
to 8× with the governor, so every timing is taken against a control
workload in the same window, and the speed test asserts a ratio, not a
time.

## Performance

- Whitestar with all three boards: **~4× real time** natively on this phone.
  The sound board alone is **2.96× in wasm against 1.92× native** on the
  same machine — the browser is not the bottleneck, and both produce the
  same 96,970 samples down to the last one.
- The AT91 costs seven times the rest of the machine put together, down
  from seventeen: the timers are left alone until one of them could do
  something, and the interrupt controller walks only the sources that are
  asking.
- Nothing accumulates. RSS is flat over twenty minutes of play and the cost
  at minute twenty equals minute zero; the perceived slowdown that started
  this stretch was the phone's governor.

## Known gaps

From the audit, in order of what a player notices. Items marked ✓ have
landed since; the rest are open.

**Machine**
- ✓ coin-door and flipper switches, fast flips, coil pulse accumulation,
  memory protect, sound-board status lines
- ✓ the DMD's 15 Hz PWM integration (`dedmd.c:225`): the two-bit frame goes
  through the eye's low-pass now instead of being shown raw
- ✓ the AT91's `PS_PCER`/`PS_PCDR` decode (it was one register down, so an
  enable was a disable), the word store to the sample port (one sample, not
  two), and the `TC_IMR`/`PIO_OSR` read offsets — each read against
  `at91.c`/`desound.c`, and the bus decode has its own test file now
- other Whitestar expander boards (servo, magnets, mini-DMDs, Titanic)

**Sound level.** The music of a Whitestar game is a setting of the machine,
not of the mixer: the mixer gain is 1.0 and PinMAME mixes at 100%. Pressing
the machine's own coin-door buttons headlessly — red opens the adjustment,
green raises, red lowers — moves the music from a factory 297 rms to 435
after four presses, 825 after eight, 2,237 after sixteen, and it saturates
at about 3,590 — 11% of full scale, 45% at the peaks — from twenty-two
presses on, twelve times the factory level. Those two buttons are on the
glass now and the level persists in the machine's battery-backed memory
with the high scores. If the machine at its own ceiling still sits under
the table's mechanical sounds, the honest knob is a player-side gain on the
board's stream, kept apart from the machine's setting — an amplifier, which
is what a cabinet has and this does not.

**Table**
- ✓ slope from the difficulty, gravity, day/night scale, bloom, flipper
  rubber, wall normals, bumper cap
- ✓ the **insert image** (`IMG1`): a lit insert is its artwork, lit
- ✓ **flashers**: the polygon, both pictures and the four filters, the
  painted and the additive blend, every script member, and the DMD mode
  that a 10.8 table places its display with. Not yet: the `Display`,
  `AlphaSeg` and external modes (plugin displays), ball shadows on a
  flasher, and a lightmap flasher snaps with its lamp's switch rather than
  following its fade
- ✓ the table's own **environment map** (`EIMG`), with the bundled one kept
  as the fallback
- ✓ the **general illumination illuminates**: the brightest lit bulbs reach
  the material loop as point lights, and their first bounce fills the corners
  no bulb reaches — a deliberate departure, because the original's halos
  *modulate* what is under them and modulating a black playfield produces
  black, where a real machine's GI string lights the wood and its glass and
  plastics scatter the rest
- ✓ and the strings' share of it is **baked**: the GI lamps are grouped by
  what switches together (colour-clustered — F-14's warm, red and blue
  strings come out as their own layers), traced once per table against the
  table's own meshes on the CPU — direct with shadows plus one diffuse
  bounce, which is light turning a corner and a wall's colour on the wood
  beside it (`vpw-render/src/bake.rs`, ~5 s for F-14's four groups) — and
  each layer is scaled by its group's **live level**, so the light show that
  flashes red against blue flashes the maps. In the browser the trace runs
  in a worker of its own after the table loads and the result is kept in
  IndexedDB, so a table pays for its bake exactly once. A table that ships
  its own lightmap flashers gets none of this — the whole departure switches
  off, because its author already did the light transport. The grouping is
  the **machine's own answer**: the bake worker boots the game headless,
  runs half a minute of attract, and groups the lamps whose switching
  histories came out identical — on F-14 that put the whole GI on one relay,
  which is what a System 11 wires, where guessing by colour had invented
  three independent strings. Names and colours remain the fallback for a
  table with no ROM. And the halo of every bulb is **capped** (`fs_bulb`):
  the authors tuned the modulate blend against the original's darker field,
  and over the field the departure lights, a slingshot lamp at eighty was a
  flash grenade — the soft cap leaves small halos as tuned and walks the
  huge ones down to a bulb's worth of glare, while a baked lamp's halo also
  shrinks to hug its bulb, its field-lighting job now done by the map
- ✓ a player-side **day/night** — the original's user-mode override
  (`Renderer.cpp:377`), because plenty of tables are authored dark on purpose
  (F-14 asks for 0.08) — and the head's score display drawn **emissive**: a
  plasma panel is a light, not a lit thing, and through the light loop it
  vanished into the dark room the table asked for
- tone-mapper selection and the colour-grade LUT
- decals, text boxes, display reels, light sequences, editor-placed balls,
  built-in-shape primitives (posts and pegs with no mesh are invisible
  *and* have no collider)
- per-primitive colour/alpha/additive blend, normal maps, depth bias,
  backface culling, part-group visibility masks
- kicker `Unhit` timing and per-shape hit arming on non-legacy drop targets
- ball image, playfield reflection strength on the ball, AO, SSR

**Interpreter**: whatever the next table needs. Each real table so far has
found a construct — `Not x Is Nothing`, a space before a member dot,
`Me` inside a class versus inside a handler — and each is a small fix once
seen.

## Risks worth naming

- **Fidelity by memory.** Twice in one day a mapping was reconstructed from
  recollection instead of read, both times wrong in a way that still made
  music. The rule now is that nothing about the original is written down
  without the file open.
- **Tests that encode the bug.** The 0.02 halo factor had a test asserting
  it, with a real measurement — taken with the bloom six times too strong.
  Two bugs each made the other look like the fix. A test is only as good as
  the reference it was checked against.
- **What cannot be seen.** Every bug above was silent. The tooling for
  looking — screenshots, headless runs, WAV capture, A/B against a control —
  is not overhead; it is where the answers came from.
- **One machine, one table each.** F-14 and Lord of the Rings are the only
  two tables that have been run end to end. The third will find things.

## Next

1. A third table, of a third family, to find what two have not. The three
   items that used to stand ahead of it — the insert image, the table's own
   environment map, the DMD's PWM filter — have landed, and the way to find
   the next three is to play a table that has not been played.
