// One instanced quad per background, glyph, line, or cursor part.

struct Uniforms {
    // Window size in pixels.
    viewport: vec2<f32>,
    // Atlas texture size in pixels.
    atlas_size: vec2<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

// Must match `Instance` in frame.rs.
struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) kind: u32,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, inst: Instance) -> VertexOut {
    // Triangle strip: (0,0) (1,0) (0,1) (1,1).
    let corner = vec2<f32>(f32(vertex & 1u), f32(vertex >> 1u));
    let pixel = inst.rect.xy + corner * inst.rect.zw;
    var out: VertexOut;
    out.position = vec4<f32>(
        pixel.x / u.viewport.x * 2.0 - 1.0,
        1.0 - pixel.y / u.viewport.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = (inst.uv.xy + corner * inst.uv.zw) / u.atlas_size;
    out.color = inst.color;
    out.kind = inst.kind;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if in.kind == 1u {
        // `textureSampleLevel` is allowed in non-uniform control flow.
        let alpha = textureSampleLevel(atlas, atlas_sampler, in.uv, 0.0).r;
        return vec4<f32>(in.color.rgb, in.color.a * alpha);
    }
    return in.color;
}
