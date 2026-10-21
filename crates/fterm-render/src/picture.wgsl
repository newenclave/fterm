// One quad per picture: its texture, scaled with a linear filter.

struct Uniforms {
    // Window size in pixels (the same buffer as the grid uses).
    viewport: vec2<f32>,
    mask_atlas_size: vec2<f32>,
    color_atlas_size: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var picture_sampler: sampler;

// The same layout as `Instance` in frame.rs; `uv` is 0..1 here.
struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) kind: u32,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, inst: Instance) -> VertexOut {
    let corner = vec2<f32>(f32(vertex & 1u), f32(vertex >> 1u));
    let pixel = inst.rect.xy + corner * inst.rect.zw;
    var out: VertexOut;
    out.position = vec4<f32>(
        pixel.x / u.viewport.x * 2.0 - 1.0,
        1.0 - pixel.y / u.viewport.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = inst.uv.xy + corner * inst.uv.zw;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(picture, picture_sampler, in.uv);
}
