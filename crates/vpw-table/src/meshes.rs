//! The built-in meshes that Visual Pinball ships.
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
struct Entry {
    name: &'static str,
    vertex_offset: usize,
    vertex_count: usize,
    index_offset: usize,
    index_count: usize,
}

const BLOB: &[u8] = include_bytes!("../assets/meshes.bin");

const MESHES: &[Entry] = &[
    Entry {
        name: "basicBallMid",
        vertex_offset: 0,
        vertex_count: 181,
        index_offset: 5792,
        index_count: 960,
    },
    Entry {
        name: "bulbLight",
        vertex_offset: 9632,
        vertex_count: 67,
        index_offset: 11776,
        index_count: 360,
    },
    Entry {
        name: "bulbSocket",
        vertex_offset: 13216,
        vertex_count: 592,
        index_offset: 32160,
        index_count: 3384,
    },
    Entry {
        name: "bumperBase",
        vertex_offset: 45696,
        vertex_count: 517,
        index_offset: 62240,
        index_count: 2352,
    },
    Entry {
        name: "bumperCap",
        vertex_offset: 71648,
        vertex_count: 839,
        index_offset: 98496,
        index_count: 1194,
    },
    Entry {
        name: "bumperRing",
        vertex_offset: 103272,
        vertex_count: 481,
        index_offset: 118664,
        index_count: 2367,
    },
    Entry {
        name: "bumperSocket",
        vertex_offset: 128132,
        vertex_count: 482,
        index_offset: 143556,
        index_count: 2232,
    },
    Entry {
        name: "hitTargetT2",
        vertex_offset: 152484,
        vertex_count: 88,
        index_offset: 155300,
        index_count: 192,
    },
    Entry {
        name: "hitTargetT3",
        vertex_offset: 156068,
        vertex_count: 36,
        index_offset: 157220,
        index_count: 66,
    },
    Entry {
        name: "hitTargetT4",
        vertex_offset: 157484,
        vertex_count: 68,
        index_offset: 159660,
        index_count: 174,
    },
    Entry {
        name: "flipperBase",
        vertex_offset: 160356,
        vertex_count: 104,
        index_offset: 163684,
        index_count: 300,
    },
    Entry {
        name: "gateBracket",
        vertex_offset: 164884,
        vertex_count: 184,
        index_offset: 170772,
        index_count: 516,
    },
    Entry {
        name: "gateLongPlate",
        vertex_offset: 172836,
        vertex_count: 62,
        index_offset: 174820,
        index_count: 132,
    },
    Entry {
        name: "gatePlate",
        vertex_offset: 175348,
        vertex_count: 70,
        index_offset: 177588,
        index_count: 156,
    },
    Entry {
        name: "gateWire",
        vertex_offset: 178212,
        vertex_count: 186,
        index_offset: 184164,
        index_count: 1008,
    },
    Entry {
        name: "gateWireRectangle",
        vertex_offset: 188196,
        vertex_count: 144,
        index_offset: 192804,
        index_count: 672,
    },
    Entry {
        name: "hitFatTargetRectangle",
        vertex_offset: 195492,
        vertex_count: 302,
        index_offset: 205156,
        index_count: 942,
    },
    Entry {
        name: "hitFatTargetSquare",
        vertex_offset: 208924,
        vertex_count: 302,
        index_offset: 218588,
        index_count: 942,
    },
    Entry {
        name: "hitTargetRectangle",
        vertex_offset: 222356,
        vertex_count: 161,
        index_offset: 227508,
        index_count: 378,
    },
    Entry {
        name: "hitTargetRound",
        vertex_offset: 229020,
        vertex_count: 209,
        index_offset: 235708,
        index_count: 522,
    },
    Entry {
        name: "hitTargetT1Slim",
        vertex_offset: 237796,
        vertex_count: 145,
        index_offset: 242436,
        index_count: 306,
    },
    Entry {
        name: "hitTargetT2Slim",
        vertex_offset: 243660,
        vertex_count: 302,
        index_offset: 253324,
        index_count: 942,
    },
    Entry {
        name: "kickerCup",
        vertex_offset: 257092,
        vertex_count: 373,
        index_offset: 269028,
        index_count: 774,
    },
    Entry {
        name: "kickerGottlieb",
        vertex_offset: 272124,
        vertex_count: 2333,
        index_offset: 346780,
        index_count: 6300,
    },
    Entry {
        name: "kickerHit",
        vertex_offset: 371980,
        vertex_count: 216,
        index_offset: 378892,
        index_count: 0,
    },
    Entry {
        name: "kickerHole",
        vertex_offset: 378892,
        vertex_count: 192,
        index_offset: 385036,
        index_count: 288,
    },
    Entry {
        name: "kickerSimpleHole",
        vertex_offset: 386188,
        vertex_count: 42,
        index_offset: 387532,
        index_count: 126,
    },
    Entry {
        name: "kickerT1",
        vertex_offset: 388036,
        vertex_count: 657,
        index_offset: 409060,
        index_count: 2094,
    },
    Entry {
        name: "kickerWilliams",
        vertex_offset: 417436,
        vertex_count: 1243,
        index_offset: 457212,
        index_count: 3582,
    },
    Entry {
        name: "spinnerBracket",
        vertex_offset: 471540,
        vertex_count: 152,
        index_offset: 476404,
        index_count: 420,
    },
    Entry {
        name: "spinnerPlate",
        vertex_offset: 478084,
        vertex_count: 228,
        index_offset: 485380,
        index_count: 912,
    },
    Entry {
        name: "triggerButton",
        vertex_offset: 489028,
        vertex_count: 528,
        index_offset: 505924,
        index_count: 948,
    },
    Entry {
        name: "triggerInder",
        vertex_offset: 509716,
        vertex_count: 152,
        index_offset: 514580,
        index_count: 312,
    },
    Entry {
        name: "triggerSimple",
        vertex_offset: 515828,
        vertex_count: 49,
        index_offset: 517396,
        index_count: 216,
    },
    Entry {
        name: "triggerStar",
        vertex_offset: 518260,
        vertex_count: 231,
        index_offset: 525652,
        index_count: 510,
    },
    Entry {
        name: "triggerDWire",
        vertex_offset: 527692,
        vertex_count: 203,
        index_offset: 534188,
        index_count: 798,
    },
];

/// A built-in mesh, already decoded.
pub struct BuiltinMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

fn read_f32(offset: usize) -> f32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&BLOB[offset..offset + 4]);
    f32::from_le_bytes(b)
}

fn read_u32(offset: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&BLOB[offset..offset + 4]);
    u32::from_le_bytes(b)
}

/// Looks up a mesh by its name, as it is called in the original
/// (`bumperBase`, `flipperBase`, `hitTargetRound`, ...).
pub fn get(name: &str) -> Option<BuiltinMesh> {
    let e = MESHES.iter().find(|e| e.name == name)?;

    let vertices = (0..e.vertex_count)
        .map(|i| {
            let o = e.vertex_offset + i * 32;
            Vertex {
                pos: [read_f32(o), read_f32(o + 4), read_f32(o + 8)],
                normal: [read_f32(o + 12), read_f32(o + 16), read_f32(o + 20)],
                uv: [read_f32(o + 24), read_f32(o + 28)],
            }
        })
        .collect();

    let indices = (0..e.index_count)
        .map(|i| read_u32(e.index_offset + i * 4))
        .collect();

    Some(BuiltinMesh { vertices, indices })
}

/// The names of every available mesh.
pub fn names() -> impl Iterator<Item = &'static str> {
    MESHES.iter().map(|e| e.name)
}
