//! Baking the general illumination onto the playfield.
//!
//! The runtime GI (`gi_diffuse` in `material.wgsl`) is honest about where the
//! light comes from and blind about what stands in its way: a point light
//! shines through a post, a ramp, a slingshot wall. What a photograph of a
//! real machine shows between the bulbs is exactly those shadows — the posts
//! print them radially across the wood — and no amount of point lights draws
//! a shadow.
//!
//! So the lamps that switch together are traced here instead, once per table,
//! on the CPU: for every texel of the playfield, a shadow ray to every lamp
//! of the group, against the table's own render meshes — plus one diffuse
//! bounce, gathered over the hemisphere, which is what carries light around a
//! corner and a wall's colour onto the wood beside it. The result is one HDR
//! lightmap per group in the playfield's UV space, and at run time each
//! group's *live level* scales its layer, so the machine still owns every
//! switch: a game that cuts the GI for a light show cuts the map with it, and
//! a table that flashes its red string against its blue one flashes them.
//!
//! The CPU on purpose. A compute shader would be faster and would not exist
//! on the WebGL2 fallback; tens of millions of rays through a BVH is a second
//! or two of native CPU and an acceptable one-time cost in wasm — which is
//! why the page caches the result in IndexedDB and pays it once per table.
//!
//! # Tables that bring their own bake
//!
//! A 10.8 table in the lightmap style carries its light transport with it:
//! flashers bound to lamps, traced offline in Blender by its author. Every
//! part of this port's GI departure — the point lights, the bounce, this bake
//! — exists to stand in for exactly that on tables that predate it, so on a
//! table that ships lightmaps the whole departure switches off
//! ([`prebaked`]), and the original's faithful behaviour is the whole story.

use vpw_math::Vec3;
use vpw_table::geometry::{MeshKind, Scene};

/// Resolution of the direct bake, x by y. The field is about twice as long as
/// it is wide, so the texel stays square. Enough for a post's shadow to read
/// as a shadow; the light itself is smooth by nature.
pub const BAKE_W: u32 = 512;
pub const BAKE_H: u32 = 1024;

/// The indirect half is gathered at half resolution: a bounce is smooth by
/// definition, and every texel costs a hemisphere of rays.
const INDIRECT_W: u32 = 256;
const INDIRECT_H: u32 = 512;

/// Hemisphere samples per indirect texel.
pub const INDIRECT_SAMPLES: u32 = 16;

/// How many groups the frame carries — the lightmap array's depth and the
/// levels vector's width. Four holds every table seen so far: a warm string,
/// two effect strings, and one stray.
pub const MAX_GROUPS: usize = 4;

/// A lamp as the baker sees it, in world units.
pub struct BakeLamp {
    pub center: Vec3,
    pub falloff: f32,
    pub power: f32,
    /// Colour already at full level and the runtime calibration — what one
    /// texel gains with nothing in the way and no distance to fall over.
    pub color: [f32; 3],
}

/// One switchable set of lamps, and where they live in `scene.lights`.
pub struct GiGroup {
    pub indices: Vec<usize>,
    /// The lamps' names, which is how a bake stored away from the scene finds
    /// its lamps again.
    pub names: Vec<String>,
    pub lamps: Vec<BakeLamp>,
}

/// The finished bake: one RGBA half-float layer per group, [`BAKE_W`] ×
/// [`BAKE_H`], in the playfield's UV space — `(0,0)` at the field's minimum
/// corner, the same convention its mesh carries.
pub struct GiBakeSet {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Vec<u16>>,
}

/// Whether the table carries its own baked light transport: any flasher bound
/// to a lamp is the 10.8 lightmap pattern, authored with the light already in
/// it. See the module doc for what that switches off.
pub fn prebaked(scene: &Scene) -> bool {
    scene.flashers.iter().any(|f| f.light_map.is_some())
}

