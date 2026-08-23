//! The broadphase: a quadtree over the plane of the table.
//!
//! A real table has thousands of collision objects. Testing the ball against
//! all of them, a thousand times per second, does not add up. The quadtree
//! splits the table into quadrants and leaves in each node only the objects
//! that **fit entirely** inside it; the ones straddling a split stay in the
//! parent.
//!
//! Port of `physics/quadtree.cpp`.
//!
//! # Why a quadtree and not an octree
//!
//! Because a pinball table is flat. Almost all the geometry lives in a band of
//! height that is small compared to the length and the width, so splitting
//! vertically too would separate almost nothing and would cost one more level
//! of tree. Height still comes in, but only as a final box filter.
//!
//! # Two things we do differently from the original
//!
//! **The tree is flat.** The original chains pointers: each node points to a
//! block of four children, and traversing it means jumping around the heap.
//! Here the nodes live in a `Vec` and the children are referenced by index.
//! Traversing the tree becomes a sequential read of contiguous memory, which
//! is what today's caches do well; on top of that it can be cloned and
//! serialized without touching pointers.
//!
//! **Queries do not allocate.** Asking "what can this ball touch?" happens a
//! thousand times per second **per ball**. Returning a fresh `Vec` every time
//! would be asking the allocator for a thousand allocations per second only to
//! throw them away immediately. Instead, [`Quadtree::query`] takes a closure
//! and hands it the indices as it finds them: zero allocations.

use crate::Aabb;
use crate::ball::Ball;
use vpw_math::{Vec2, Vec3};

/// How many objects stop the splitting. It is the original's `<= 4`
/// (`quadtree.cpp:269`, flagged right there as a magic number).
const MIN_ITEMS: usize = 4;

/// How many nodes the traversal stack holds (`quadtree.cpp:21`, `:421`).
const MAX_LEVEL: usize = 128;

/// How deep the tree is allowed to go.
///
/// A third of the stack, which is the original's own arithmetic
/// (`quadtree.cpp:359`, `level + 1 < 128 / 3` sitting beside a `stack[128]`).
/// The relationship is what makes the traversal safe: taking a node off the
/// stack puts at most three back, so a tree this deep cannot ask for more room
/// than there is, and no subtree is ever skipped.
///
/// Reading the 128 as a *depth* cap instead — which is what it looks like —
/// lets the tree grow three times deeper than the stack can walk, and then a
/// query silently stops descending. The shapes down there are still perfectly
/// good; nothing ever asks them.
const MAX_DEPTH: usize = MAX_LEVEL / 3;

/// Taking a node off the stack puts at most three back, so the walk needs three
/// slots per level. Checked here rather than in a test because a test can only
/// try to build a tree deep enough to overflow — and with `f32` coordinates it
/// is not clear one can be built at all. The relationship is the thing that has
/// to hold, so it is the thing that is checked, and breaking it stops the
/// build rather than losing a wall on some table nobody has tried yet.
const _: () = assert!(MAX_DEPTH * 3 <= MAX_LEVEL);

/// The "no children" marker in the flat array.
const NO_CHILDREN: u32 = u32::MAX;

/// An object stored in the tree: its box and an index into the real data.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub bbox: Aabb,
    /// Index of the object inside the array the caller manages.
    pub index: usize,
}

/// A node of the flat tree.
///
/// Its objects are the range `[first_item, first_item + item_count)` of the
/// shared array, and its four children are consecutive starting at
/// `first_child`.
#[derive(Debug, Clone, Copy)]
struct Node {
    center: Vec2,
    first_item: u32,
    item_count: u32,
    /// Index of the first child, or [`NO_CHILDREN`].
    first_child: u32,
}

/// The built tree.
#[derive(Debug, Clone)]
pub struct Quadtree {
    nodes: Vec<Node>,
    /// The objects, reordered so that the ones of each node end up contiguous.
    items: Vec<Entry>,
}

