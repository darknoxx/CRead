// CRT post-processing. Everything here works on the *emulated* pixel grid, so
// scanlines and glow stay crisp no matter what the display resolution is.

// Layout must match `CrtUniforms` in gpu.rs byte for byte; there is a
// compile-time size check on the Rust side.
struct Crt {
    // Emulated raster size in pixels.
    vres: vec2<f32>,
    // Device pixels per emulated pixel.
    scale: f32,
    scanline: f32,
    bloom: f32,
    brightness: f32,
    // Seconds since start, and the drift speed in emulated rows per second.
    time: f32,
    roll: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp_nearest: sampler;
@group(0) @binding(2) var samp_linear: sampler;
@group(0) @binding(3) var<uniform> crt: Crt;
@group(0) @binding(4) var bloom_tex: texture_2d<f32>;

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

    // Phosphor glow, blurred separately at half resolution so it can be wide.
    let glow = textureSample(bloom_tex, samp_linear, in.uv).rgb;
    var color = base + glow * crt.bloom;

    // Scanlines as a beam profile rather than a hard shutter. A real CRT has a
    // gaussian spot wider than the line pitch, so the gap is never black.
    // Three device pixels per emulated row is the minimum that can show a
    // gradient; at two it collapses to one darker row, which is all we can do.
    // A slow downward drift makes the raster feel alive; a still pattern reads
    // as a printed overlay. `roll` is in emulated rows per second.
    let drift = crt.time * crt.roll;
    var beam: f32;
    if (crt.scale >= 2.5) {
        // A cosine across the row: full brightness at the scanline centre,
        // falling off smoothly towards the gap. The dip depth is controlled by
        // `scanline`, so the look does not change with the scale factor.
        let d = fract(in.uv.y * vres.y + drift) - 0.5;
        beam = 1.0 - crt.scanline * (1.0 - cos(d * 3.14159265));
    } else {
        let odd = fract(floor(in.uv.y * vres.y + drift) * 0.5) * 2.0;
        beam = 1.0 - 0.5 * odd * crt.scanline;
    }
    color *= beam;

    // Gentle vignette.
    let d = in.uv - vec2<f32>(0.5);
    color *= 1.0 - dot(d, d) * 0.45;

    color *= crt.brightness;
    return vec4<f32>(color, 1.0);
}
