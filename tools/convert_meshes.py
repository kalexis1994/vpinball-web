#!/usr/bin/env python3
"""Converts Visual Pinball's built-in meshes to a binary blob.

Visual Pinball ships the meshes for the flippers, bumpers, gates, targets and
the rest as C arrays in `src/meshes/*.h` — 2.3 MB of literals. They are
GPLv3+, same as this project, so they can be ported as they are; what makes no
sense is putting them in as Rust code, because compiling two million float
literals is slow and the binary comes out worse.

Instead they are dumped to a single binary file, in the same format the rest
of the renderer already uses (position, normal, uv as f32), plus a Rust module
with the offsets.

    python tools/convert_meshes.py ../vpinball/src/meshes

It is run once and the result is checked in. You do not need the C++ tree to
build the project.
"""

import re
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT_BIN = ROOT / "crates/vpw-table/assets/meshes.bin"
OUT_RS = ROOT / "crates/vpw-table/src/meshes.rs"

# The size can be written as a product: `basicBallMidIndices[320*3]`.
SIZE = r"[\d\s*]+"

# `static constexpr Vertex3D_NoTex2 name[N] = { {...}, ... };`
RE_VERTICES = re.compile(
    r"Vertex3D_NoTex2\s+(\w+)\s*\[\s*(" + SIZE + r")\s*\]\s*=\s*\{(.*?)\n\s*\};",
    re.S,
)
# `static constexpr WORD nameIndices[M] = { ... };`
RE_INDICES = re.compile(
    r"(?:WORD|unsigned short|unsigned int)\s+(\w+)\s*\[\s*(" + SIZE + r")\s*\]\s*=\s*\{(.*?)\n\s*\};",
    re.S,
)
RE_NUMBER = re.compile(r"-?\d+\.?\d*(?:[eE][-+]?\d+)?f?")


def numbers(text):
    return [float(m.rstrip("f")) for m in RE_NUMBER.findall(text)]


def size(expr):
    """`181` or `320*3`: the original writes some sizes as a product."""
    total = 1
    for factor in expr.split("*"):
        total *= int(factor.strip())
    return total


def base_name(ident):
    """`bumperBaseMesh` and `bumperBaseIndices` are the same mesh."""
    for suffix in ("Mesh", "Indices", "Vertices"):
        if ident.endswith(suffix):
            return ident[: -len(suffix)]
    return ident


def main():
    if len(sys.argv) < 2:
        sys.exit(f"usage: {sys.argv[0]} <path to vpinball/src/meshes>")
    source = Path(sys.argv[1])
    if not source.is_dir():
        sys.exit(f"{source} does not exist")

    meshes = []
    blob = bytearray()

    for file in sorted(source.glob("*.h")):
        text = file.read_text(encoding="utf-8", errors="replace")

        mv = RE_VERTICES.search(text)
        if not mv:
            print(f"  skipped {file.name}: no vertex array")
            continue
        name_v, count_v, body_v = mv.group(1), size(mv.group(2)), mv.group(3)

        vals = numbers(body_v)
        if len(vals) != count_v * 8:
            print(f"  skipped {file.name}: {len(vals)} values, expected {count_v * 8}")
            continue

        # The indices are the first integer array that is **not** the vertex one.
        indices = None
        for mi in RE_INDICES.finditer(text):
            if mi.group(1) == name_v:
                continue
            vals_i = [int(x) for x in re.findall(r"\d+", mi.group(3))]
            if len(vals_i) == size(mi.group(2)):
                indices = vals_i
                break
        # A mesh with no index array is not a broken one. `kickerHitMesh.h` is
        # a point cloud on purpose: nothing ever draws it, and the only thing
        # the original asks of it is "which vertex is nearest the ball, and
        # what is its normal" (`kicker.cpp:1047`). Skipping those, which is
        # what this did, left the one mesh the physics needs out of the blob.
        if indices is None:
            indices = []

        offset_v = len(blob)
        for v in vals:
            blob += struct.pack("<f", v)
        offset_i = len(blob)
        for i in indices:
            blob += struct.pack("<I", i)

        name = base_name(name_v)
        meshes.append((name, offset_v, count_v, offset_i, len(indices)))
        print(f"  {name:26} {count_v:5} vertices  {len(indices):5} indices")

    OUT_BIN.parent.mkdir(parents=True, exist_ok=True)
    OUT_BIN.write_bytes(bytes(blob))

    rows = "\n".join(
        f'    Entry {{ name: "{n}", vertex_offset: {ov}, vertex_count: {cv}, '
        f"index_offset: {oi}, index_count: {ci} }},"
        for n, ov, cv, oi, ci in meshes
    )
    OUT_RS.write_text(
        f'''//! The built-in meshes that Visual Pinball ships.
//!
//! The flippers, the bumpers, the gates, the targets and the kickers are not
//! generated: the original keeps them as C arrays in `src/meshes/*.h`, two and
//! a half million float literals. They are GPLv3+, same as this project.
//!
//! Putting them in as Rust code would be slow to compile and worse to package,
//! so they live in a binary blob with the same format the rest of the renderer
//! uses: per vertex, position, normal and uv as `f32`; then the indices as
//! `u32`.
//!
//! **This file is generated.** To redo it:
//!
//! ```text
//! python tools/convert_meshes.py ../vpinball/src/meshes
//! ```

use crate::geometry::Vertex;

/// Where each mesh lives inside the blob.
struct Entry {{
    name: &'static str,
    vertex_offset: usize,
    vertex_count: usize,
    index_offset: usize,
    index_count: usize,
}}

const BLOB: &[u8] = include_bytes!("../assets/meshes.bin");

const MESHES: &[Entry] = &[
{rows}
];

/// A built-in mesh, already decoded.
pub struct BuiltinMesh {{
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}}

fn read_f32(offset: usize) -> f32 {{
    let mut b = [0u8; 4];
    b.copy_from_slice(&BLOB[offset..offset + 4]);
    f32::from_le_bytes(b)
}}

fn read_u32(offset: usize) -> u32 {{
    let mut b = [0u8; 4];
    b.copy_from_slice(&BLOB[offset..offset + 4]);
    u32::from_le_bytes(b)
}}

/// Looks up a mesh by its name, as it is called in the original
/// (`bumperBase`, `flipperBase`, `hitTargetRound`, ...).
pub fn get(name: &str) -> Option<BuiltinMesh> {{
    let e = MESHES.iter().find(|e| e.name == name)?;

    let vertices = (0..e.vertex_count)
        .map(|i| {{
            let o = e.vertex_offset + i * 32;
            Vertex {{
                pos: [read_f32(o), read_f32(o + 4), read_f32(o + 8)],
                normal: [read_f32(o + 12), read_f32(o + 16), read_f32(o + 20)],
                uv: [read_f32(o + 24), read_f32(o + 28)],
            }}
        }})
        .collect();

    let indices = (0..e.index_count)
        .map(|i| read_u32(e.index_offset + i * 4))
        .collect();

    Some(BuiltinMesh {{ vertices, indices }})
}}

/// The names of every available mesh.
pub fn names() -> impl Iterator<Item = &'static str> {{
    MESHES.iter().map(|e| e.name)
}}
''',
        encoding="utf-8",
    )

    print()
    print(f"{len(meshes)} meshes -> {OUT_BIN} ({len(blob) / 1024:.0f} KB)")
    print(f"index      -> {OUT_RS}")


if __name__ == "__main__":
    main()
