//! Tests for the broadphase.
//!
//! The one thing a broadphase **cannot** do is miss a candidate: if it does
//! not report it, the ball goes through the object. Returning too many only
//! costs time. Almost every test in here verifies that asymmetry.

use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::quadtree::{Entry, Quadtree};
use vpw_physics::{Aabb, aabb_of_points};

fn bbox(x: f32, y: f32, half: f32) -> Aabb {
    Aabb {
        min: Vec3::new(x - half, y - half, -10.0),
        max: Vec3::new(x + half, y + half, 10.0),
    }
}

/// A grid of small objects spread over the table.
fn grid(n: usize) -> Vec<Entry> {
    let mut v = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let (x, y) = (i as f32 * 50.0, j as f32 * 50.0);
            v.push(Entry {
                bbox: bbox(x, y, 10.0),
                index: v.len(),
            });
        }
    }
    v
}

/// The correct answer, computed the brute-force way.
fn brute_force(entries: &[Entry], bbox: &Aabb) -> Vec<usize> {
    let mut v: Vec<usize> = entries
        .iter()
        .filter(|e| e.bbox.intersects(bbox))
        .map(|e| e.index)
        .collect();
    v.sort_unstable();
    v
}

#[test]
fn the_tree_does_not_lose_any_candidate() {
    // It is **the** property of the broadphase. It is compared against brute
    // force over many different queries: if the tree reports fewer than the
    // complete list, there are objects the ball would go through.
    let entries = grid(20); // 400 objects
    let tree = Quadtree::build(entries.clone());

    for i in 0..40 {
        let x = (i as f32) * 25.0;
        for j in 0..40 {
            let y = (j as f32) * 25.0;
            let query = bbox(x, y, 30.0);

            let mut from_tree = tree.query_vec(&query);
            from_tree.sort_unstable();
            let expected = brute_force(&entries, &query);

            assert_eq!(
                from_tree, expected,
                "at ({x}, {y}) the tree gave {from_tree:?} and it should have been {expected:?}"
            );
        }
    }
}

#[test]
fn an_object_crossing_a_split_is_found_from_both_sides() {
    // It is the case that makes the tree keep objects in the interior nodes
    // instead of always pushing them down. A long object crossing the middle
    // of the table has to show up wherever it is looked for from.
    let mut entries = grid(10);
    let crossing = Entry {
        bbox: Aabb {
            min: Vec3::new(-500.0, 200.0, -10.0),
            max: Vec3::new(500.0, 220.0, 10.0),
        },
        index: entries.len(),
    };
    entries.push(crossing);
    let tree = Quadtree::build(entries.clone());

    for x in [-400.0f32, -100.0, 0.0, 100.0, 400.0] {
        let query = bbox(x, 210.0, 5.0);
        let found = tree.query_vec(&query);
        assert!(
            found.contains(&crossing.index),
            "at x={x} it did not find the object that crosses the table"
        );
    }
}

#[test]
fn the_tree_really_filters() {
    // If it discarded nothing, it would be useless: a small query on a big
    // table has to look at a tiny fraction of the objects.
    let entries = grid(30); // 900 objects
    let tree = Quadtree::build(entries);

    let query = bbox(0.0, 0.0, 20.0);
    let found = tree.query_vec(&query);

    assert!(
        found.len() < 20,
        "a small query should return few; it returned {}",
        found.len()
    );
}

#[test]
fn the_tree_splits_when_there_are_many_objects() {
    let few = Quadtree::build(grid(2)); // 4 objects
    assert_eq!(few.depth(), 1, "with four or fewer it does not split");

    let many = Quadtree::build(grid(20)); // 400
    assert!(
        many.depth() > 3,
        "with 400 it has to split; it gave {}",
        many.depth()
    );
    assert!(
        many.node_count() > 1,
        "and have more than one node; it gave {}",
        many.node_count()
    );
}

#[test]
fn an_empty_tree_does_not_blow_up() {
    let tree = Quadtree::build(Vec::new());
    assert!(tree.is_empty());
    assert_eq!(tree.query_vec(&bbox(0.0, 0.0, 100.0)).len(), 0);
    assert_eq!(tree.depth(), 1);
}