/// The lamps worth a bake layer: bulbs whose reach is field-scale.
///
/// An insert lights its own window and is drawn as its artwork either way;
/// only a lamp that pours light across the field is worth tracing. The
/// threshold is in VPU — about two and a half inches. This is also the
/// candidate list for the machine-observed grouping: what is worth watching
/// is exactly what would be worth baking.
pub const FIELD_SCALE_FALLOFF: f32 = 120.0;

fn bakeable(l: &vpw_table::light::Light) -> bool {
    l.is_bulb && l.falloff_radius >= FIELD_SCALE_FALLOFF && l.intensity > 0.0
}

/// The names of every lamp worth watching, for `vpw-game`'s observer.
pub fn field_scale_candidates(scene: &Scene) -> Vec<String> {
    scene
        .lights
        .iter()
        .filter(|l| bakeable(l))
        .map(|l| l.name.clone())
        .collect()
}

/// Builds bake groups from groups of lamp names — the machine's own answer to
/// what switches together, observed by `vpw_game::grouping` and handed back
/// here. Names that resolve to nothing bakeable are dropped; groups that end
/// up empty are dropped with them; what remains is capped at [`MAX_GROUPS`]
/// by flux, brightest first, and the lamps that fall off the cap stay on the
/// runtime point path.
pub fn gi_groups_from_names(scene: &Scene, observed: &[Vec<String>]) -> Vec<GiGroup> {
    if prebaked(scene) {
        return Vec::new();
    }
    let mut groups = Vec::new();
    for names in observed {
        let mut group = GiGroup {
            indices: Vec::new(),
            names: Vec::new(),
            lamps: Vec::new(),
        };
        for name in names {
            let Some((i, l)) = scene
                .lights
                .iter()
                .enumerate()
                .find(|(_, l)| l.name.eq_ignore_ascii_case(name))
            else {
                continue;
            };
            if !bakeable(l) {
                continue;
            }
            let level = l.intensity * crate::lights::GI_ILLUMINATION;
            group.indices.push(i);
            group.names.push(l.name.clone());
            group.lamps.push(BakeLamp {
                center: l.center,
                falloff: l.falloff_radius,
                power: l.falloff_power,
                color: [l.color[0] * level, l.color[1] * level, l.color[2] * level],
            });
        }
        if !group.lamps.is_empty() {
            groups.push(group);
        }
    }
    let flux = |g: &GiGroup| -> f32 {
        g.lamps
            .iter()
            .map(|l| (l.color[0] + l.color[1] + l.color[2]) * l.falloff * l.falloff)
            .sum()
    };
    groups.sort_by(|a, b| flux(b).total_cmp(&flux(a)));
    groups.truncate(MAX_GROUPS);
    groups
}

/// The GI lamps, grouped by what switches together.
///
/// Phase-two grouping: the bulbs whose name says GI, clustered by colour —
/// F-14's warm string, red string and blue string switch independently and
/// are told apart by nothing else in the file. Telemetry-driven grouping —
/// watching which lamps the ROM moves together — is the honest general answer
/// and comes later. Groups beyond [`MAX_GROUPS`] keep their lamps on the
/// runtime point path instead, which is the graceful half of the answer.
pub fn gi_groups(scene: &Scene) -> Vec<GiGroup> {
    if prebaked(scene) {
        return Vec::new();
    }

    use std::collections::BTreeMap;
    // Quantised colour as the key: lamps of one string share their colour
    // exactly, and a tenth is far coarser than any author's palette.
    let mut by_color: BTreeMap<[u8; 3], GiGroup> = BTreeMap::new();
    for (i, l) in scene.lights.iter().enumerate() {
        if !l.is_bulb || !l.name.to_ascii_lowercase().starts_with("gi") {
            continue;
        }
        let key = [
            (l.color[0] * 10.0).round() as u8,
            (l.color[1] * 10.0).round() as u8,
            (l.color[2] * 10.0).round() as u8,
        ];
        let level = l.intensity * crate::lights::GI_ILLUMINATION;
        let group = by_color.entry(key).or_insert_with(|| GiGroup {
            indices: Vec::new(),
            names: Vec::new(),
            lamps: Vec::new(),
        });
        group.indices.push(i);
        group.names.push(l.name.clone());
        group.lamps.push(BakeLamp {
            center: l.center,
            falloff: l.falloff_radius,
            power: l.falloff_power,
            color: [l.color[0] * level, l.color[1] * level, l.color[2] * level],
        });
    }

    let mut groups: Vec<GiGroup> = by_color.into_values().collect();
    // The brightest groups keep the layers; the rest stay runtime points.
    let flux = |g: &GiGroup| -> f32 {
        g.lamps
            .iter()
            .map(|l| (l.color[0] + l.color[1] + l.color[2]) * l.falloff * l.falloff)
            .sum()
    };
    groups.sort_by(|a, b| flux(b).total_cmp(&flux(a)));
    groups.truncate(MAX_GROUPS);
    groups
}

