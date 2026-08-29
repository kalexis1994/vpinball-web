//! Triangle and vertex ordering, for the GPU's two small memories.
//!
//! A GPU keeps the last few dozen transformed vertices in a post-transform
//! cache, and a triangle whose vertices are still in it costs almost nothing
//! to shade. A `.vpx` mesh arrives in whatever order the authoring tool left
//! it — often the order a tessellator emitted, which walks the cache straight
//! through — and on a phone, where the vertex is fetched over a memory bus
//! shared with everything else, the misses are a real part of the frame.
//!
//! Two passes, both order-only — the triangles drawn are identical, which is
//! what makes this free:
//!
//! - [`optimize_order`] is Tom Forsyth's linear-speed vertex cache
//!   optimisation: greedily emit the triangle whose vertices score best,
//!   where a vertex scores high for being warm in a simulated cache and for
//!   having few triangles left that need it (so stragglers are not orphaned).
//! - [`remap_by_first_use`] then renumbers the vertices in the order the
//!   optimised indices first touch them, so vertex *fetch* walks the buffer
//!   forward instead of hopping.
//!
//! Applied per opaque batch: a batch is one draw call, so its order is the
//! cache's whole world — and a transparent batch is left exactly as it is,
//! because there the triangle order is the blending order.

/// The simulated cache. Thirty-two entries models new hardware conservatively
/// and old hardware generously, which is the standard compromise.
const CACHE: usize = 32;
/// Score parameters, from Forsyth's own writeup.
const DECAY_POWER: f32 = 1.5;
const LAST_TRI_SCORE: f32 = 0.75;
const VALENCE_SCALE: f32 = 2.0;
const VALENCE_POWER: f32 = -0.5;

fn vertex_score(cache_pos: i32, remaining: u32) -> f32 {
    if remaining == 0 {
        return -1.0;
    }
    let mut score = 0.0;
    if cache_pos >= 0 {
        if (cache_pos as usize) < 3 {
            // One of the last triangle's own: usable, but not so tempting
            // that the walk keeps strip-mining one fan forever.
            score = LAST_TRI_SCORE;
        } else {
            let scaled = (cache_pos as f32 - 3.0) / (CACHE as f32 - 3.0);
            score = (1.0 - scaled).powf(DECAY_POWER);
        }
    }
    score + VALENCE_SCALE * (remaining as f32).powf(VALENCE_POWER)
}

/// Reorders `indices` (a triangle list) in place for the post-transform
/// cache. Order-only: the set of triangles is untouched.
pub fn optimize_order(indices: &mut [u32]) {
    let tri_count = indices.len() / 3;
    if tri_count < 2 {
        return;
    }
    let vertex_count = match indices.iter().max() {
        Some(&m) => m as usize + 1,
        None => return,
    };

    // Per vertex: how many triangles still need it, and which they are.
    let mut remaining = vec![0u32; vertex_count];
    for &i in indices.iter() {
        remaining[i as usize] += 1;
    }
    let mut offsets = vec![0u32; vertex_count + 1];
    for v in 0..vertex_count {
        offsets[v + 1] = offsets[v] + remaining[v];
    }
    let mut tri_lists = vec![0u32; indices.len()];
    {
        let mut cursor = offsets.clone();
        for t in 0..tri_count {
            for k in 0..3 {
                let v = indices[t * 3 + k] as usize;
                tri_lists[cursor[v] as usize] = t as u32;
                cursor[v] += 1;
            }
        }
    }

    let mut cache_pos = vec![-1i32; vertex_count];
    let mut vscore = vec![0.0f32; vertex_count];
    for v in 0..vertex_count {
        vscore[v] = vertex_score(-1, remaining[v]);
    }
    let mut tscore = vec![0.0f32; tri_count];
    let mut emitted = vec![false; tri_count];
    for t in 0..tri_count {
        for k in 0..3 {
            tscore[t] += vscore[indices[t * 3 + k] as usize];
        }
    }

    let mut cache: Vec<u32> = Vec::with_capacity(CACHE + 3);
    let mut out = Vec::with_capacity(indices.len());
    // The fallback cursor: when nothing in the cache leads anywhere — a new
    // island of the mesh — the walk restarts at the best-scored triangle not
    // yet passed, and never rereads what it has already consumed. Amortised
    // linear over the whole batch.
    let mut fallback = 0usize;

    let mut best_tri = tscore
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(t, _)| t);

    for _ in 0..tri_count {
        if emitted[best_tri] {
            // Scan the cache's triangles for the best candidate.
            let mut best = None;
            for &v in &cache {
                let (s, e) = (offsets[v as usize], offsets[v as usize + 1]);
                for &t in &tri_lists[s as usize..e as usize] {
                    if !emitted[t as usize] {
                        let score = tscore[t as usize];
                        if best.is_none_or(|(bs, _)| score > bs) {
                            best = Some((score, t as usize));
                        }
                    }
                }
            }
            best_tri = match best {
                Some((_, t)) => t,
                None => {
                    while fallback < tri_count && emitted[fallback] {
                        fallback += 1;
                    }
                    if fallback == tri_count {
                        break;
                    }
                    fallback
                }
            };
        }

        emitted[best_tri] = true;
        let tri = [
            indices[best_tri * 3],
            indices[best_tri * 3 + 1],
            indices[best_tri * 3 + 2],
        ];
        out.extend_from_slice(&tri);

        // The three step to the cache's front; everything else slides back.
        for &v in &tri {
            remaining[v as usize] -= 1;
            cache.retain(|&c| c != v);
        }
        for &v in tri.iter().rev() {
            cache.insert(0, v);
        }
        cache.truncate(CACHE);

        // Rescore what moved, and the triangles that read it.
        for (pos, &v) in cache.iter().enumerate() {
            cache_pos[v as usize] = pos as i32;
        }
        for &v in &tri {
            if remaining[v as usize] == 0 {
                cache_pos[v as usize] = -1;
            }
        }
        let mut touched: Vec<u32> = tri.to_vec();
        touched.extend(cache.iter().copied());
        touched.sort_unstable();
        touched.dedup();
        for &v in &touched {
            let new = vertex_score(cache_pos[v as usize], remaining[v as usize]);
            let delta = new - vscore[v as usize];
            if delta != 0.0 {
                vscore[v as usize] = new;
                let (s, e) = (offsets[v as usize], offsets[v as usize + 1]);
                for &t in &tri_lists[s as usize..e as usize] {
                    tscore[t as usize] += delta;
                }
            }
        }

        // The next winner, looked for where it can be: among the cache's.
        let mut best = None;
        for &v in &cache {
            let (s, e) = (offsets[v as usize], offsets[v as usize + 1]);
            for &t in &tri_lists[s as usize..e as usize] {
                if !emitted[t as usize] {
                    let score = tscore[t as usize];
                    if best.is_none_or(|(bs, _)| score > bs) {
                        best = Some((score, t as usize));
                    }
                }
            }
        }
        if let Some((_, t)) = best {
            best_tri = t;
        }
        // Otherwise the top of the loop falls back past the emitted flag.
    }

    debug_assert_eq!(out.len(), indices.len());
    indices.copy_from_slice(&out);
}

