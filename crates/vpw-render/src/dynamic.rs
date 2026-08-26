//! The pieces that move, on the GPU.
//!
//! The static scene bakes its transform once and never touches it again
//! ([`crate::scene`]). That is right for the three thousand parts of a table
//! that stand still and wrong for the dozen that do not: a flipper, a gate, a
//! spinner, a trigger, a bumper's ring, the plunger and the balls.
//!
//! Those go here. Each one keeps its vertices in a local frame, gets its own
//! model matrix in a uniform of its own, and gets its own draw call. Twelve
//! draw calls against a table's twenty-six does not move the needle, and it is
//! the only way a flipper can rotate without re-uploading its vertices.
//!
//! # How it is arranged
//!
//! One vertex buffer and one index buffer for **all** the moving pieces
//! together, same as the static scene: a part is a range of indices, and the
//! buffers are bound once for the whole pass. What changes per draw is group 1
//! (the material) and group 2 (the matrix).
//!
//! Balls are a special case: they all share one range of indices —the same
//! mesh, uploaded once— and differ only in their matrix. Slots for them are
//! reserved up front and hidden while unused, so that starting a multiball does
//! not allocate a buffer in the middle of a frame.

use crate::pipeline::TablePipeline;
use crate::scene::{GpuVertex, material_slot, table_sampler, white_texture};
use vpw_math::Mat4;
use vpw_table::animation::AnimatedPart;
use vpw_table::geometry::{Material, Mesh, Scene};
use wgpu::util::DeviceExt;

/// How many balls can be on the table at once.
///
/// The most crowded multiball in existence does not reach a dozen. Reserving
/// eight and hiding the unused ones costs eight matrices worth of memory and
/// saves allocating anything while the ball is in play.
pub const MAX_BALLS: usize = 8;

/// What the vertex stage needs of each piece. Mirrors `struct Model` in
/// `dynamic_vs.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuModel {
    model: [[f32; 4]; 4],
    /// Inverse transpose of the above, for the normals.
    normal: [[f32; 4]; 4],
}

impl GpuModel {
    fn new(m: Mat4) -> Self {
        Self {
            model: m.to_cols_array_2d(),
            normal: m.inverse().transpose().to_cols_array_2d(),
        }
    }
}

/// One moving piece: a range of indices, a material and a matrix.
struct Part {
    first_index: u32,
    index_count: u32,
    material: wgpu::BindGroup,
    model_buffer: wgpu::Buffer,
    model: wgpu::BindGroup,
    transparent: bool,
    /// Hidden pieces are skipped. Only the reserved ball slots use it.
    visible: bool,
    /// What was last written to `model_buffer`.
    ///
    /// Most of a table's moving parts stand still most of the time: measured on
    /// F-14, the median number that move between one frame and the next is
    /// **zero**, out of seventy-three. Writing them all anyway was four
    /// thousand pointless buffer writes a second, each one a trip out of wasm
    /// and into the GPU process. The lights already skip an unchanged write
    /// (`Lights::set_state`); this is the same thing for the parts.
    written: Mat4,
}

/// Everything that moves, ready to draw.
pub struct DynamicParts {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    parts: Vec<Part>,
    /// Where the ball slots start inside `parts`. The table's own pieces come
    /// first, so an index into `parts` is also an index into the
    /// `Vec<AnimatedPart>` it was built from.
    first_ball: usize,
}