#[test]
fn objects_all_in_the_same_place_do_not_hang_the_build() {
    // If every object shares a box, no split separates them. The tree has to
    // notice and stop splitting, not go down to the depth limit.
    let entries: Vec<Entry> = (0..100)
        .map(|i| Entry {
            bbox: bbox(0.0, 0.0, 10.0),
            index: i,
        })
        .collect();
    let tree = Quadtree::build(entries);

    assert_eq!(tree.len(), 100);
    assert!(
        tree.depth() <= 2,
        "there is no point splitting what does not separate; it gave depth {}",
        tree.depth()
    );
    // And it still finds every one of them.
    assert_eq!(tree.query_vec(&bbox(0.0, 0.0, 5.0)).len(), 100);
}

#[test]
fn a_balls_query_uses_its_motion_box() {
    // The ball's box grows with the velocity, so a fast ball has to see more
    // candidates than a still one in the same place.
    let entries = grid(20);
    let tree = Quadtree::build(entries);

    let mut still = Ball::new(Vec3::new(200.0, 200.0, 0.0), 25.0);
    let mut seen_still = 0;
    tree.query_ball(&still, |_| seen_still += 1);

    still.vel = Vec3::new(300.0, 0.0, 0.0);
    let mut seen_fast = 0;
    tree.query_ball(&still, |_| seen_fast += 1);

    assert!(
        seen_fast > seen_still,
        "moving it has to see more: {seen_still} vs {seen_fast}"
    );
}

#[test]
fn the_query_does_not_allocate() {
    // The closure version exists precisely for this: in the physics loop it is
    // called a thousand times per second per ball, and allocating a `Vec` each
    // time would be asking the allocator for a thousand allocations per second
    // only to throw them away.
    //
    // The allocation cannot be measured from a test without instrumenting the
    // allocator, but we can pin down that the API exists and that it gives the
    // same thing as the convenient version.
    let entries = grid(15);
    let tree = Quadtree::build(entries);
    let query = bbox(100.0, 100.0, 60.0);

    let mut with_closure = Vec::new();
    tree.query(&query, |i| with_closure.push(i));
    with_closure.sort_unstable();

    let mut with_vec = tree.query_vec(&query);
    with_vec.sort_unstable();

    assert_eq!(
        with_closure, with_vec,
        "the two ways have to give the same thing"
    );
}

#[test]
fn height_filters_too() {
    // Two objects in the same place on the plane but at different heights: a
    // ball rolling underneath must not see the one up top.
    let low = Entry {
        bbox: Aabb {
            min: Vec3::new(-10.0, -10.0, 0.0),
            max: Vec3::new(10.0, 10.0, 20.0),
        },
        index: 0,
    };
    let high = Entry {
        bbox: Aabb {
            min: Vec3::new(-10.0, -10.0, 500.0),
            max: Vec3::new(10.0, 10.0, 520.0),
        },
        index: 1,
    };
    let tree = Quadtree::build(vec![low, high]);

    let query = Aabb {
        min: Vec3::new(-5.0, -5.0, 0.0),
        max: Vec3::new(5.0, 5.0, 30.0),
    };
    let found = tree.query_vec(&query);
    assert_eq!(found, vec![0], "the one up top must not show up");
}

#[test]
fn the_box_of_a_set_of_points_contains_them_all() {
    let points = [
        Vec3::new(-10.0, 5.0, 3.0),
        Vec3::new(20.0, -8.0, 100.0),
        Vec3::new(0.0, 0.0, -50.0),
    ];
    let c = aabb_of_points(&points);
    for p in points {
        assert!(
            p.x >= c.min.x && p.x <= c.max.x && p.y >= c.min.y && p.y <= c.max.y,
            "{p:?} ended up outside {c:?}"
        );
    }
    assert!((c.min.z - (-50.0)).abs() < 1e-5);
    assert!((c.max.z - 100.0).abs() < 1e-5);
}
