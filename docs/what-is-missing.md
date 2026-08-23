# What is missing before it plays well

State as of 23 August 2026. The table can be played and its own rules run: the
physics runs in the browser, everything that moves is drawn moving, nudging
costs you something, the table's VBScript is loaded and driving its parts, and
the machine underneath it boots, takes a coin, serves a ball and keeps score
with its lamps, its solenoids and its sound.

The order below is not by difficulty but by **how much closer it gets us to
playing**. Each item says what there is today, what is missing and how you know
it came out right.

---

## Where we are

What already works, measured on F-14 Tomcat:

| | |
|---|---|
| Collision shapes | 3101 (walls, ramps, rubbers, bumpers, gates, spinners, kickers, flippers, plungers) |
| Triggers | 13 |
| Performance | ~1100× real time |
| Balls that do not escape | 36 of 36 |
| One flipper lifts the ball | 1071 table units |
| The plunger gets it out of the lane | 651 units |

The physics, the geometry, the renderer and the loop that joins them are there,
the table's script runs, the machine's own firmware runs the game, it takes a
coin, serves a ball, and it makes noise. The lamps light and the score shows on
the machine's head.

---

## Done

These were items 1 to 3 and the nudge. They are written down because the *why*
of each one is worth keeping, and because the sharp edges found along the way
are the kind of thing that gets rediscovered painfully.

### The physics in the browser loop

`Player` holds a [`Table`](../crates/vpw-player/src/table.rs) — engine, controls
and moving parts — and the fixed-step loop calls `step()` on it, at a fixed
1000 Hz decoupled from the frame rate, as in `PhysicsEngine::UpdatePhysics`. The
state reaches the GPU **once per frame** and not once per step: at 1000 Hz
against 60 fps, writing the matrices every step would be sixteen uploads nobody
ever sees.

The events the physics queues are drained every step whether anything is
listening or not. That is not tidiness: the queue has no bound, so a table left
running with nobody reading it is a slow memory leak.

### The ball

It is `basicBallMid` from the original's `src/meshes/ballMesh.h` — a subdivided
icosahedron, 181 vertices, with its `uv`s already mapped. It was not in
`meshes.bin` for a silly reason: `tools/convert_meshes.py` skipped the file
because the array size is written `[320*3]` and the regex only accepted digits.
Fixing that added the ball and left the other 34 meshes byte for byte identical.

`Ball::orientation` was already a quaternion and already integrated correctly,
so the ball rolls instead of sliding. A ball held in a kicker is drawn one radius
lower, so it looks sunk into the hole rather than floating over it
(`ball.cpp:445`).

### Everything that moves

A separate pass, in [`vpw-render/src/dynamic.rs`](../crates/vpw-render/src/dynamic.rs):
each piece keeps its vertices in a local frame, gets its own model matrix in
group 2 and its own draw call. About a dozen draw calls against a table's
twenty-six, which does not move the needle.

The shader is split in three so the lighting exists once:
`material.wgsl` holds everything downstream of the vertex stage, and
`table_vs.wgsl` / `dynamic_vs.wgsl` hold one vertex stage each. Putting a model
matrix on the static path too, set to the identity, would have added a matrix
multiply to every one of a table's eighty thousand baked vertices in order to
serve about a dozen.

The moving pieces are **removed** from the baked scene (`Scene::remove`). If they
are not, each one is drawn twice: once frozen at its rest pose and once following
the physics.

| part | what drives it |
|---|---|
| flipper | `angle_cur`, as a rotation about its own pivot |
| gate, spinner | `angle`, about the horizontal axis |
| trigger | `anim_offset`, a translation in z |
| bumper | a ring that drops and comes back on its own |
| plunger | `pos` — and it **stretches**, see below |

The plunger was the surprise. The obvious reading is that the rod slides inside
the barrel; the original's own geometry says otherwise. The last lathe point is
pinned to `rody` and every other one travels with the tip
(`plunger.cpp:509-518`), so pulling it makes the shaft *longer*. Visual Pinball
gets away with it by pre-computing twenty-five whole meshes, one per animation
frame. We draw with one matrix per piece, so the rod is two pieces — a rigid head
and a cylinder that scales — which reproduces it exactly with two matrices.

### The nudge and the tilt

Ported from `physics/cabinet/`, which is real mechanics: a 25 ms impulse shaped
like a raised cosine (`CabModelKeyboardNudge`), a 113 kg cabinet as one damped
harmonic oscillator per axis with frequencies measured on real machines
(`CabinetPhysics`), and a ten-centimetre pendulum with a ring around it
(`PlumbHandler`).