/// Bakes every group's light onto the playfield: direct with shadows, plus
/// `indirect_samples` hemisphere rays of one diffuse bounce per texel — zero
/// keeps the bounce out, which is what the tests use to see it.
pub fn bake_gi_set(scene: &Scene, groups: &[GiGroup], indirect_samples: u32) -> GiBakeSet {
    let bvh = Bvh::occluders(scene);
    let b = &scene.playfield;
    let (dx, dy) = (b.max.x - b.min.x, b.max.y - b.min.y);

    // The direct half, at full resolution, one map per group.
    let mut direct = vec![vec![[0.0f32; 3]; (BAKE_W * BAKE_H) as usize]; groups.len()];
    for j in 0..BAKE_H {
        let y = b.min.y + (j as f32 + 0.5) / BAKE_H as f32 * dy;
        for i in 0..BAKE_W {
            let x = b.min.x + (i as f32 + 0.5) / BAKE_W as f32 * dx;
            // A hair above the wood, so the ray does not start inside
            // whatever stands exactly on it.
            let p = Vec3::new(x, y, 0.5);
            for (g, group) in groups.iter().enumerate() {
                direct[g][(j * BAKE_W + i) as usize] = direct_at(&bvh, &group.lamps, p);
            }
        }
    }

    // The bounce, at half resolution: light that turned one corner. The
    // geometry rays are walked **once** per texel and shared by every group
    // -- where a ray lands does not depend on whose light lands there -- and
    // only the shadow rays at the hit are each group's own.
    let mut indirect = vec![vec![[0.0f32; 3]; (INDIRECT_W * INDIRECT_H) as usize]; groups.len()];
    if indirect_samples > 0 {
        for j in 0..INDIRECT_H {
            let y = b.min.y + (j as f32 + 0.5) / INDIRECT_H as f32 * dy;
            for i in 0..INDIRECT_W {
                let x = b.min.x + (i as f32 + 0.5) / INDIRECT_W as f32 * dx;
                let p = Vec3::new(x, y, 0.5);
                let at = (j * INDIRECT_W + i) as usize;
                gather(
                    &bvh,
                    groups,
                    p,
                    indirect_samples,
                    i ^ (j << 9),
                    &mut indirect,
                    at,
                );
            }
        }
    }

    // The bounce is Monte Carlo and sixteen samples leave speckle. A small
    // separable blur over the indirect map is the whole denoiser this bake
    // needs: the map is one flat plane, so unlike a screen-space denoiser
    // there are no geometry edges to preserve — the only sharp features a
    // playfield's light has live in the direct half, which is deterministic
    // and untouched. Ignis-grade filtering (SVGF and friends) is for one
    // sample per pixel at sixty frames a second; sixteen per texel once can
    // afford to just be smoothed.
    for map in &mut indirect {
        blur_separable(map, INDIRECT_W, INDIRECT_H);
    }

    // Sum, with the bounce sampled bilinearly at the finer grid.
    let mut layers = Vec::with_capacity(groups.len());
    for g in 0..groups.len() {
        let mut texels = Vec::with_capacity((BAKE_W * BAKE_H * 4) as usize);
        for j in 0..BAKE_H {
            for i in 0..BAKE_W {
                let d = direct[g][(j * BAKE_W + i) as usize];
                let ind = sample_bilinear(
                    &indirect[g],
                    INDIRECT_W,
                    INDIRECT_H,
                    (i as f32 + 0.5) / BAKE_W as f32,
                    (j as f32 + 0.5) / BAKE_H as f32,
                );
                for c in 0..3 {
                    texels.push(half::f16::from_f32(d[c] + ind[c]).to_bits());
                }
                texels.push(half::f16::ONE.to_bits());
            }
        }
        layers.push(texels);
    }

    GiBakeSet {
        width: BAKE_W,
        height: BAKE_H,
        layers,
    }
}

