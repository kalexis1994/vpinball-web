//! The pass that draws the table.
//!
//! It lives on its own because both paths use it: the windowless render that
//! writes a PNG from the terminal, and the `<canvas>` one in the browser. That
//! it is the same code is what makes verifying from the terminal worth
//! anything.

use crate::dynamic::DynamicParts;
use crate::flashers::Flashers;
use crate::lights::Lights;
use crate::pipeline::TablePipeline;
use crate::scene::GpuScene;

/// What to do with the attachment when the pass ends: keep it, unless it was
/// only ever the staging ground for a resolve.
fn store_for(resolve: Option<&wgpu::TextureView>) -> wgpu::StoreOp {
    if resolve.is_some() {
        wgpu::StoreOp::Discard
    } else {
        wgpu::StoreOp::Store
    }
}

/// Background color. The original uses the table's `colorbackdrop`; until we
/// read that field, a very dark blue that competes with nothing.
pub const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.02,
    g: 0.02,
    b: 0.04,
    a: 1.0,
};

/// The lights, on their own, into the transmitted-light buffer.
///
/// This runs **before** the table, because the material shader reads what it
/// produces. Nothing else is in it: no playfield, no plastics, no depth test —
/// the point is a picture of where the lamp light is, so that a translucent
/// surface can look up how much of it is arriving underneath itself.
/// `Renderer::DrawBulbLightBuffer`, `Renderer.cpp:1484`.
///
/// It runs even with nothing lit, because the pass is also what clears the
/// buffer, and a stale one would leave last frame's lamps glowing through this
/// frame's plastics.
pub fn draw_lights_only(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    pipeline: &TablePipeline,
    lights: &Lights,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("vpw-transmitted-light"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    lights.draw_flat(&mut pass, &pipeline.light_frame_bind_group);
}

/// The table upside down, into the reflection probe.
///
/// A camera flipped through the playfield, and a clip plane that throws away
/// everything at or below it. Because the camera is the mirror of the real one,
/// a point on the playfield finds its own reflection at its own place on the
/// screen — which is why the material shader can sample this with nothing but a
/// screen-space lookup.
///
/// The lights are left out. The original can be told to
/// (`m_disableLightReflection`, `RenderProbe.cpp:411`) and here it is not a
/// choice: a light's halo is a flat piece of geometry lying on the playfield,
/// and mirrored it is in exactly the same place, so it would double every lamp
/// on the table rather than reflecting anything.
/// `RenderProbe::DoRenderReflectionProbe`, `RenderProbe.cpp:404`.
pub fn draw_reflection(
    encoder: &mut wgpu::CommandEncoder,
    color: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    depth: &wgpu::TextureView,
    pipeline: &TablePipeline,
    scene: &GpuScene,
    dynamic: Option<&DynamicParts>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("vpw-reflection"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
            depth_slice: None,
            resolve_target: resolve,
            ops: wgpu::Operations {
                // Black, not the table's background: what is not reflected must
                // add nothing, and this is added rather than blended.
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                // The multisamples are working data: once resolved they are
                // done, and discarding them is what keeps them on-tile on the
                // GPUs where that matters.
                store: store_for(resolve),
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });

    pass.set_bind_group(0, &pipeline.mirror_bind_group, &[]);
    pass.set_pipeline(&pipeline.opaque);
    scene.draw_filtered(&mut pass, |b| !b.transparent);
    if let Some(d) = dynamic.filter(|d| d.any(false)) {
        pass.set_pipeline(&pipeline.dynamic_opaque);
        d.draw(&mut pass, false);
    }
    if scene.batches.iter().any(|b| b.transparent) {
        pass.set_pipeline(&pipeline.blended);
        scene.draw_filtered(&mut pass, |b| b.transparent);
    }
    if let Some(d) = dynamic.filter(|d| d.any(true)) {
        pass.set_pipeline(&pipeline.dynamic_blended);
        d.draw(&mut pass, true);
    }

    // The layers of light a baked table carries: added on top of the machine
    // once the machine is drawn, before the halos and the flashers, which is
    // where the original puts them too. See
    // [`vpw_table::geometry::Additive`].
    if let Some(d) = dynamic.filter(|d| d.any_additive()) {
        pass.set_pipeline(&pipeline.dynamic_additive);
        d.draw_additive(&mut pass);
    }
}

/// Draws the whole scene: the opaque ones first, then the transparent ones.
///
/// The two groups use different pipelines —the second blends and does not write
/// depth— but they share buffers and bind groups, so switching from one to the
/// other is the only thing that costs anything.
pub fn draw(
    encoder: &mut wgpu::CommandEncoder,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    pipeline: &TablePipeline,
    scene: &GpuScene,
    dynamic: Option<&DynamicParts>,
    lights: Option<&Lights>,
) {
    draw_full(
        encoder,
        color,
        None,
        depth,
        pipeline,
        scene,
        dynamic,
        lights,
        None,
        |_| true,
    );
}

/// The same, but drawing only the batches that pass the filter. Useful for
/// isolating who is covering what.
pub fn draw_filtered(
    encoder: &mut wgpu::CommandEncoder,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    pipeline: &TablePipeline,
    scene: &GpuScene,
    filter: impl Fn(&crate::scene::Batch) -> bool,
) {
    draw_full(
        encoder, color, None, depth, pipeline, scene, None, None, None, filter,
    );
}

/// The whole pass: opaque geometry, transparent geometry and lights.
///
/// The moving pieces go **inside** the same two groups and not in a pass of
/// their own. They have to: a flipper is opaque and has to take part in the
/// early-Z of the opaque geometry, and the ball is opaque and has to be able to
/// be covered by a ramp. Drawing them afterwards would paint the ball on top of
/// whatever is hiding it.
///
/// The flashers come after everything, the lights included. The original
/// sorts them among the other transparent draws by `depthBias - z`
/// (`RenderPass.cpp:118`), and a flasher stands *above* the playfield — on
/// Lord of the Rings the strobes sit between fifty and four hundred units up,
/// and a lamp's halo lies at a tenth of one — so for nearly every pair the
/// sort puts the lamp first and the flasher over it. Drawing the flashers before the lights instead lets a
/// bulb's modulating blend multiply a strobe that is not under it.
#[expect(
    clippy::too_many_arguments,
    reason = "one argument per stage of the pass"
)]
pub fn draw_full(
    encoder: &mut wgpu::CommandEncoder,
    color: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    depth: &wgpu::TextureView,
    pipeline: &TablePipeline,
    scene: &GpuScene,
    dynamic: Option<&DynamicParts>,
    lights: Option<&Lights>,
    flashers: Option<&Flashers>,
    filter: impl Fn(&crate::scene::Batch) -> bool,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("vpw-table"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
            depth_slice: None,
            resolve_target: resolve,
            ops: wgpu::Operations {
                // The pipeline's, not the constant: the backdrop wears the
                // room's tint, and the room is whichever map is installed.
                load: wgpu::LoadOp::Clear(pipeline.clear),
                // See `draw_reflection` for why the multisamples are discarded.
                store: store_for(resolve),
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });

    pass.set_bind_group(0, &pipeline.frame_bind_group, &[]);

    pass.set_pipeline(&pipeline.opaque);
    scene.draw_filtered(&mut pass, |b| !b.transparent && !b.culled && filter(b));
    if scene.batches.iter().any(|b| b.culled && filter(b)) {
        pass.set_pipeline(&pipeline.opaque_culled);
        scene.draw_filtered(&mut pass, |b| b.culled && filter(b));
    }

    if let Some(d) = dynamic.filter(|d| d.any(false)) {
        pass.set_pipeline(&pipeline.dynamic_opaque);
        d.draw(&mut pass, false);
    }

    if scene.batches.iter().any(|b| b.transparent && filter(b)) {
        pass.set_pipeline(&pipeline.blended);
        scene.draw_filtered(&mut pass, |b| b.transparent && filter(b));
    }

    if let Some(d) = dynamic.filter(|d| d.any(true)) {
        pass.set_pipeline(&pipeline.dynamic_blended);
        d.draw(&mut pass, true);
    }

    // The layers of light a baked table carries: added on top of the machine
    // once the machine is drawn, before the halos and the flashers, which is
    // where the original puts them too. See
    // [`vpw_table::geometry::Additive`].
    if let Some(d) = dynamic.filter(|d| d.any_additive()) {
        pass.set_pipeline(&pipeline.dynamic_additive);
        d.draw_additive(&mut pass);
    }

    // The lights go last: they add on top of what is already drawn.
    if let Some(l) = lights {
        l.draw(&mut pass, &pipeline.frame_bind_group);
    }

    // And the flashers over the lot; see above for why not before the lights.
    if let Some(f) = flashers {
        f.draw(&mut pass, &pipeline.light_frame_bind_group);
    }
}