One departure, and it is measured. With the original's default threshold of one
degree, a keyboard nudge takes the plumb to 53% of the ring and the strongest one
the scatter produces to 74%: it **never** touches. Ten in a row do not either, at
any rhythm. That is fine in Visual Pinball, where a real cabinet has an
accelerometer that hits far harder than a key and where the threshold is a user
setting; here it would mean the tilt does not exist. The default is 0.6°, which
leaves a soft nudge free and makes a firm one warn.

Counting the warnings is ours as a fallback. On a real machine the plumb only
closes a contact and **the ROM counts**; on a ROM table it now does, and on a
table without one it is three warnings and the fourth ends the ball.

---

## Done: the table's rules run

`vpw-vbscript` is a VBScript interpreter, checked against the real thing: it
parses all seventy scripts Visual Pinball ships and loads `core.vbs`, the
three-thousand-line library every table shares. `vpw-game` is the wire from it
to the table — names to parts, physics events to handlers, table time to timers.

All four published tables tried load their scripts, run `Table1_Init` **with no
complaints**, and play. Silence on start-up is the result worth having: a
table's start-up is a gauntlet of `On Error Resume Next` blocks that each end in
a `MsgBox` when the host is missing something, and every one of those messages
was a real gap at some point.

### What real tables taught us that no invented test would have

- **A script is stored however its author's editor left it.** Terminator 2's
  uses bare carriage returns as line endings; read as whitespace, the file
  collapses onto one line.
- **`SetLamp (118), 0` is a call with two arguments**, the first parenthesised —
  not a call with one argument whose result is then indexed.
- **`Dim` and `Const` are hoisted to the top of their scope.** `core.vbs` uses
  a constant on line 2090 and declares it on line 2850.
- **`Eval` and `Execute` are separate compilation units**, so `Option Explicit`
  does not reach into them. `core.vbs` asks `IsEmpty(Eval("BallSize"))` to find
  out whether the host defined something, and under the outer rule that is an
  error and the library does not load.
- **`On Error Resume Next` clears `Err`.** `s11.vbs` writes
  `If VPBuildVersion < 0 Or Err Then` meaning "did *that* line fail"; with a
  stale `Err` it takes a path that reads files off a disk a browser does not
  have, and the table reports it cannot find `core.vbs`.
- **A table needs the player's own globals.** One missing `VPBuildVersion` is
  the difference between a table that loads and one that does not.
- **A table's libraries are named at run time.** `LoadVPM "…", "S11.VBS"` is
  `ExecuteGlobal GetTextFile("S11.VBS")`, so the set of libraries a table needs
  is not knowable until it asks.

### What the binding still does not cover

The member surface was measured — parsing four tables and `core.vbs` and
counting every `x.Member` — and the head of that distribution is implemented.
The tail is **accepted and remembered** rather than refused, because a real
Visual Pinball part has all of those members and refusing one would stop a
table dead over a colour. `cargo run -p vpw-game --example play` prints what a
given table asked for and did not get.

Not modelled at all yet, and each is its own piece of work:

- **Lights.** `State` and `IntensityScale` are stored and nothing draws them.
  The renderer has a light pass; joining the two is the smallest remaining win.
- **Primitives that move.** A script animating a toy with `TransZ` / `RotX` is
  recorded and not drawn.
- **Drop targets.** `IsDropped` is asked for by two of the four tables tried.
- **Sound.** `PlaySound` is counted, not played. See below.

## Done: the ROM runs the game

F-14 is a ROM table, and on a ROM table the script is not the game. The rules
were burned into `f14_u26.l1` in 1987; the table's eleven hundred lines of
VBScript are a switchboard that reports switches to the board and copies back
what it does with its lamps and its solenoids.

`vpw-s11` already emulated that board — three CPUs, six PIAs, both matrices, the
solenoids, the displays. What was missing was the object the script calls
`Controller`, and that is now `vpw-game::controller`. The whole chain runs
against the real firmware in `crates/vpw-game/tests/rom_table.rs`: the zip is
read and inflated, the images are placed by the layout the manifest names, the
board boots, and `Table1_Init` — through `core.vbs`, through `s11.vbs` — starts
the machine and polls it.

Three things it does differently from VPinMAME, all of them on purpose:

- **The board runs on the table's clock**, one millisecond per millisecond,
  instead of on its own thread. Same inputs, same game, every time — which is
  the only way a ROM can take part in a test.
- **A blank machine primes itself.** A System 11 whose memory does not checksum
  shows `FACTORY SETTING` and waits for the operator. Its own boot code writes
  the defaults on the way there, so the second power-up finds them; a VPinMAME
  player is told to press F3 by hand, and there is no reason to make anybody do
  that. Nothing is faked — it is switched off and on again.
- **The machine's memory is saved.** Settings, audits and high scores go back to
  the browser's storage, so it is the same machine next time.

