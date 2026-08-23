
// Vertex stage for the static geometry of the table.
//
// Goes after `material.wgsl`, which declares `Frame`, `VsIn` and `VsOut`.

@vertex
fn vs_main(in : VsIn) -> VsOut {
    var out : VsOut;
    // The positions already arrive in world space: the transform of every
    // static part was baked at load time, so there is no model matrix to upload
    // and none to multiply per vertex.
    out.clip = frame.view_proj * vec4<f32>(in.pos, 1.0);
    out.world = in.pos;
    out.normal = in.normal;
    out.uv = in.uv;
    return out;
}