/// Renumbers vertices in first-use order of `indices`, rewriting both. The
/// remap covers the whole shared vertex buffer, so it is done once, after
/// every batch's indices are in their final order.
pub fn remap_by_first_use<V: Copy>(vertices: &mut Vec<V>, indices: &mut [u32]) {
    let mut new_of = vec![u32::MAX; vertices.len()];
    let mut order: Vec<u32> = Vec::with_capacity(vertices.len());
    for i in indices.iter_mut() {
        let v = *i as usize;
        if new_of[v] == u32::MAX {
            new_of[v] = order.len() as u32;
            order.push(*i);
        }
        *i = new_of[v];
    }
    // A vertex nothing indexes is dropped: it was never drawn.
    let remapped: Vec<V> = order.iter().map(|&old| vertices[old as usize]).collect();
    *vertices = remapped;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangles(indices: &[u32]) -> Vec<[u32; 3]> {
        let mut tris: Vec<[u32; 3]> = indices
            .chunks(3)
            .map(|t| {
                let mut t = [t[0], t[1], t[2]];
                // Rotation-normalise so the set compare ignores which vertex
                // leads — the optimiser never rotates, but the guard is
                // about the *set* of triangles.
                let min = (0..3).min_by_key(|&k| t[k]).unwrap();
                t.rotate_left(min);
                t
            })
            .collect();
        tris.sort_unstable();
        tris
    }

    #[test]
    fn the_order_changes_and_the_triangles_do_not() {
        // A strip of quads: 0-1-2, 2-1-3, 2-3-4, ... deliberately emitted in
        // a scattered order.
        let mut indices: Vec<u32> = Vec::new();
        for q in 0..64u32 {
            let base = q * 2;
            indices.extend_from_slice(&[base, base + 1, base + 2]);
            indices.extend_from_slice(&[base + 2, base + 1, base + 3]);
        }
        // Scatter: reverse pairs of triangles.
        let mut scattered = indices.clone();
        scattered.chunks_mut(6).for_each(|c| c.rotate_left(3));

        let before = triangles(&scattered);
        optimize_order(&mut scattered);
        assert_eq!(triangles(&scattered), before, "order-only, always");
    }

    #[test]
    fn the_remap_keeps_every_drawn_triangle_pointing_at_the_same_data() {
        let mut vertices: Vec<u32> = (100..116).collect();
        let mut indices: Vec<u32> = vec![7, 3, 12, 3, 7, 1, 15, 0, 7];
        let drawn: Vec<Vec<u32>> = indices
            .chunks(3)
            .map(|t| t.iter().map(|&i| vertices[i as usize]).collect())
            .collect();
        remap_by_first_use(&mut vertices, &mut indices);
        let after: Vec<Vec<u32>> = indices
            .chunks(3)
            .map(|t| t.iter().map(|&i| vertices[i as usize]).collect())
            .collect();
        assert_eq!(drawn, after);
        assert!(vertices.len() <= 16, "unused vertices are dropped");
    }

    #[test]
    fn a_cache_walk_beats_the_scattered_order() {
        // Measure simulated cache misses before and after on a quad grid.
        let side = 24u32;
        let mut indices: Vec<u32> = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let a = y * (side + 1) + x;
                let b = a + 1;
                let c = a + side + 1;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, c, c, b, d]);
            }
        }
        // Scatter deterministically.
        let tris: Vec<[u32; 3]> = indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
        let mut scattered = Vec::new();
        let mut k = 0usize;
        let n = tris.len();
        for _ in 0..n {
            k = (k + 397) % n;
            scattered.extend_from_slice(&tris[k]);
        }

        let misses = |ix: &[u32]| {
            let mut cache: Vec<u32> = Vec::new();
            let mut m = 0usize;
            for &v in ix {
                if !cache.contains(&v) {
                    m += 1;
                    cache.insert(0, v);
                    cache.truncate(CACHE);
                } else {
                    cache.retain(|&c| c != v);
                    cache.insert(0, v);
                }
            }
            m
        };
        let mut optimized = scattered.clone();
        optimize_order(&mut optimized);
        assert!(
            misses(&optimized) * 3 < misses(&scattered) * 2,
            "the walk must cut misses by a third: {} to {}",
            misses(&scattered),
            misses(&optimized)
        );
    }
}