What is not modelled: general illumination, the LED strings, and the flashers
that hang off the sound board rather than the CPU board — a table numbers those
in the same series as the solenoids, up to 32, and only 1 to 16 exist here.

Found on the way: `EG_RATE_SHIFT` in the YM2151 had been transcribed with the
scaling its neighbouring table needs, giving shifts of up to 88. `1 << 88` is a
panic in debug and a different, wrong shift in release. It took a real music ROM
to find it, which is the argument for testing against real firmware.

## Done: the table makes noise

Both halves of it. The sound board's stream comes out of `vpw-s11` as it runs;
the mechanical sounds are the `.vpx`'s own recordings, decoded on load
(`vpw-table::sound`) and started by the script's `PlaySound`. Both go into one
stereo mix (`vpw_audio::mixer`), which the browser plays through an
`AudioWorklet`.

Mixing in Rust rather than handing the samples to Web Audio was deliberate: the
board's stream has to leave wasm as a stream whatever happens, so there is one
buffer and one latency budget either way — and a mix built here can be measured
by a test. `crates/vpw-game/tests/audio.rs` plays the real table and asserts on
what came out: how loud, for how long, on which side.

`PlaySound`'s nine arguments are the fiddly part and none of them mean the
obvious thing. `volume` is added to the sound's own stored level rather than
replacing it. `pan` goes through a tenth root, undoing a convention tables were
written against. `LoopCount` of 1 means play once. And `usesame` without
`restart` **updates the voice that is already playing** — which is how a rolling
sound follows the ball, re-asked for a hundred times a second with a new level
and pitch.

Two things found by listening rather than by reading:

- Both sound boards idle at a level that is not zero — the XS board around
  -0.31, the CS board a flat -0.29. On the real machine a coupling capacitor
  takes that out. Nothing here had one, so a third of the headroom was gone
  before a note played and every buffer boundary was a click.
  `vpw_audio::DcBlocker` is that capacitor.
- `Table1.Width` was not implemented, and the `Pan` function that virtually
  every table copies is `ball.x * 2 / table1.width - 1`. Dividing by zero raised
  and took the whole `PlaySound` line with it, so the ball rolled in silence.

What is still missing: music, which is streamed from a file next to the table
rather than kept inside it; and the front-to-rear fade of a four-speaker
cabinet, which is read from the file and ignored.

## Done: a coin buys a credit

The coin door is wired. The digit row is it: `1` starts a game, `3`, `4` and `5`
are the coin chutes, `6` resets the high scores, `7` and `8` are the up/down and
advance buttons that walk the operator menu, `9` and `0` are the board
diagnostics.

Getting a coin from a keypress to the ROM turned out to run through five
separate bugs, each of which silently did nothing:

1. **`Not` bound too tightly.** `Not cb Is Nothing` parsed as
   `(Not cb) Is Nothing`, which raises rather than answering — and that line is
   how `core.vbs` dispatches every solenoid callback.
2. **A dot with a space before it was read as a member access.** `s11.vbs`
   writes `Case StartGameKey    .Switch(swStartButton) = True`, and the greedy
   reading is `StartGameKey.Switch(...)` — a member of a number.
3. **Negative switch numbers.** The coin door's four operator buttons are -7 to
   -4, and they are not in the playfield matrix.
4. **Column 0.** Switches 1 to 8 — tilt, start, the three coin chutes, slam,
   high-score reset — are a ninth column that is never strobed and is always
   part of what the CPU reads.
5. **`Me` was the wrong object.** Inside a class method it resolved to the
   host's idea of `Me` rather than the instance, so `vpmTimer.EnableUpdate Me`
   registered the table instead of the ball trough; and the interpreter cached
   host globals, freezing `Me` and `ActiveBall` at whatever they were the first
   time anything asked.

And a sixth, which was the one holding up everything else:

6. **The numbering itself.** Switches and lamps run **1 to 64, straight
   through**, in sweep order — not column-and-row. Williams' service
   documentation lays the matrix out as a grid and positions get written as two
   digits, which makes 11 to 88 look right; the tables disprove it on their own.
   F-14 has switches 20, 30, 49, 50, 59 and 60 — rows 0 and 9 in a matrix with
   eight — and maps its lights `NFadeL 1, l1` through `NFadeL 64, l64`. Read the
   wrong way every lamp still lights and every switch still closes, just not the
   ones the table meant, which is the kind of wrong that survives a long time.

With that right, the whole thing works: the machine takes a coin, shows the
credit, accepts the start button, announces `BALL 1`, fires the coil under the
trough, and a ball appears in the shooter lane.
`crates/vpw-game/tests/rom_table.rs` asserts exactly that, and it is the widest
assertion in the repository — a keypress reaches the script, the script reaches
the board, the board's own program decides to start a game, and the coil it
fires comes back through the script as a kick.

