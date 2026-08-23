#!/usr/bin/env python3
"""Builds `docs/parity.md`: which of the original's runtime functions this port
has a counterpart for.

Run it from the repository root, with the two originals checked out beside it:

    ../vpinball    https://github.com/vpinball/vpinball
    ../pinmame     https://github.com/vpinball/pinmame

    python tools/parity.py

Why this exists
---------------

Chasing a symptom can only find code that is *wrong*. It can never find code
that is *absent*, because a function nobody translated produces no failure to
point at — only a table that feels off in a way nobody can reproduce on demand.
Half of the real divergences found in this port so far were omissions of that
kind: a ramp's floor joints, a flipper's contact handler, the spin damping for a
resting ball.

So this walks the original's runtime functions and asks, for each one, whether
the port names anything that could be it. A `?` is not proof of a correct
translation — only that somebody has been there. Turning a `?` into a verified
row means reading both sides.

What it deliberately ignores
----------------------------

Everything the port has no business having: the editor, serialisation, mesh
generation, the UI, and the COM property accessors. This is a player.
"""

import collections
import pathlib
import re

# The files of the original this port claims to translate. Adding a crate means
# adding its counterparts here; a file absent from this list is simply not
# counted, which is a lie of omission the ledger cannot detect on its own.
TARGETS = {
    "vpinball/src/physics": [
        "collide.cpp",
        "collideex.cpp",
        "hitball.cpp",
        "PhysicsEngine.cpp",
        "quadtree.cpp",
        "hitflipper.cpp",
        "hitplunger.cpp",
    ],
    "vpinball/src/parts": [
        "surface.cpp",
        "ramp.cpp",
        "rubber.cpp",
        "kicker.cpp",
        "trigger.cpp",
        "bumper.cpp",
        "gate.cpp",
        "spinner.cpp",
        "flipper.cpp",
        "hittarget.cpp",
    ],
    "pinmame/src/wpc": ["s11.c", "core.c"],
}

# Editor, serialisation, UI, mesh building and COM plumbing.
SKIP = re.compile(
    r"^(get_|put_|Interface|Render|Draw|UIRender|Set(Defaults|DefaultPhysics)"
    r"|WriteRegDefaults|CopyForPlay|UpdateStatusBarInfo|SaveData|LoadData"
    r"|InitLoad|InitPostLoad|InitVBA|GetTypeName|GetPTable|MoveOffset|GetCenter"
    r"|PutCenter|EndPlay|Delete|Uncreate|ClearForOverwrite|AddPoint|DoCommand"
    r"|GetProperty|IsTransparent|UpdatePropertyPanes|SetSelectState"
    r"|GetDialogPanes|ExportMesh|SetObjectPos|Flip[XY]|EditMenu"
    r"|GetBoundingVertices|UpdateBounds|GetDepth|PhysicRelease"
    r"|Generate\w+Mesh)"
)

# A C++ member definition, and a C function definition, at column zero.
CPP = re.compile(r"^[A-Za-z_][\w\s\*&:<>,]*?\b(\w+)::(\w+)\s*\(")
CFN = re.compile(
    r"^(?:INLINE\s+|static\s+)?"
    r"(?:void|int|float|UINT\d+|INT\d+|BOOL|char|double)\s+\**(\w+)\s*\("
)
KEYWORDS = {"if", "for", "while", "switch", "return", "else"}


def snake(name: str) -> str:
    """`ApplyFriction` to `apply_friction`, which is what the port would call it."""
    once = re.sub(r"(.)([A-Z][a-z]+)", r"\1_\2", name)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", once).lower()


def main() -> None:
    root = pathlib.Path(__file__).resolve().parent.parent
    port = "\n".join(
        p.read_text(encoding="utf-8", errors="replace")
        for p in (root / "crates").rglob("*.rs")
    ).lower()

    rows: dict[str, list[tuple[str, int, bool]]] = collections.defaultdict(list)
    total = found = 0
    for directory, files in TARGETS.items():
        for name in files:
            path = root.parent / directory / name
            if not path.is_file():
                print(f"  (missing, not counted: {directory}/{name})")
                continue
            seen: set[str] = set()
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
            for number, line in enumerate(lines, 1):
                match = CPP.match(line)
                if match:
                    full, short = f"{match.group(1)}::{match.group(2)}", match.group(2)
                else:
                    plain = CFN.match(line)
                    if not plain:
                        continue
                    full = short = plain.group(1)
                if short in KEYWORDS or SKIP.match(short) or full in seen:
                    continue
                seen.add(full)
                total += 1
                here = short.lower() in port or snake(short) in port
                found += here
                rows[name].append((full, number, here))

    if not total:
        raise SystemExit("no originals found; check them out beside this repo")

    out = [
        "# Translation parity ledger",
        "",
        "Generated, not written by hand — regenerate with `tools/parity.py` after",
        "any change to either side. It answers one question: **which of the",
        "original's runtime functions this port has a counterpart for**, and which",
        "it has never mentioned.",
        "",
        "The point is coverage rather than findings. Chasing a symptom can only",
        "find code that is *wrong*; it can never find code that is *absent*,",
        "because a function nobody translated produces no failure to point at —",
        "only a table that feels off. Every row here is either accounted for or it",
        "is not.",
        "",
        "A name being present is evidence somebody has been there, **not** that the",
        "translation is right. Verifying a row means reading both sides and marking",
        "it.",
        "",
        "Editor, serialisation, mesh generation and UI entry points are filtered",
        "out: this is a player and does not have them.",
        "",
        "| | |",
        "|---|---|",
        f"| runtime functions in the files this port claims | {total} |",
        f"| named somewhere in the port | {found} ({100 * found // total}%) |",
        f"| never mentioned | {total - found} |",
        "",
    ]
    for name in sorted(rows):
        missing = [r for r in rows[name] if not r[2]]
        out.append(f"## {name} — {len(rows[name]) - len(missing)}/{len(rows[name])}")
        out.append("")
        if not missing:
            out.append("Every runtime function has a counterpart by name.")
            out.append("")
            continue
        out.append("Never mentioned in the port:")
        out.append("")
        out += [f"- [ ] `{full}` ({name}:{line})" for full, line, _ in missing]
        out.append("")

    (root / "docs" / "parity.md").write_text("\n".join(out), encoding="utf-8")
    print(f"docs/parity.md: {total} functions, {found} with a counterpart, {total - found} without")


if __name__ == "__main__":
    main()
