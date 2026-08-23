//! Lists the meshes that fall outside the playfield's extent.
//!
//! Useful for telling whether a primitive is far away because the table put it
//! there — a backglass, a DMD panel — or because we transformed it wrong.

fn main() {
    let path = std::env::args().nth(1).expect("usage: escapes <table.vpx>");
    let vpx = vpin::vpx::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
    let sc = vpw_table::geometry::extract(&vpx);
    let pf = sc.playfield;
    let (width, length) = (pf.max.x - pf.min.x, pf.max.y - pf.min.y);

    println!(
        "playfield  x {:.0}..{:.0}  y {:.0}..{:.0}",
        pf.min.x, pf.max.x, pf.min.y, pf.max.y
    );
    println!();

    let mut outside = 0;
    for m in sc.meshes.iter().filter(|m| m.visible) {
        let Some(b) = m.bounds() else { continue };
        // How far it sticks out, in table widths.
        let dx = ((pf.min.x - b.min.x).max(0.0) + (b.max.x - pf.max.x).max(0.0)) / width;
        let dy = ((pf.min.y - b.min.y).max(0.0) + (b.max.y - pf.max.y).max(0.0)) / length;
        let d = dx.max(dy);
        if d > 0.15 {
            outside += 1;
            println!(
                "{:32} sticks out {:5.1}%  x {:7.0}..{:7.0}  y {:7.0}..{:7.0}  z {:6.0}..{:6.0}",
                m.name,
                d * 100.0,
                b.min.x,
                b.max.x,
                b.min.y,
                b.max.y,
                b.min.z,
                b.max.z
            );
        }
    }
    println!();
    println!(
        "{outside} of {} visible meshes stick out more than 15%",
        sc.meshes.iter().filter(|m| m.visible).count()
    );
}