/// The group's direct light at a point: every lamp in range, shadow-tested.
/// The same attenuation the runtime lights use, and no cosine for the same
/// reason they carry none: this is bounced room light under a plastic, not a
/// spotlight.
fn direct_at(bvh: &Bvh, lamps: &[BakeLamp], p: Vec3) -> [f32; 3] {
    let mut acc = [0.0f32; 3];
    for lamp in lamps {
        let to = lamp.center - p;
        let dist = to.length();
        let len = dist / lamp.falloff.max(1.0);
        if len >= 1.0 || dist <= 0.0 {
            continue;
        }
        if bvh.blocked(p, to / dist, dist - 1.0) {
            continue;
        }
        let atten = (1.0 - len).powf(lamp.power);
        for (a, c) in acc.iter_mut().zip(&lamp.color) {
            *a += c * atten;
        }
    }
    acc
}

/// One diffuse bounce for every group at once, gathered over the up
/// hemisphere and accumulated into `out[group][at]`.
///
/// Cosine-weighted, so with a Lambertian bounce the estimator collapses to
/// the average of `albedo x direct` over the hits: what the walls and posts
/// around this texel are lit to, in their own colour -- which is where a red
/// slingshot lends the wood beside it its red.
fn gather(
    bvh: &Bvh,
    groups: &[GiGroup],
    p: Vec3,
    samples: u32,
    seed: u32,
    out: &mut [Vec<[f32; 3]>],
    at: usize,
) {
    let mut rng = seed;
    let mut next = move || {
        // PCG-ish: good enough to jitter rays, cheap enough to not matter.
        rng = rng.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        let word = ((rng >> ((rng >> 28) + 4)) ^ rng).wrapping_mul(277_803_737);
        ((word >> 22) ^ word) as f32 / u32::MAX as f32
    };

    for _ in 0..samples {
        let (u1, u2) = (next(), next());
        // Cosine hemisphere about +Z via the concentric disc.
        let r = u1.sqrt();
        let phi = u2 * 2.0 * std::f32::consts::PI;
        let dir = Vec3::new(r * phi.cos(), r * phi.sin(), (1.0 - u1).max(0.0).sqrt());

        let Some((t, tri)) = bvh.closest(p, dir, 4000.0) else {
            continue;
        };
        let hit = p + dir * (t - 0.5).max(0.0);
        let albedo = bvh.tris[tri].albedo;
        for (g, group) in groups.iter().enumerate() {
            let direct = direct_at(bvh, &group.lamps, hit);
            for c in 0..3 {
                out[g][at][c] += albedo[c] * direct[c] / samples as f32;
            }
        }
    }
}

/// A separable 5-tap binomial blur, run once each way.
fn blur_separable(map: &mut [[f32; 3]], w: u32, h: u32) {
    const K: [f32; 5] = [1.0, 4.0, 6.0, 4.0, 1.0];
    let pass = |horizontal: bool, src: &[[f32; 3]], dst: &mut [[f32; 3]]| {
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let mut acc = [0.0f32; 3];
                let mut weight = 0.0f32;
                for (i, k) in K.iter().enumerate() {
                    let o = i as i32 - 2;
                    let (sx, sy) = if horizontal { (x + o, y) } else { (x, y + o) };
                    if sx < 0 || sy < 0 || sx >= w as i32 || sy >= h as i32 {
                        continue;
                    }
                    let s = src[(sy * w as i32 + sx) as usize];
                    weight += k;
                    for c in 0..3 {
                        acc[c] += s[c] * k;
                    }
                }
                dst[(y * w as i32 + x) as usize] = acc.map(|a| a / weight.max(1.0));
            }
        }
    };
    let mut tmp = vec![[0.0f32; 3]; map.len()];
    pass(true, map, &mut tmp);
    pass(false, &tmp, map);
}