impl DynamicParts {
    /// The layout of group 2: one matrix per piece.
    pub fn model_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-model-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        })
    }

    /// Uploads the table's moving pieces plus the ball slots.
    ///
    /// `scene` is only used to resolve materials and textures by name, which is
    /// why the balls —which are in no `.vpx`— carry their own.
    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &TablePipeline,
        scene: &Scene,
        animated: &[AnimatedPart],
        ball: &Mesh,
        ball_material: &Material,
    ) -> Self {
        let sampler = table_sampler(device);
        let white = white_texture(device, queue);

        let mut vertices: Vec<GpuVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut parts = Vec::with_capacity(animated.len() + MAX_BALLS);

        // A piece's vertices go in **unbaked**: the whole point is that its
        // transform is alive.
        let push_mesh = |m: &Mesh, vertices: &mut Vec<GpuVertex>, indices: &mut Vec<u32>| {
            let first_index = indices.len() as u32;
            let base = vertices.len() as u32;
            vertices.extend(m.vertices.iter().map(|v| GpuVertex {
                pos: v.pos,
                normal: v.normal,
                uv: v.uv,
            }));
            indices.extend(m.indices.iter().map(|i| i + base));
            (first_index, indices.len() as u32 - first_index)
        };

        let make_part = |device: &wgpu::Device,
                         first_index: u32,
                         index_count: u32,
                         material: Option<&Material>,
                         image: Option<&vpw_table::geometry::Image>,
                         transform: Mat4,
                         visible: bool| {
            let slot = material_slot(
                device,
                queue,
                &pipeline.material_layout,
                &sampler,
                &white,
                material,
                image,
                false,
            );
            let with_alpha = image.is_some_and(|i| i.has_alpha);
            let transparent = material.is_some_and(|m| m.is_transparent(with_alpha));

            let model_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-model"),
                contents: bytemuck::bytes_of(&GpuModel::new(transform)),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            Part {
                first_index,
                index_count,
                written: transform,
                material: slot.bind_group,
                model: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vpw-model-bg"),
                    layout: &pipeline.model_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: model_buffer.as_entire_binding(),
                    }],
                }),
                model_buffer,
                transparent,
                visible,
            }
        };

        for part in animated {
            let (first_index, index_count) = push_mesh(&part.mesh, &mut vertices, &mut indices);
            parts.push(make_part(
                device,
                first_index,
                index_count,
                scene.material(&part.mesh.material),
                scene.image(&part.mesh.image),
                part.mesh.transform,
                true,
            ));
        }

        let first_ball = parts.len();
        let (ball_first, ball_count) = push_mesh(ball, &mut vertices, &mut indices);
        for _ in 0..MAX_BALLS {
            parts.push(make_part(
                device,
                ball_first,
                ball_count,
                Some(ball_material),
                None,
                Mat4::IDENTITY,
                false,
            ));
        }

        Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-dynamic-vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-dynamic-indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            parts,
            first_ball,
        }
    }

    /// How many table pieces there are, not counting the ball slots.
    pub fn table_parts(&self) -> usize {
        self.first_ball
    }

    /// Moves one of the table's pieces.
    pub fn set_part_transform(&mut self, queue: &wgpu::Queue, index: usize, m: Mat4) {
        self.write(queue, index, Some(m));
    }

    /// Shows or hides one piece.
    ///
    /// A script does this constantly, and on a table with primitive flippers it
    /// is not decoration: the built-in flipper is hidden and a primitive bat is
    /// shown in its place, so drawing both gives one flipper that swings and
    /// another that does not.
    pub fn set_part_visible(&mut self, index: usize, visible: bool) {
        if let Some(part) = self.parts.get_mut(index) {
            part.visible = visible;
        }
    }

    /// Moves ball `index`, or hides it with `None`.
    pub fn set_ball_transform(&mut self, queue: &wgpu::Queue, index: usize, m: Option<Mat4>) {
        if index >= MAX_BALLS {
            return;
        }
        self.write(queue, self.first_ball + index, m);
    }

    fn write(&mut self, queue: &wgpu::Queue, index: usize, m: Option<Mat4>) {
        let Some(part) = self.parts.get_mut(index) else {
            return;
        };
        match m {
            Some(m) => {
                part.visible = true;
                // An exact comparison on purpose: the question is not whether
                // the piece moved enough to see, it is whether the bytes about
                // to be sent are the ones already there.
                if m != part.written {
                    part.written = m;
                    queue.write_buffer(
                        &part.model_buffer,
                        0,
                        bytemuck::bytes_of(&GpuModel::new(m)),
                    );
                }
            }
            None => part.visible = false,
        }
    }

    /// Whether anything is going to be drawn in the given pass.
    pub fn any(&self, transparent: bool) -> bool {
        self.parts
            .iter()
            .any(|p| p.visible && p.transparent == transparent && p.index_count > 0)
    }

    /// Emits the draws for one of the two passes. Group 0 has to be bound
    /// already; the pipeline too.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, transparent: bool) {
        if !self.any(transparent) {
            return;
        }
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        for p in &self.parts {
            if !p.visible || p.transparent != transparent || p.index_count == 0 {
                continue;
            }
            pass.set_bind_group(1, &p.material, &[]);
            pass.set_bind_group(2, &p.model, &[]);
            pass.draw_indexed(p.first_index..p.first_index + p.index_count, 0, 0..1);
        }
    }
}