impl Quadtree {
    /// Builds the tree over the given objects.
    pub fn build(entries: Vec<Entry>) -> Self {
        let mut bounds = Aabb::empty();
        for e in &entries {
            bounds = bounds.union(e.bbox);
        }
        if entries.is_empty() {
            bounds = Aabb {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
            };
        }

        let total = entries.len() as u32;
        let mut tree = Self {
            nodes: vec![Node {
                center: Vec2::new(bounds.center().x, bounds.center().y),
                first_item: 0,
                item_count: total,
                first_child: NO_CHILDREN,
            }],
            items: entries,
        };
        tree.subdivide(0, bounds, 0);
        tree
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many nodes the tree has.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Depth of the tree, so we can measure whether it came out balanced.
    pub fn depth(&self) -> usize {
        self.depth_of(0)
    }

    fn depth_of(&self, node: usize) -> usize {
        let n = self.nodes[node];
        if n.first_child == NO_CHILDREN {
            return 1;
        }
        1 + (0..4)
            .map(|i| self.depth_of(n.first_child as usize + i))
            .max()
            .unwrap_or(0)
    }

    /// Walks the objects whose box can touch `bbox`.
    ///
    /// The closure receives the index of each candidate. It is a **filter**:
    /// handing over too many is correct —the narrowphase discards them—,
    /// handing over too few is a bug, because the ball would go through
    /// things.
    pub fn query(&self, bbox: &Aabb, mut visit: impl FnMut(usize)) {
        // An explicit stack instead of recursion: it avoids overflowing with
        // deep trees and is easier to follow in a profiler.
        //
        // It can never fill, and that is not luck: popping a node pushes at
        // most three more than it took off, so the deepest the stack can get is
        // three times the tree's depth. [`MAX_DEPTH`] is a third of it for
        // exactly that reason — it is why the original writes its own cap as
        // `128 / 3` (`quadtree.cpp:359`) beside a `stack[128]`.
        let mut stack = [0u32; MAX_LEVEL];
        let mut top = 1usize;
        stack[0] = 0;

        while top > 0 {
            top -= 1;
            let n = self.nodes[stack[top] as usize];

            let from = n.first_item as usize;
            for e in &self.items[from..from + n.item_count as usize] {
                if e.bbox.intersects(bbox) {
                    visit(e.index);
                }
            }

            if n.first_child == NO_CHILDREN {
                continue;
            }

            // We only descend into the quadrants the box actually touches.
            // This is the entire saving the tree buys.
            let left = bbox.min.x <= n.center.x;
            let right = bbox.max.x >= n.center.x;
            let upper = bbox.min.y <= n.center.y;
            let lower = bbox.max.y >= n.center.y;

            for (i, enters) in [
                (0, upper && left),
                (1, upper && right),
                (2, lower && left),
                (3, lower && right),
            ] {
                if enters {
                    stack[top] = n.first_child + i;
                    top += 1;
                }
            }
        }
    }

    /// The same thing, against a ball's box.
    pub fn query_ball(&self, ball: &Ball, visit: impl FnMut(usize)) {
        self.query(&ball.hit_bbox(), visit);
    }

    /// Version that gathers the indices into a `Vec`. Handy for tests; in the
    /// physics loop [`Quadtree::query`] is better, since it does not allocate.
    pub fn query_vec(&self, bbox: &Aabb) -> Vec<usize> {
        let mut out = Vec::new();
        self.query(bbox, |i| out.push(i));
        out
    }

    /// Splits a node's objects among its four quadrants.
    ///
    /// The ones that cross a split stay in the parent: that is what keeps the
    /// tree from having to duplicate objects or clip them.
    fn subdivide(&mut self, node: usize, bounds: Aabb, level: usize) {
        let n = self.nodes[node];
        if n.item_count as usize <= MIN_ITEMS || level + 1 >= MAX_DEPTH {
            return;
        }

        let center = Vec2::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        );
        self.nodes[node].center = center;

        // Split in place: the ones that stay first, then the four quadrants.
        // That way each node is still a contiguous range.
        let from = n.first_item as usize;
        let to = from + n.item_count as usize;

        let mut staying = Vec::new();
        let mut groups: [Vec<Entry>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];

        for e in &self.items[from..to] {
            // Bit 0: right of the center. Bit 1: below. The 128 marks "crosses
            // the split", and crossing on a single axis is enough to stay up
            // in the parent.
            let x = if e.bbox.min.x > center.x {
                1
            } else if e.bbox.max.x < center.x {
                0
            } else {
                128
            };
            let y = if e.bbox.min.y > center.y {
                2
            } else if e.bbox.max.y < center.y {
                0
            } else {
                128
            };

            match x | y {
                0 => groups[0].push(*e),
                1 => groups[1].push(*e),
                2 => groups[2].push(*e),
                3 => groups[3].push(*e),
                _ => staying.push(*e),
            }
        }

        // If nothing got separated, splitting further is pointless.
        if groups.iter().all(Vec::is_empty) {
            return;
        }

        let mut cursor = from;
        self.items[cursor..cursor + staying.len()].copy_from_slice(&staying);
        self.nodes[node].first_item = cursor as u32;
        self.nodes[node].item_count = staying.len() as u32;
        cursor += staying.len();

        let first_child = self.nodes.len() as u32;
        self.nodes[node].first_child = first_child;

        let (cx, cy) = (center.x, center.y);
        let quadrants = [
            // 0: left-upper, 1: right-upper,
            // 2: left-lower, 3: right-lower.
            Aabb {
                min: bounds.min,
                max: Vec3::new(cx, cy, bounds.max.z),
            },
            Aabb {
                min: Vec3::new(cx, bounds.min.y, bounds.min.z),
                max: Vec3::new(bounds.max.x, cy, bounds.max.z),
            },
            Aabb {
                min: Vec3::new(bounds.min.x, cy, bounds.min.z),
                max: Vec3::new(cx, bounds.max.y, bounds.max.z),
            },
            Aabb {
                min: Vec3::new(cx, cy, bounds.min.z),
                max: bounds.max,
            },
        ];

        for group in &groups {
            self.items[cursor..cursor + group.len()].copy_from_slice(group);
            self.nodes.push(Node {
                center: Vec2::new(cx, cy),
                first_item: cursor as u32,
                item_count: group.len() as u32,
                first_child: NO_CHILDREN,
            });
            cursor += group.len();
        }

        for (i, quadrant) in quadrants.into_iter().enumerate() {
            self.subdivide(first_child as usize + i, quadrant, level + 1);
        }
    }
}