fn sample_bilinear(map: &[[f32; 3]], w: u32, h: u32, u: f32, v: f32) -> [f32; 3] {
    let x = (u * w as f32 - 0.5).clamp(0.0, w as f32 - 1.0);
    let y = (v * h as f32 - 0.5).clamp(0.0, h as f32 - 1.0);
    let (x0, y0) = (x as u32, y as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: u32, y: u32| map[(y * w + x) as usize];
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        out[i] = top + (bottom - top) * fy;
    }
    out
}

/// A ray-traceable triangle, with the colour its surface would bounce.
struct Tri {
    a: Vec3,
    edge1: Vec3,
    edge2: Vec3,
    albedo: [f32; 3],
}

/// A bounding-volume hierarchy over the table's occluders, for the two
/// questions the bake asks: is anything between here and the lamp, and what
/// is the first thing this way.
struct Bvh {
    tris: Vec<Tri>,
    nodes: Vec<Node>,
}

struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: `start..start+len` into `tris`. Inner: `len` is zero, the left
    /// child follows this node directly (the build is pre-order), and `start`
    /// is the right child's index.
    start: u32,
    len: u32,
}

/// Leaves stop splitting at this many triangles.
const LEAF: usize = 8;

impl Bvh {
    /// The meshes a ray should respect: everything visible and opaque that is
    /// not the playfield itself — the floor cannot shadow the floor — and not
    /// the head, which stands behind the lamps, not between them and the
    /// wood. A translucent plastic is left out too: it tints and scatters
    /// rather than blocks, and a bake that treats it as a wall paints the
    /// slingshots black under their own covers.
    fn occluders(scene: &Scene) -> Self {
        let mut tris = Vec::new();
        for mesh in scene.meshes.iter().filter(|m| m.additive.is_none()) {
            if !mesh.visible || matches!(mesh.kind, MeshKind::Playfield | MeshKind::Backbox) {
                continue;
            }
            let material = scene.material(&mesh.material);
            let translucent = material.is_some_and(|m| {
                m.is_transparent(scene.image(&mesh.image).is_some_and(|i| i.has_alpha))
            });
            if translucent {
                continue;
            }
            // What the surface bounces: its material's diffuse colour. Its
            // picture would be more honest and is not carried per triangle;
            // the material is what most walls and posts are painted with.
            let albedo = material.map_or([0.5; 3], |m| m.base_color);
            let world: Vec<Vec3> = mesh
                .baked()
                .iter()
                .map(|v| Vec3::from_array(v.pos))
                .collect();
            for t in mesh.indices.chunks(3) {
                let (a, b, c) = (
                    world[t[0] as usize],
                    world[t[1] as usize],
                    world[t[2] as usize],
                );
                tris.push(Tri {
                    a,
                    edge1: b - a,
                    edge2: c - a,
                    albedo,
                });
            }
        }

        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let mut nodes = Vec::new();
        if !tris.is_empty() {
            build(&tris, &mut order, 0, &mut nodes);
        }
        // The build sorted `order`; store the triangles in that order so a
        // leaf is a contiguous run.
        let mut sorted = Vec::with_capacity(tris.len());
        for &i in &order {
            let t = &tris[i as usize];
            sorted.push(Tri {
                a: t.a,
                edge1: t.edge1,
                edge2: t.edge2,
                albedo: t.albedo,
            });
        }
        Self {
            tris: sorted,
            nodes,
        }
    }

