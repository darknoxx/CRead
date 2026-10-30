// Separable gaussian for the phosphor glow.
//
// No bright pass: the page is black, so blurring the frame already isolates the
// text. Two cheap passes at half resolution give a wider, softer halo than the
// handful of taps a single pass can afford — and it is the *radius* that makes
// a glow look like a glow instead of a fringe around every glyph.

struct Bloom {
    // One destination texel in uv space; the sampling step.
    texel: vec2<f32>,
    // Blur axis: (1,0) horizontal, (0,1) vertical.
    dir: vec2<f32>,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> bloom: Bloom;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let p = corners[idx];
    var out: VsOut;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var weights = array<f32, 5>(0.2270, 0.1945, 0.1216, 0.0540, 0.0162);
    var sum = textureSample(src, samp, in.uv).rgb * weights[0];
    var total = weights[0];
    for (var i = 1u; i < 5u; i = i + 1u) {
        let offset = bloom.dir * bloom.texel * f32(i);
        let w = weights[i];
        sum += textureSample(src, samp, in.uv + offset).rgb * w;
        sum += textureSample(src, samp, in.uv - offset).rgb * w;
        total += w * 2.0;
    }
    return vec4<f32>(sum / total, 1.0);
}