## Smaller things that show anyway

**Ramps.** They collide now, which is what makes a launch lane work: F-14's
shooter lane is `Ramp3` lifting the ball from the playfield to 61 units over a
wall that is 45 tall, then `Ramp5` carrying it round the top. Ported from
`Ramp::PhysicSetup` — the floor as two triangles per step of the path, the same
two again facing down, the side walls as segments that subdivide where they
climb faster than the collision skin, and a pole at every seam so a ball cannot
slip between two surfaces that meet at an angle. That last part is most of the
five thousand shapes and all of the reason a ramp feels solid.

Finding it turned up a sign error in the edge normal that had been there all
along: `ramp.cpp:428` reads `D = vnext.y - vmiddle.y` and the port had it the
other way round, so the intersection of two nearly-parallel edges — which is
most of a long straight run once the curve is subdivided — landed thousands of
units off the table. It was wrong in the meshes too; the ramps have been drawn
that way since they were written.

**Lamps that flickered.** The matrix is multiplexed *and* modulated: the ROM
strobes one column at a time and dims a lamp by driving it on one pass and
skipping the next. Measured on F-14 it walks the matrix about 67 times a second,
so there is a bit over one pass per display frame and nothing to average within
one — a boolean answer alternates whatever the frame timing, and the whole
playfield strobed at the beat between the two rates.

What settles it on a real machine is the filament, which follows the drive with
a lag of a few tens of milliseconds, so a lamp driven every other pass sits at
half brightness rather than blinking at thirty hertz. The matrix now closes its
sweep when the strobe comes round — the only moment it holds a whole pass — and
reports a **level** rather than a switch, moved towards the measured duty by a
first-order lag. On F-14 the lamps come out steady, at levels between about 0.5
and 1, which is the dimming the ROM was asking for all along.

**Kickers that do not catch.** A saucer in *legacy* mode grabs whatever touches
it however high the ball is riding (`kicker.cpp:1128`); the height test only
applies to modern ones. Most tables' saucers are legacy — F-14's launch-lane
saucer is — and asking the question anyway gave a ball that rolled into the
hole, stopped on top of it and stayed there, with nothing telling the script it
had arrived. Alongside it went the original's own workaround for a ball
crawling over a kicker's bevel and stopping dead on the lip, which it calls an
ugly hack and keeps anyway.

**The ball stuck in `Wall272`.** F-14 has ramp guides drawn as outlines that
fold back on themselves, thinner than the ball itself. A ball that gets in
there does not know how to get out. It is the reason 27 out of 36 reach the
bottom and no more. It is written down and the threshold of the test is set so
that making it worse breaks; fixing it properly probably means detecting those
outlines while building the geometry and treating them as a single double-sided
wall.

**The brightness.** There is still a difference with Visual Pinball that is not
fully explained. Closing it needs a real capture of VP running F-14 to compare
pixel against pixel; measuring by eye is not enough, and I have already drawn
two wrong conclusions from looking at images instead of measuring.

**Drop targets.** The meshes are in the original (`dropTargetT2..T4`) and we did
not convert them. The physics of a target that drops is not there either.

**The plunger's spring.** The original lathes a coil spring behind the rod from
`springLoops` / `springGauge` / `springDiam`. We do not draw it: from the play
camera it sits on the far side of the cabinet wall. Worth revisiting the day
there is a camera that can look at the shooter lane from the side.

**The ball's own shader.** The original gives the ball a whole shader
(`BallShader.hlsl`) with a playfield reflection and the six nearest lamps
reflected on it. Ours is drawn as metal through the table's material shader,
which is what makes a sphere look like a ball rather than a grey plastic
marble, but it is not the same thing.

**The camera.** It is still the inspection camera: drag and wheel. When
`ViewSetup` is ported, the table fixes the camera and this stays only for
looking around.

---

## The order I propose

1. **Drop targets** — `IsDropped` is asked for by two of the four tables tried,
   and neither the meshes nor the physics of a target that drops are there.
2. **A machine to look at** — the head is a plain panel standing where a real
   one would be, which is enough for the camera to frame and for the score to
   sit on, and is not a machine. The cabinet, the sides, the legs and the
   artwork are all still missing.
3. **The fourteen-segment font** — the reverse table that turns segments back
   into characters is written by hand and has holes in it, so a real ROM's
   output comes back with a `?` in it now and then. It only affects the text a
   script can read, not what is drawn, and the fix is to invert PinMAME's
   `core_ascii2seg16` rather than to keep guessing at letterforms.

Done since this was written: the lamps, which needed the numbering fixed first
(a System 11 runs 1 to 64, not WPC's 11 to 88), and the displays, which are now
drawn from the raw segments onto the machine's head.