    /// Whether anything sits on the ray before `tmax`.
    fn blocked(&self, origin: Vec3, dir: Vec3, tmax: f32) -> bool {
        self.walk(origin, dir, tmax, true).is_some()
    }

    /// The nearest triangle on the ray, if any: `(t, index)`.
    fn closest(&self, origin: Vec3, dir: Vec3, tmax: f32) -> Option<(f32, usize)> {
        self.walk(origin, dir, tmax, false)
    }

    fn walk(&self, origin: Vec3, dir: Vec3, tmax: f32, any: bool) -> Option<(f32, usize)> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = Vec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut best: Option<(f32, usize)> = None;
        let mut limit = tmax;
        let mut stack = [0u32; 64];
        let mut top = 0usize;
        stack[top] = 0;
        top += 1;
        while top > 0 {
            top -= 1;
            let idx = stack[top];
            let node = &self.nodes[idx as usize];
            if !slab_hit(node.min, node.max, origin, inv, limit) {
                continue;
            }
            if node.len > 0 {
                let (s, e) = (node.start as usize, (node.start + node.len) as usize);
                for (offset, t) in self.tris[s..e].iter().enumerate() {
                    if let Some(hit_t) = hit(t, origin, dir, limit) {
                        if any {
                            return Some((hit_t, s + offset));
                        }
                        best = Some((hit_t, s + offset));
                        limit = hit_t;
                    }
                }
            } else {
                stack[top] = idx + 1;
                stack[top + 1] = node.start;
                top += 2;
            }
        }
        best
    }
}

/// Builds one node over `order` — a slice starting at `base` within the
/// whole — and recurses; returns its index.
fn build(tris: &[Tri], order: &mut [u32], base: u32, nodes: &mut Vec<Node>) -> u32 {
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &i in order.iter() {
        let t = &tris[i as usize];
        for p in [t.a, t.a + t.edge1, t.a + t.edge2] {
            min = min.min(p);
            max = max.max(p);
        }
    }

    let at = nodes.len() as u32;
    nodes.push(Node {
        min,
        max,
        start: 0,
        len: 0,
    });

    if order.len() <= LEAF {
        nodes[at as usize].start = base;
        nodes[at as usize].len = order.len() as u32;
        return at;
    }

    // Split at the median of the widest axis: not the best tree, but an
    // honest one, and the bake is run once.
    let size = max - min;
    let axis = if size.x > size.y && size.x > size.z {
        0
    } else if size.y > size.z {
        1
    } else {
        2
    };
    let centroid = |t: &Tri| {
        let c = t.a + (t.edge1 + t.edge2) / 3.0;
        match axis {
            0 => c.x,
            1 => c.y,
            _ => c.z,
        }
    };
    let mid = order.len() / 2;
    order.sort_by(|&x, &y| centroid(&tris[x as usize]).total_cmp(&centroid(&tris[y as usize])));
    let (left, right) = order.split_at_mut(mid);

    build(tris, left, base, nodes);
    let right_at = build(tris, right, base + mid as u32, nodes);
    nodes[at as usize].start = right_at;
    at
}

fn slab_hit(min: Vec3, max: Vec3, origin: Vec3, inv: Vec3, tmax: f32) -> bool {
    let t0 = (min - origin) * inv;
    let t1 = (max - origin) * inv;
    let lo = t0.min(t1);
    let hi = t0.max(t1);
    let enter = lo.x.max(lo.y).max(lo.z).max(0.0);
    let exit = hi.x.min(hi.y).min(hi.z).min(tmax);
    enter <= exit
}

/// Möller–Trumbore. Answers the hit distance on `0..tmax`.
fn hit(t: &Tri, origin: Vec3, dir: Vec3, tmax: f32) -> Option<f32> {
    let p = dir.cross(t.edge2);
    let det = t.edge1.dot(p);
    if det.abs() < 1e-8 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - t.a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(t.edge1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let hit_t = t.edge2.dot(q) * inv;
    (hit_t > 1e-4 && hit_t < tmax).then_some(hit_t)
}
