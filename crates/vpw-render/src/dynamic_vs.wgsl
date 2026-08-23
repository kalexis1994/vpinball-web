
// Vertex stage for the pieces that move.
//
// Goes after `material.wgsl`, which declares `Frame`, `VsIn` and `VsOut`.
//
// A flipper, a gate, a spinner, a trigger, a bumper's ring, a plunger and the
// balls: about a dozen pieces against a table's three thousand. Each one gets
// its own matrix in group 2 and its own draw call, which is why its vertices
// can stay in a local frame instead of being baked.

struct Model {
    // Local space to world space.
    model  : mat4x4<f32>,
    // The inverse transpose of the above. It is not the same matrix: a plunger
    // shaft is stretched along one axis only, and under a non-uniform scale the
    // normals do not follow the positions. Getting this wrong lights the rod up
    // as if it were fatter than it is.
    normal : mat4x4<f32>,
};

@group(2) @binding(0) var<uniform> piece : Model;

@vertex
fn vs_main(in : VsIn) -> VsOut {
    var out : VsOut;
    let world = piece.model * vec4<f32>(in.pos, 1.0);
    out.clip = frame.view_proj * world;
    out.world = world.xyz;
    out.normal = (piece.normal * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    return out;
}
