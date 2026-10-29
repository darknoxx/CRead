// CRT post-processing. Everything here works on the *emulated* pixel grid, so
// scanlines and glow stay crisp no matter what the display resolution is.

// Layout must match `CrtUniforms` in gpu.rs byte for byte; there is a
// compile-time size check on the Rust side.
struct Crt {
    // Emulated raster size in pixels.
    vres: vec2<f32>,
    scanline: f32,
    bloom: f32,
    brightness: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp_nearest: sampler;
@group(0) @binding(2) var samp_linear: sampler;
@group(0) @binding(3) var<uniform> crt: Crt;

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
    // Clip space is y-up, our framebuffer is y-down.
    out.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vres = crt.vres;
    // Snap to the emulated pixel this fragment belongs to, so the effect maths
    // never straddles a block boundary.
    let vp = floor(in.uv * vres);
    let texel = (vp + vec2<f32>(0.5)) / vres;
    let base = textureSample(src, samp_nearest, texel).rgb;

    // Phosphor bleed. The linear sampler turns the 2x2 texel average into a
    // cheap blur; we only add it where a neighbour is *brighter* than this
    // pixel, so flat backgrounds stay flat instead of washing out.
    let px = 1.0 / vres;
    var blur = vec3<f32>(0.0);
    blur += textureSample(src, samp_linear, texel + vec2<f32>(-1.5,  0.0) * px).rgb;
    blur += textureSample(src, samp_linear, texel + vec2<f32>( 1.5,  0.0) * px).rgb;
    blur += textureSample(src, samp_linear, texel + vec2<f32>( 0.0, -1.5) * px).rgb;
    blur += textureSample(src, samp_linear, texel + vec2<f32>( 0.0,  1.5) * px).rgb;
    blur += textureSample(src, samp_linear, texel + vec2<f32>(-2.5, -2.5) * px).rgb;
    blur += textureSample(src, samp_linear, texel + vec2<f32>( 2.5, -2.5) * px).rgb;
    blur += textureSample(src, samp_linear, texel + vec2<f32>(-2.5,  2.5) * px).rgb;
    blur += textureSample(src, samp_linear, texel + vec2<f32>( 2.5,  2.5) * px).rgb;
    blur *= 0.125;

    let bleed = max(luma(blur) - luma(base), 0.0);
    var color = base + blur * bleed * crt.bloom * 1.6;

    // Scanlines: darken every other emulated row. `scale` physical pixels per
    // emulated pixel means each bar is exactly `scale` pixels tall.
    let odd = fract(vp.y * 0.5) * 2.0;
    color *= 1.0 - crt.scanline * 0.55 * odd;

    // Gentle vignette.
    let d = in.uv - vec2<f32>(0.5);
    color *= 1.0 - dot(d, d) * 0.45;

    color *= crt.brightness;
    return vec4<f32>(color, 1.0);
}
