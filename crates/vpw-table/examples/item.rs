//! Prints what the file says about one part, field by field.
//!
//! ```text
//! cargo run -p vpw-table --example item -- table.vpx Primitive186
//! ```
//!
//! The question it answers is "why is this thing on screen when Visual Pinball
//! does not draw it", and the answer is always a field: an alpha, a blend
//! mode, a flag that says the part is a toy, a layer that is switched off.
//! Guessing which one costs an afternoon; reading them costs a second.

use vpin::vpx::gameitem::GameItemEnum;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: item <table.vpx> <name>");
    let wanted = args.next().unwrap_or_default().to_lowercase();

    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");

    let g = &vpx.gamedata;
    println!("table object name: {:?}", g.name);
    println!(
        "desktop view: mode {:?} fov {} inclination {} layback {} offset ({}, {}, {}) scale ({}, {}, {})",
        g.bg_view_mode_desktop,
        g.bg_fov_desktop,
        g.bg_inclination_desktop,
        g.bg_layback_desktop,
        g.bg_offset_x_desktop,
        g.bg_offset_y_desktop,
        g.bg_offset_z_desktop,
        g.bg_scale_x_desktop,
        g.bg_scale_y_desktop,
        g.bg_scale_z_desktop
    );
    println!(
        "cabinet view: mode {:?} fov {} inclination {} layback {} offset ({}, {}, {})",
        g.bg_view_mode_fullscreen,
        g.bg_fov_fullscreen,
        g.bg_inclination_fullscreen,
        g.bg_layback_fullscreen,
        g.bg_offset_x_fullscreen,
        g.bg_offset_y_fullscreen,
        g.bg_offset_z_fullscreen
    );

    // And the meshes that part became, with the numbers that decide where its
    // texture lands: a UV outside 0..1 is a texture asked to tile, and a UV
    // that runs backwards is a picture asked to mirror.
    let scene = vpw_table::geometry::extract(&vpx);
    for m in scene
        .meshes
        .iter()
        .filter(|m| m.name.to_lowercase().contains(&wanted))
    {
        println!(
            "=== mesh {:?} — {} vertices, {} triangles, image {:?}, clamp {}",
            m.name,
            m.vertices.len(),
            m.triangles(),
            m.image,
            m.clamp
        );
        for (i, v) in m.vertices.iter().enumerate().take(24) {
            println!(
                "  {i:3}  pos ({:8.2}, {:8.2}, {:7.2})  uv ({:7.4}, {:7.4})",
                v.pos[0], v.pos[1], v.pos[2], v.uv[0], v.uv[1]
            );
        }
        println!("  indices {:?}", &m.indices[..m.indices.len().min(36)]);
    }

    for item in &vpx.gameitems {
        let GameItemEnum::Primitive(p) = item else {
            continue;
        };
        if !p.name.to_lowercase().contains(&wanted) {
            continue;
        }
        println!("=== {} ===", p.name);
        println!("  visible          {}", p.is_visible);
        println!("  material         {:?}", p.material);
        println!("  image            {:?}", p.image);
        println!("  alpha            {:?}", p.alpha);
        println!("  color            {:?}", p.color);
        println!("  add blend        {:?}", p.add_blend);
        println!("  depth mask       {:?}", p.use_depth_mask);
        println!("  depth bias       {}", p.depth_bias);
        println!("  toy              {}", p.is_toy);
        println!("  collidable       {}", p.is_collidable);
        println!("  static           {}", p.static_rendering);
        println!("  backfaces        {:?}", p.backfaces_enabled);
        println!("  textures inside  {}", p.draw_textures_inside);
        println!("  reflects         {:?}", p.is_reflection_enabled);
        println!("  disable light    {:?}", p.disable_lighting_top);
        println!("  reflection probe {:?}", p.reflection_probe);
        println!("  refraction probe {:?}", p.refraction_probe);
        println!("  light map        {:?}", p.light_map);
        println!("  layer            {:?}", p.editor_layer_name);
        println!("  layer visible    {:?}", p.editor_layer_visibility);
        println!("  part group       {:?}", p.part_group_name);
        println!("  position         {:?}", p.position);
        println!("  size             {:?}", p.size);
        println!("  rot/tra          {:?}", p.rot_and_tra);
        println!("  use 3d mesh      {}", p.use_3d_mesh);
        println!("  vertices         {:?}", p.num_vertices);
        println!("  indices          {:?}", p.num_indices);
        println!(
            "  animation frames {:?}",
            p.compressed_animation_vertices_len.as_ref().map(Vec::len)
        );
        println!("  file bounds min  {:?}", p.min_aa_bound);
        println!("  file bounds max  {:?}", p.max_aa_bound);
        // The material it names, because half the reasons a part is drawn
        // differently live there rather than on the part.
        if let Some(m) = vpx
            .gamedata
            .materials
            .iter()
            .flatten()
            .find(|m| m.name.eq_ignore_ascii_case(&p.material))
        {
            println!("  --- material {:?}", m.name);
            println!(
                "      opacity      {} active {}",
                m.opacity, m.opacity_active
            );
            println!("      edge alpha   {}", m.edge_alpha);
            println!("      base color   {:?}", m.base_color);
            println!("      type         {:?}", m.type_);
        }
    }
}
