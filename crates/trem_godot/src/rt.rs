//! Ray-traced effects of the enhanced graphics mode (port addition).
//!
//! A `CompositorEffect` that runs before the transparent pass. The map's
//! opaque triangles (built by `bspview::BspRender` with the light map
//! sampled at their vertices) are traced per pixel for:
//!
//! * ray-traced ambient occlusion;
//! * reflections on glossy surfaces (roughness from the normal/roughness
//!   buffer), shaded with the baked radiance of the surfaces they hit;
//! * in path-tracing mode, a two-bounce diffuse path per pixel (multi-bounce
//!   indirect light and colour bleeding on top of the light maps).
//!
//! The terms are accumulated over frames with camera reprojection, then a
//! depth / normal aware blur applies them to the HDR colour buffer.
//!
//! Two backends, picked from the GPU at startup (`gfx::GpuCaps`):
//!
//! * **hardware**: a Vulkan ray tracing pipeline (raygen / miss / closest
//!   hit) over a BLAS / TLAS: NVIDIA RTX, AMD Radeon RX 6000+ (ray
//!   accelerators), Intel Arc;
//! * **software**: the same shader as a compute shader walking a BVH built
//!   here on the CPU, at half resolution: any GPU with compute shaders
//!   (older Radeons, GTX cards, D3D12), so every Forward+ system gets the
//!   effects. A shader that fails to compile disables the effect with a
//!   console message instead of crashing.

use godot::classes::compositor_effect::EffectCallbackType;
use godot::classes::rendering_device::{
    AccelerationStructureFlagBits, AccelerationStructureGeometryFlagBits, AccelerationStructureInstanceFlagBits, BufferCreationBits, DataFormat, IndexBufferFormat, SamplerFilter, ShaderStage, TextureType, TextureUsageBits, UniformType,
};
use godot::classes::{
    CompositorEffect, ICompositorEffect, RdAccelerationStructureGeometry, RdAccelerationStructureInstance, RdHitGroup, RdPipelineShader, RdSamplerState, RdShaderSource, RdTextureFormat, RdTextureView, RdUniform, RenderData, RenderSceneBuffersRd, RenderingDevice, RenderingServer,
    UniformSetCacheRd,
};
use godot::prelude::*;

use crate::bspview::RtGeometry;
use crate::gfx::{RtBackend, RtMode};

/// Bindings 1..13, shared by both backends (binding 0 is the TLAS or the BVH).
const BINDINGS: &str = r#"
layout(set = 0, binding = 1) uniform sampler2D depth_tex;
layout(set = 0, binding = 2) uniform sampler2D nr_tex;
layout(set = 0, binding = 3, std430) readonly buffer Idx { uint idx[]; };
layout(set = 0, binding = 4, std430) readonly buffer Pos { vec4 pos[]; };
layout(set = 0, binding = 5, std430) readonly buffer Rad { vec4 rad[]; };
layout(set = 0, binding = 6, std430) readonly buffer Alb { vec4 alb[]; };
layout(set = 0, binding = 7) uniform sampler2D hist_gi;
layout(set = 0, binding = 8) uniform sampler2D hist_refl;
layout(set = 0, binding = 9) uniform sampler2D hist_z;
layout(rgba16f, set = 0, binding = 10) uniform restrict writeonly image2D out_gi;
layout(rgba16f, set = 0, binding = 11) uniform restrict writeonly image2D out_refl;
layout(r32f, set = 0, binding = 12) uniform restrict writeonly image2D out_z;
layout(set = 0, binding = 13, std140) uniform Params {
    mat4 proj;
    mat4 inv_proj;
    mat4 cam;        // camera -> world
    mat4 prev_view;  // world -> previous camera
    mat4 prev_proj;
    vec4 sky;        // rgb sky radiance, a = mode (1 ray tracing, 2 path tracing)
    vec4 misc;       // x frame, y near, z far, w history valid
    vec4 size;       // xy frame size, zw 1 / size
    vec4 strength;   // x ao radius (m), y ao strength, z gi strength, w reflection strength
    vec4 ext;        // x frame pixels per accumulation pixel, yz accumulation size
    vec4 debug;      // x: 0 off, 1 ao, 2 indirect, 3 reflections
} P;
struct Hit { bool hit; float t; uint prim; vec2 bary; };
"#;

const PRELUDE_HW: &str = r#"
#version 460
#extension GL_EXT_ray_tracing : require
layout(set = 0, binding = 0) uniform accelerationStructureEXT tlas;
struct Payload { float t; uint prim; vec2 bary; uint hit; };
"#;

const PRELUDE_SW: &str = r#"
#version 450
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;
// the BVH: per node two vec4s, (min.xyz, first / left), (max.xyz, count)
layout(set = 0, binding = 0, std430) readonly buffer Bvh { vec4 nodes[]; };
"#;

const MISS: &str = r#"
layout(location = 0) rayPayloadInEXT Payload pl;
void main() {
    pl.hit = 0u;
}
"#;

const CLOSEST_HIT: &str = r#"
layout(location = 0) rayPayloadInEXT Payload pl;
hitAttributeEXT vec2 attribs;
void main() {
    pl.hit = 1u;
    pl.t = gl_HitTEXT;
    pl.prim = uint(gl_PrimitiveID);
    pl.bary = attribs;
}
"#;

const TRACE_HW: &str = r#"
layout(location = 0) rayPayloadEXT Payload pl;
Hit trace(vec3 o, vec3 d, float tmax, bool any_hit) {
    pl.hit = 0u;
    pl.t = tmax;
    pl.prim = 0u;
    pl.bary = vec2(0.0);
    uint flags = gl_RayFlagsOpaqueEXT | (any_hit ? gl_RayFlagsTerminateOnFirstHitEXT : 0u);
    traceRayEXT(tlas, flags, 0xFF, 0, 0, 0, o, 0.0, d, tmax, 0);
    Hit h;
    h.hit = pl.hit != 0u;
    h.t = h.hit ? pl.t : tmax;
    h.prim = pl.prim;
    h.bary = pl.bary;
    return h;
}
"#;

const TRACE_SW: &str = r#"
bool slab(vec3 o, vec3 inv, vec3 bmin, vec3 bmax, float tmax) {
    vec3 t0 = (bmin - o) * inv;
    vec3 t1 = (bmax - o) * inv;
    vec3 lo = min(t0, t1);
    vec3 hi = max(t0, t1);
    float tn = max(max(lo.x, lo.y), max(lo.z, 0.0));
    float tf = min(min(hi.x, hi.y), min(hi.z, tmax));
    return tn <= tf;
}
Hit trace(vec3 o, vec3 d, float tmax, bool any_hit) {
    Hit h;
    h.hit = false;
    h.t = tmax;
    h.prim = 0u;
    h.bary = vec2(0.0);
    vec3 dd = mix(d, vec3(1e-7), lessThan(abs(d), vec3(1e-7)));
    vec3 inv = 1.0 / dd;
    uint stack[40];
    int sp = 0;
    stack[sp++] = 0u;
    while (sp > 0) {
        uint n = stack[--sp];
        vec4 a = nodes[2u * n];
        vec4 b = nodes[2u * n + 1u];
        if (!slab(o, inv, a.xyz, b.xyz, h.t)) {
            continue;
        }
        uint first = floatBitsToUint(a.w);
        uint count = floatBitsToUint(b.w);
        if (count > 0u) {
            for (uint t = first; t < first + count; t++) {
                vec3 v0 = pos[idx[3u * t]].xyz;
                vec3 e1 = pos[idx[3u * t + 1u]].xyz - v0;
                vec3 e2 = pos[idx[3u * t + 2u]].xyz - v0;
                vec3 p = cross(d, e2);
                float det = dot(e1, p);
                if (abs(det) < 1e-9) { continue; }
                float id = 1.0 / det;
                vec3 s = o - v0;
                float u = dot(s, p) * id;
                if (u < 0.0 || u > 1.0) { continue; }
                vec3 q = cross(s, e1);
                float v = dot(d, q) * id;
                if (v < 0.0 || u + v > 1.0) { continue; }
                float tt = dot(e2, q) * id;
                if (tt > 1e-4 && tt < h.t) {
                    h.hit = true;
                    h.t = tt;
                    h.prim = t;
                    h.bary = vec2(u, v);
                    if (any_hit) { return h; }
                }
            }
        } else if (sp < 38) {
            stack[sp++] = first;
            stack[sp++] = n + 1u;
        }
    }
    return h;
}
"#;

const BODY_HELPERS: &str = r#"
uint hash(uint x) {
    x ^= x >> 16; x *= 0x7feb352dU; x ^= x >> 15; x *= 0x846ca68bU; x ^= x >> 16;
    return x;
}
float rnd(inout uint s) {
    s = hash(s);
    return float(s & 0x00ffffffU) / 16777216.0;
}
vec3 cosine_dir(vec3 n, inout uint s) {
    float u = rnd(s), v = rnd(s);
    float r = sqrt(u), phi = 6.2831853 * v;
    vec3 t = normalize(abs(n.y) < 0.99 ? cross(n, vec3(0.0, 1.0, 0.0)) : cross(n, vec3(1.0, 0.0, 0.0)));
    vec3 b = cross(n, t);
    return normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(0.0, 1.0 - u)));
}
"#;

const BODY_ATTRS: &str = r#"
@@ATTRS@@"#;

const BODY_RUN: &str = r#"
@@MAIN@@"#;

const MAIN_HW: &str = "\nvoid main() {\n    run(ivec2(gl_LaunchIDEXT.xy));\n}\n";
const MAIN_SW: &str = "\nvoid main() {\n    run(ivec2(gl_GlobalInvocationID.xy));\n}\n";

const COMPOSE_SHADER: &str = r#"
#version 450
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;
layout(rgba16f, set = 0, binding = 0) uniform restrict image2D color_img;
layout(set = 0, binding = 1) uniform sampler2D gi_tex;
layout(set = 0, binding = 2) uniform sampler2D refl_tex;
layout(set = 0, binding = 3) uniform sampler2D z_tex;
layout(set = 0, binding = 4) uniform sampler2D nr_tex;
layout(set = 0, binding = 5, std140) uniform Params {
    mat4 proj;
    mat4 inv_proj;
    mat4 cam;
    mat4 prev_view;
    mat4 prev_proj;
    vec4 sky;
    vec4 misc;
    vec4 size;
    vec4 strength;
    vec4 ext;
    vec4 debug;
} P;

void main() {
    ivec2 px = ivec2(gl_GlobalInvocationID.xy);
    if (px.x >= int(P.size.x) || px.y >= int(P.size.y)) {
        return;
    }
    ivec2 hsize = ivec2(P.ext.yz);
    ivec2 hp = min(ivec2(vec2(px) / P.ext.x), hsize - 1);
    float z = texelFetch(z_tex, hp, 0).r;
    if (z <= 0.0) {
        return;
    }
    vec3 n0 = texelFetch(nr_tex, px, 0).xyz * 2.0 - 1.0;
    vec4 gsum = vec4(0.0);
    vec4 rsum = vec4(0.0);
    float wsum = 0.0, rws = 0.0;
    for (int dy = -2; dy <= 2; dy++) {
        for (int dx = -2; dx <= 2; dx++) {
            ivec2 q = clamp(hp + ivec2(dx, dy) * 2, ivec2(0), hsize - 1);
            float zq = texelFetch(z_tex, q, 0).r;
            if (zq <= 0.0) { continue; }
            ivec2 fq = min(ivec2((vec2(q) + 0.5) * P.ext.x), ivec2(P.size.xy) - 1);
            vec3 nq = texelFetch(nr_tex, fq, 0).xyz * 2.0 - 1.0;
            float w = exp(-abs(zq - z) / (0.03 * z + 0.02)) * pow(max(dot(n0, nq), 0.0), 16.0) * exp(-float(dx * dx + dy * dy) / 6.0);
            gsum += texelFetch(gi_tex, q, 0) * w;
            wsum += w;
            float wr = w * ((dx * dx + dy * dy) <= 2 ? 1.0 : 0.0);
            rsum += texelFetch(refl_tex, q, 0) * wr;
            rws += wr;
        }
    }
    vec4 g = wsum > 0.0 ? gsum / wsum : texelFetch(gi_tex, hp, 0);
    vec4 r = rws > 0.0 ? rsum / rws : texelFetch(refl_tex, hp, 0);
    vec4 c = imageLoad(color_img, px);
    vec3 rgb = c.rgb;
    float ao = mix(1.0, g.a, P.strength.y);
    rgb *= ao;
    rgb += g.rgb * P.strength.z * ao;
    float w = clamp(r.a * P.strength.w, 0.0, 1.0);
    rgb = rgb * (1.0 - 0.5 * w) + r.rgb * w;
    if (P.debug.x > 0.5 && P.debug.x < 1.5) { rgb = vec3(g.a); }
    if (P.debug.x > 1.5 && P.debug.x < 2.5) { rgb = g.rgb; }
    if (P.debug.x > 2.5) { rgb = r.rgb * r.a; }
    imageStore(color_img, px, vec4(rgb, c.a));
}
"#;

/// 5 mat4 + 6 vec4 (std140).
const PARAMS_BYTES: usize = (5 * 16 + 6 * 4) * 4;

/// The trace shader's sources for a backend.
fn trace_source(backend: RtBackend) -> String {
    let body = format!("{BODY_HELPERS}{}{}", BODY_ATTRS.replace("@@ATTRS@@", ATTRS), BODY_RUN.replace("@@MAIN@@", MAIN));
    match backend {
        RtBackend::Software => format!("{PRELUDE_SW}{BINDINGS}{TRACE_SW}{body}{MAIN_SW}"),
        _ => format!("{PRELUDE_HW}{BINDINGS}{TRACE_HW}{body}{MAIN_HW}"),
    }
}

const ATTRS: &str = r#"
vec3 attr_rad(Hit h) {
    uint i = h.prim * 3u;
    vec3 a = rad[idx[i]].rgb, b = rad[idx[i + 1u]].rgb, c = rad[idx[i + 2u]].rgb;
    return a * (1.0 - h.bary.x - h.bary.y) + b * h.bary.x + c * h.bary.y;
}
vec3 attr_alb(Hit h) {
    uint i = h.prim * 3u;
    vec3 a = alb[idx[i]].rgb, b = alb[idx[i + 1u]].rgb, c = alb[idx[i + 2u]].rgb;
    return a * (1.0 - h.bary.x - h.bary.y) + b * h.bary.x + c * h.bary.y;
}
vec3 attr_normal(Hit h, vec3 d) {
    uint i = h.prim * 3u;
    vec3 a = pos[idx[i]].xyz, b = pos[idx[i + 1u]].xyz, c = pos[idx[i + 2u]].xyz;
    vec3 n = normalize(cross(b - a, c - a));
    return dot(n, d) > 0.0 ? -n : n;
}

"#;

const MAIN: &str = r#"
// `px`: the accumulation buffers' pixel; `fpx`: the frame's (P.ext.x = 2
// for the software tracer's half resolution)
void run(ivec2 px) {
    if (px.x >= int(P.ext.y) || px.y >= int(P.ext.z)) {
        return;
    }
    ivec2 fpx = min(ivec2((vec2(px) + 0.5) * P.ext.x), ivec2(P.size.xy) - 1);
    vec2 uv = (vec2(fpx) + 0.5) * P.size.zw;
    float d = texelFetch(depth_tex, fpx, 0).r;
    if (d <= 0.0) {
        // the sky (reverse Z): nothing to trace
        imageStore(out_gi, px, vec4(0.0, 0.0, 0.0, 1.0));
        imageStore(out_refl, px, vec4(0.0));
        imageStore(out_z, px, vec4(0.0));
        return;
    }
    // the projection Godot hands over includes its depth correction
    // (Vulkan clip space, reverse Z, [0, 1] depth): unproject the raw depth
    vec4 vp = P.inv_proj * vec4(uv * 2.0 - 1.0, d, 1.0);
    vec3 view_pos = vp.xyz / vp.w;
    float lin = -view_pos.z;
    vec3 wpos = (P.cam * vec4(view_pos, 1.0)).xyz;
    vec3 cam_pos = P.cam[3].xyz;
    vec3 V = normalize(cam_pos - wpos);

    vec4 nr = texelFetch(nr_tex, fpx, 0);
    float rough = nr.w;
    // Godot marks dynamic objects (players, structures, the view weapon,
    // which is depth-hacked) in the roughness' high half: the tracer only
    // knows the static world, so they keep their raster lighting
    if (rough > 0.5) {
        imageStore(out_gi, px, vec4(0.0, 0.0, 0.0, 1.0));
        imageStore(out_refl, px, vec4(0.0));
        imageStore(out_z, px, vec4(-view_pos.z));
        return;
    }
    rough = clamp(rough / (127.0 / 255.0), 0.0, 1.0);
    vec3 nw = normalize(mat3(P.cam) * normalize(nr.xyz * 2.0 - 1.0));
    if (dot(nw, V) < 0.0) { nw = -nw; }

    uint seed = hash(uint(fpx.x) * 1973u + uint(fpx.y) * 9277u + uint(P.misc.x) * 26699u);
    vec3 o = wpos + nw * 0.03;

    // ambient occlusion
    float occ = 0.0;
    const int AO_RAYS = 2;
    for (int k = 0; k < AO_RAYS; k++) {
        vec3 dir = cosine_dir(nw, seed);
        Hit h = trace(o, dir, P.strength.x, true);
        if (h.hit) {
            float x = h.t / P.strength.x;
            occ += 1.0 - x * x;
        }
    }
    float ao = 1.0 - occ / float(AO_RAYS);
    ao *= ao;

    // multi-bounce indirect light (path tracing)
    vec3 gi = vec3(0.0);
    if (P.sky.a > 1.5) {
        // the surface's own albedo: the static world under this pixel
        vec3 to = wpos - cam_pos;
        float dist = length(to);
        Hit p = trace(cam_pos, to / dist, dist * 1.03 + 0.05, false);
        vec3 a_local = (p.hit && abs(p.t - dist) < dist * 0.03 + 0.05) ? attr_alb(p) : vec3(0.35);
        vec3 dir = cosine_dir(nw, seed);
        Hit h = trace(o, dir, 60.0, false);
        vec3 L;
        if (h.hit) {
            L = attr_rad(h);
            // second bounce from the hit surface
            vec3 hp = o + dir * h.t;
            vec3 hn = attr_normal(h, dir);
            vec3 d2 = cosine_dir(hn, seed);
            Hit h2 = trace(hp + hn * 0.03, d2, 60.0, false);
            L += attr_alb(h) * (h2.hit ? attr_rad(h2) : P.sky.rgb);
        } else {
            L = P.sky.rgb;
        }
        gi = a_local * L;
    }

    // reflections on glossy surfaces
    vec4 refl = vec4(0.0);
    if (rough < 0.65) {
        vec3 r = reflect(-V, nw);
        vec3 jitter = cosine_dir(r, seed);
        r = normalize(mix(r, jitter, rough * rough));
        if (dot(r, nw) < 0.0) { r = reflect(r, nw); }
        Hit h = trace(o, r, 80.0, false);
        vec3 c = h.hit ? attr_rad(h) : P.sky.rgb;
        float cos_v = clamp(dot(nw, V), 0.0, 1.0);
        float fres = 0.04 + 0.96 * pow(1.0 - cos_v, 5.0);
        float w = fres * (1.0 - rough) * (1.0 - rough);
        refl = vec4(c, w);
    }

    // temporal accumulation (camera reprojection; the world is static)
    vec4 gi_ao = vec4(gi, ao);
    vec4 pv = P.prev_view * vec4(wpos, 1.0);
    vec4 pc = P.prev_proj * vec4(pv.xyz, 1.0);
    vec2 puv = pc.xy / pc.w * 0.5 + 0.5;
    float blend = 1.0;
    if (P.misc.w > 0.5 && all(greaterThanEqual(puv, vec2(0.0))) && all(lessThanEqual(puv, vec2(1.0))) && pv.z < 0.0) {
        float pz = texture(hist_z, puv).r;
        if (pz > 0.0 && abs(pz - (-pv.z)) < 0.06 * (-pv.z) + 0.02) {
            blend = 0.12;
        }
    }
    if (blend < 1.0) {
        gi_ao = mix(texture(hist_gi, puv), gi_ao, blend);
        refl = mix(texture(hist_refl, puv), refl, max(blend, 0.25));
    }
    imageStore(out_gi, px, gi_ao);
    imageStore(out_refl, px, refl);
    imageStore(out_z, px, vec4(lin));
}
"#;

/// A bounding volume hierarchy over the triangles for the software tracer:
/// per node (min, first-or-right) and (max, count) as 8 floats; the
/// triangle order it needs (indices into the input triangles).
pub fn build_bvh(pos: &[[f32; 3]], idx: &[u32]) -> (Vec<[f32; 8]>, Vec<u32>) {
    let n = idx.len() / 3;
    let mut order: Vec<u32> = (0..n as u32).collect();
    let mut cent = Vec::with_capacity(n);
    let mut tmin = Vec::with_capacity(n);
    let mut tmax = Vec::with_capacity(n);
    for t in 0..n {
        let v = [pos[idx[3 * t] as usize], pos[idx[3 * t + 1] as usize], pos[idx[3 * t + 2] as usize]];
        let mut lo = v[0];
        let mut hi = v[0];
        for p in &v[1..] {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        tmin.push(lo);
        tmax.push(hi);
        cent.push([(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5]);
    }
    let mut nodes: Vec<[f32; 8]> = Vec::with_capacity(2 * n / 2 + 1);
    // (node index, start, end) work list; children are laid out depth first
    fn build(nodes: &mut Vec<[f32; 8]>, order: &mut [u32], start: usize, tmin: &[[f32; 3]], tmax: &[[f32; 3]], cent: &[[f32; 3]]) -> usize {
        let me = nodes.len();
        nodes.push([0.0; 8]);
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        let mut clo = [f32::MAX; 3];
        let mut chi = [f32::MIN; 3];
        for &t in order.iter() {
            let t = t as usize;
            for k in 0..3 {
                lo[k] = lo[k].min(tmin[t][k]);
                hi[k] = hi[k].max(tmax[t][k]);
                clo[k] = clo[k].min(cent[t][k]);
                chi[k] = chi[k].max(cent[t][k]);
            }
        }
        let count = order.len();
        let axis = (0..3).max_by(|&a, &b| (chi[a] - clo[a]).partial_cmp(&(chi[b] - clo[b])).unwrap()).unwrap();
        if count <= 4 || chi[axis] - clo[axis] < 1e-6 {
            nodes[me] = [lo[0], lo[1], lo[2], f32::from_bits(start as u32), hi[0], hi[1], hi[2], f32::from_bits(count as u32)];
            return me;
        }
        let mid = count / 2;
        order.select_nth_unstable_by(mid, |&a, &b| cent[a as usize][axis].partial_cmp(&cent[b as usize][axis]).unwrap_or(std::cmp::Ordering::Equal));
        let (l, r) = order.split_at_mut(mid);
        build(nodes, l, start, tmin, tmax, cent);
        let right = build(nodes, r, start + mid, tmin, tmax, cent);
        nodes[me] = [lo[0], lo[1], lo[2], f32::from_bits(right as u32), hi[0], hi[1], hi[2], f32::from_bits(0)];
        me
    }
    if n > 0 {
        build(&mut nodes, &mut order, 0, &tmin, &tmax, &cent);
    }
    (nodes, order)
}

/// GPU resources of the map geometry.
struct Geometry {
    vbuf: Rid,
    ibuf: Rid,
    idx: Rid,
    pos: Rid,
    rad: Rid,
    alb: Rid,
    blas: Rid,
    tlas: Rid,
    bvh: Rid,
}

/// The accumulation buffers (ping-pong) at the accumulation size.
struct History {
    size: Vector2i,
    gi: [Rid; 2],
    refl: [Rid; 2],
    z: [Rid; 2],
    cur: usize,
    valid: bool,
}

/// The trace pass: (shader, pipeline, hit SBT, SBT range); the software
/// backend has no SBT.
type TracePass = (Rid, Rid, Rid, i64);

#[derive(GodotClass)]
#[class(base=CompositorEffect)]
pub struct TremRtEffect {
    base: Base<CompositorEffect>,
    mode: RtMode,
    backend: RtBackend,
    sky: [f32; 3],
    debug: i32,
    pending: Option<RtGeometry>,
    /// the current map's geometry, kept for a backend switch (r_rtBackend)
    last_geom: Option<RtGeometry>,
    geom: Option<Geometry>,
    hist: Option<History>,
    trace: Option<TracePass>,
    compose: Option<(Rid, Rid)>,
    params: Option<Rid>,
    samplers: Option<(Rid, Rid)>,
    failed: bool,
    frame: u32,
    prev_view: Transform3D,
    prev_proj: Projection,
    pub status: String,
}

#[godot_api]
impl ICompositorEffect for TremRtEffect {
    fn init(base: Base<CompositorEffect>) -> Self {
        TremRtEffect {
            base,
            mode: RtMode::Off,
            backend: RtBackend::None,
            sky: [0.2, 0.22, 0.25],
            debug: 0,
            pending: None,
            last_geom: None,
            geom: None,
            hist: None,
            trace: None,
            compose: None,
            params: None,
            samplers: None,
            failed: false,
            frame: 0,
            prev_view: Transform3D::IDENTITY,
            prev_proj: Projection::IDENTITY,
            status: String::new(),
        }
    }

    fn render_callback(&mut self, _effect_callback_type: i32, render_data: Option<Gd<RenderData>>) {
        if self.mode == RtMode::Off || self.failed || self.backend == RtBackend::None {
            return;
        }
        let Some(rd) = RenderingServer::singleton().get_rendering_device() else { return };
        let Some(render_data) = render_data else { return };
        self.render(rd, render_data);
    }

    fn on_notification(&mut self, what: godot::classes::notify::ObjectNotification) {
        if what == godot::classes::notify::ObjectNotification::PREDELETE {
            self.free_all();
        }
    }
}

#[godot_api]
impl TremRtEffect {}

fn bytes_f32(v: &[f32]) -> PackedByteArray {
    let mut b = Vec::with_capacity(v.len() * 4);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    PackedByteArray::from(b.as_slice())
}
fn bytes_u32(v: &[u32]) -> PackedByteArray {
    let mut b = Vec::with_capacity(v.len() * 4);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    PackedByteArray::from(b.as_slice())
}
fn mat_proj(p: &Projection) -> [f32; 16] {
    let mut m = [0f32; 16];
    for (c, col) in p.cols.iter().enumerate() {
        m[c * 4] = col.x;
        m[c * 4 + 1] = col.y;
        m[c * 4 + 2] = col.z;
        m[c * 4 + 3] = col.w;
    }
    m
}
fn mat_xform(t: &Transform3D) -> [f32; 16] {
    let (a, b, c) = (t.basis.col_a(), t.basis.col_b(), t.basis.col_c());
    [a.x, a.y, a.z, 0.0, b.x, b.y, b.z, 0.0, c.x, c.y, c.z, 0.0, t.origin.x, t.origin.y, t.origin.z, 1.0]
}

impl TremRtEffect {
    pub fn create() -> Gd<Self> {
        Gd::from_init_fn(|base| <Self as ICompositorEffect>::init(base))
    }

    /// The backend (from `GpuCaps`, or r_rtBackend): a change rebuilds
    /// the pipelines and the geometry.
    pub fn set_backend(&mut self, b: RtBackend) {
        if b == self.backend {
            return;
        }
        self.free_all();
        self.backend = b;
        self.failed = false;
        self.status.clear();
        if self.pending.is_none() {
            self.pending = self.last_geom.clone();
        }
    }

    /// Configure the effect: `mode` Off disables it.
    pub fn set_mode(&mut self, mode: RtMode) {
        if mode != self.mode {
            self.mode = mode;
            if let Some(h) = self.hist.as_mut() {
                h.valid = false;
            }
        }
        let on = mode != RtMode::Off && !self.failed && self.backend != RtBackend::None;
        let mut base = self.base_mut();
        base.set_enabled(on);
        base.set_effect_callback_type(EffectCallbackType::PRE_TRANSPARENT);
        base.set_needs_normal_roughness(true);
        base.set_access_resolved_depth(true);
        base.set_access_resolved_color(true);
    }
    pub fn set_sky(&mut self, rgb: [f32; 3]) {
        self.sky = rgb;
    }
    pub fn set_debug(&mut self, d: i32) {
        self.debug = d;
    }
    /// A new map: its geometry is uploaded on the next frame.
    pub fn set_geometry(&mut self, g: RtGeometry) {
        self.last_geom = Some(g.clone());
        self.pending = Some(g);
        if let Some(h) = self.hist.as_mut() {
            h.valid = false;
        }
    }
    pub fn failed(&self) -> bool {
        self.failed
    }
    pub fn describe(&self) -> String {
        format!("{} backend, {} frames traced{}", self.backend.name(), self.frame, if self.status.is_empty() { String::new() } else { format!(", {}", self.status) })
    }

    fn fail(&mut self, why: String) {
        godot_warn!("trem: ray tracing disabled: {why}");
        self.status = format!("disabled: {why}");
        self.failed = true;
        self.base_mut().set_enabled(false);
    }

    fn compile(rd: &mut Gd<RenderingDevice>, code: &str, name: &str) -> Result<(Rid, Rid), String> {
        let mut src = RdShaderSource::new_gd();
        src.set_stage_source(ShaderStage::COMPUTE, code);
        let Some(spirv) = rd.shader_compile_spirv_from_source(&src) else { return Err(format!("{name}: no SPIR-V")) };
        let err = spirv.get_stage_compile_error(ShaderStage::COMPUTE).to_string();
        if !err.is_empty() {
            return Err(format!("{name}: {err}"));
        }
        let shader = rd.shader_create_from_spirv_ex(&spirv).name(name).done();
        if !shader.is_valid() {
            return Err(format!("{name}: shader_create_from_spirv failed"));
        }
        let pipe = rd.compute_pipeline_create(shader);
        if !pipe.is_valid() {
            return Err(format!("{name}: compute_pipeline_create failed"));
        }
        Ok((shader, pipe))
    }

    fn compile_hw(rd: &mut Gd<RenderingDevice>) -> Result<TracePass, String> {
        let raygen_src = trace_source(RtBackend::Hardware);
        let mut src = RdShaderSource::new_gd();
        src.set_stage_source(ShaderStage::RAYGEN, &raygen_src);
        src.set_stage_source(ShaderStage::MISS, &format!("{PRELUDE_HW}{BINDINGS}{MISS}"));
        src.set_stage_source(ShaderStage::CLOSEST_HIT, &format!("{PRELUDE_HW}{BINDINGS}{CLOSEST_HIT}"));
        let Some(spirv) = rd.shader_compile_spirv_from_source(&src) else { return Err("trace: no SPIR-V".into()) };
        for st in [ShaderStage::RAYGEN, ShaderStage::MISS, ShaderStage::CLOSEST_HIT] {
            let err = spirv.get_stage_compile_error(st).to_string();
            if !err.is_empty() {
                return Err(format!("trace {st:?}: {err}"));
            }
        }
        let shader = rd.shader_create_from_spirv_ex(&spirv).name("trem_rt_trace").done();
        if !shader.is_valid() {
            return Err("trace: shader_create_from_spirv failed".into());
        }
        let ps = || {
            let mut p = RdPipelineShader::new_gd();
            p.set_shader(shader);
            p
        };
        let mut raygen: Array<Gd<RdPipelineShader>> = Array::new();
        raygen.push(&ps());
        let mut miss: Array<Gd<RdPipelineShader>> = Array::new();
        miss.push(&ps());
        let mut hg = RdHitGroup::new_gd();
        hg.set_closest_hit_shader(&ps());
        let mut groups: Array<Gd<RdHitGroup>> = Array::new();
        groups.push(&hg);
        let pipe = rd.raytracing_pipeline_create(&raygen, &miss, &groups, 1);
        if !pipe.is_valid() {
            return Err("raytracing_pipeline_create failed".into());
        }
        let sbt = rd.hit_sbt_create(pipe, 1);
        if !sbt.is_valid() {
            return Err("hit_sbt_create failed".into());
        }
        let range = rd.hit_sbt_range_alloc(sbt, 1);
        if range == 0 || rd.hit_sbt_range_update(sbt, range, 0, &PackedInt32Array::from(&[0i32][..])) != godot::global::Error::OK {
            return Err("hit SBT range failed".into());
        }
        Ok((shader, pipe, sbt, range))
    }

    fn ensure_pipelines(&mut self, rd: &mut Gd<RenderingDevice>) -> bool {
        if self.trace.is_some() {
            return true;
        }
        let trace = match self.backend {
            RtBackend::Hardware => Self::compile_hw(rd),
            _ => Self::compile(rd, &trace_source(RtBackend::Software), "trem_rt_trace_sw").map(|(s, p)| (s, p, Rid::Invalid, 0)),
        };
        let r = trace.and_then(|t| Self::compile(rd, COMPOSE_SHADER, "trem_rt_compose").map(|c| (t, c)));
        match r {
            Ok((t, c)) => {
                self.trace = Some(t);
                self.compose = Some(c);
                let params = rd.uniform_buffer_create(PARAMS_BYTES as u32);
                self.params = Some(params);
                let mut s = RdSamplerState::new_gd();
                s.set_min_filter(SamplerFilter::NEAREST);
                s.set_mag_filter(SamplerFilter::NEAREST);
                let nearest = rd.sampler_create(&s);
                let mut s = RdSamplerState::new_gd();
                s.set_min_filter(SamplerFilter::LINEAR);
                s.set_mag_filter(SamplerFilter::LINEAR);
                let linear = rd.sampler_create(&s);
                self.samplers = Some((nearest, linear));
                self.status = "running".into();
                true
            }
            Err(e) => {
                self.fail(e);
                false
            }
        }
    }

    /// Every GPU resource of the effect (PREDELETE).
    fn free_all(&mut self) {
        let Some(mut rd) = RenderingServer::singleton().get_rendering_device() else { return };
        self.free_geometry(&mut rd);
        if let Some(h) = self.hist.take() {
            for r in h.gi.iter().chain(h.refl.iter()).chain(h.z.iter()) {
                rd.free_rid(*r);
            }
        }
        if let Some((shader, pipe, sbt, _)) = self.trace.take() {
            if sbt.is_valid() {
                rd.free_rid(sbt);
            }
            rd.free_rid(pipe);
            rd.free_rid(shader);
        }
        if let Some((shader, pipe)) = self.compose.take() {
            rd.free_rid(pipe);
            rd.free_rid(shader);
        }
        if let Some(p) = self.params.take() {
            rd.free_rid(p);
        }
        if let Some((a, b)) = self.samplers.take() {
            rd.free_rid(a);
            rd.free_rid(b);
        }
    }

    fn free_geometry(&mut self, rd: &mut Gd<RenderingDevice>) {
        if let Some(g) = self.geom.take() {
            for r in [g.tlas, g.blas, g.vbuf, g.ibuf, g.idx, g.pos, g.rad, g.alb, g.bvh] {
                if r.is_valid() {
                    rd.free_rid(r);
                }
            }
        }
    }

    fn upload(&mut self, rd: &mut Gd<RenderingDevice>, mut g: RtGeometry) {
        self.free_geometry(rd);
        if g.idx.is_empty() {
            return;
        }
        let sb = |rd: &mut Gd<RenderingDevice>, b: PackedByteArray| rd.storage_buffer_create_ex(b.len() as u32).data(&b).done();
        // the software backend's BVH (the triangles reordered to match)
        let mut bvh = Rid::Invalid;
        if self.backend == RtBackend::Software {
            let t0 = std::time::Instant::now();
            let (nodes, order) = build_bvh(&g.pos, &g.idx);
            g.idx = order.iter().flat_map(|&t| [g.idx[3 * t as usize], g.idx[3 * t as usize + 1], g.idx[3 * t as usize + 2]]).collect();
            let flat: Vec<f32> = nodes.iter().flat_map(|n| *n).collect();
            bvh = sb(rd, bytes_f32(&flat));
            godot_print!("trem: ray tracing BVH: {} nodes in {} ms", nodes.len(), t0.elapsed().as_millis());
        }
        let pos4: Vec<f32> = g.pos.iter().flat_map(|p| [p[0], p[1], p[2], 1.0]).collect();
        let rad: Vec<f32> = g.rad.iter().flat_map(|p| *p).collect();
        let alb: Vec<f32> = g.alb.iter().flat_map(|p| *p).collect();
        let ib = bytes_u32(&g.idx);
        let idx = sb(rd, ib.clone());
        let pos = sb(rd, bytes_f32(&pos4));
        let radb = sb(rd, bytes_f32(&rad));
        let albb = sb(rd, bytes_f32(&alb));
        let (mut vbuf, mut ibuf, mut blas, mut tlas) = (Rid::Invalid, Rid::Invalid, Rid::Invalid, Rid::Invalid);
        if self.backend == RtBackend::Hardware {
            let flat: Vec<f32> = g.pos.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
            let as_in = BufferCreationBits::DEVICE_ADDRESS_BIT | BufferCreationBits::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_BIT;
            let vb = bytes_f32(&flat);
            vbuf = rd.vertex_buffer_create_ex(vb.len() as u32).data(&vb).creation_bits(as_in).done();
            ibuf = rd.index_buffer_create_ex(g.idx.len() as u32, IndexBufferFormat::UINT32).data(&ib).creation_bits(as_in).done();
            let mut geo = RdAccelerationStructureGeometry::new_gd();
            geo.set_flags(AccelerationStructureGeometryFlagBits::OPAQUE_BIT);
            geo.set_vertex_buffer(vbuf);
            geo.set_vertex_offset(0);
            geo.set_vertex_stride(12);
            geo.set_vertex_count(g.pos.len() as u32);
            geo.set_vertex_format(DataFormat::R32G32B32_SFLOAT);
            geo.set_index_buffer(ibuf);
            geo.set_index_offset(0);
            geo.set_index_count(g.idx.len() as u32);
            let mut geos = Array::new();
            geos.push(&geo);
            blas = rd.blas_create(&geos, AccelerationStructureFlagBits::PREFER_FAST_TRACE_BIT);
            if !blas.is_valid() || rd.blas_build(blas) != godot::global::Error::OK {
                self.fail("blas_build failed".into());
                return;
            }
            tlas = rd.tlas_create(1, AccelerationStructureFlagBits::PREFER_FAST_TRACE_BIT);
            let mut inst = RdAccelerationStructureInstance::new_gd();
            inst.set_transform(Transform3D::IDENTITY);
            inst.set_id(0);
            inst.set_mask(0xff);
            inst.set_hit_sbt_range(self.trace.map(|t| t.3).unwrap_or(0));
            inst.set_flags(AccelerationStructureInstanceFlagBits::TRIANGLE_FACING_CULL_DISABLE_BIT | AccelerationStructureInstanceFlagBits::FORCE_OPAQUE_BIT);
            inst.set_blas(blas);
            let mut insts = Array::new();
            insts.push(&inst);
            if !tlas.is_valid() || rd.tlas_build(tlas, &insts) != godot::global::Error::OK {
                self.fail("tlas_build failed".into());
                return;
            }
        }
        godot_print!("trem: ray tracing geometry: {} triangles ({})", g.idx.len() / 3, self.backend.name());
        self.geom = Some(Geometry { vbuf, ibuf, idx, pos, rad: radb, alb: albb, blas, tlas, bvh });
    }

    fn ensure_history(&mut self, rd: &mut Gd<RenderingDevice>, size: Vector2i) {
        if self.hist.as_ref().is_some_and(|h| h.size == size) {
            return;
        }
        if let Some(h) = self.hist.take() {
            for r in h.gi.iter().chain(h.refl.iter()).chain(h.z.iter()) {
                rd.free_rid(*r);
            }
        }
        let mk = |rd: &mut Gd<RenderingDevice>, fmt: DataFormat| {
            let mut tf = RdTextureFormat::new_gd();
            tf.set_texture_type(TextureType::TYPE_2D);
            tf.set_format(fmt);
            tf.set_width(size.x as u32);
            tf.set_height(size.y as u32);
            tf.set_usage_bits(TextureUsageBits::SAMPLING_BIT | TextureUsageBits::STORAGE_BIT | TextureUsageBits::CAN_UPDATE_BIT);
            rd.texture_create(&tf, &RdTextureView::new_gd())
        };
        let gi = [mk(rd, DataFormat::R16G16B16A16_SFLOAT), mk(rd, DataFormat::R16G16B16A16_SFLOAT)];
        let refl = [mk(rd, DataFormat::R16G16B16A16_SFLOAT), mk(rd, DataFormat::R16G16B16A16_SFLOAT)];
        let z = [mk(rd, DataFormat::R32_SFLOAT), mk(rd, DataFormat::R32_SFLOAT)];
        self.hist = Some(History { size, gi, refl, z, cur: 0, valid: false });
    }

    fn render(&mut self, mut rd: Gd<RenderingDevice>, render_data: Gd<RenderData>) {
        if !self.ensure_pipelines(&mut rd) {
            return;
        }
        if let Some(g) = self.pending.take() {
            self.upload(&mut rd, g);
        }
        if self.geom.is_none() {
            self.status = "waiting for map geometry".into();
            return;
        }
        let Some(buffers) = render_data.get_render_scene_buffers().and_then(|b| b.try_cast::<RenderSceneBuffersRd>().ok()) else {
            self.status = "no RenderSceneBuffersRD".into();
            return;
        };
        let Some(scene) = render_data.get_render_scene_data() else { return };
        let size = buffers.get_internal_size();
        if size.x < 8 || size.y < 8 {
            return;
        }
        if !buffers.has_texture("forward_clustered", "normal_roughness") {
            self.status = "no normal/roughness buffer (Forward+ only)".into();
            return;
        }
        let normal = buffers.get_texture("forward_clustered", "normal_roughness");
        let depth = buffers.get_depth_layer(0);
        let color = buffers.get_color_layer(0);
        // the software tracer works at half resolution
        let scale = if self.backend == RtBackend::Software { 2 } else { 1 };
        let hsize = Vector2i::new((size.x + scale - 1) / scale, (size.y + scale - 1) / scale);
        self.ensure_history(&mut rd, hsize);
        let (nearest, linear) = self.samplers.unwrap();
        let cam = scene.get_cam_transform();
        let proj = scene.get_view_projection(0);
        let (near, far) = (0.0f32, 0.0f32); // unused: the shader unprojects with the full matrix
        let hist = self.hist.as_mut().unwrap();
        let (prev, cur) = (hist.cur, 1 - hist.cur);
        let valid = hist.valid;
        let gi = hist.gi;
        let refl = hist.refl;
        let z = hist.z;
        // parameters
        let mut p: Vec<f32> = Vec::with_capacity(PARAMS_BYTES / 4);
        p.extend_from_slice(&mat_proj(&proj));
        p.extend_from_slice(&mat_proj(&proj.inverse()));
        p.extend_from_slice(&mat_xform(&cam));
        p.extend_from_slice(&mat_xform(&self.prev_view.affine_inverse()));
        p.extend_from_slice(&mat_proj(&self.prev_proj));
        p.extend_from_slice(&[self.sky[0], self.sky[1], self.sky[2], self.mode.as_int() as f32]);
        p.extend_from_slice(&[self.frame as f32, near, far, if valid { 1.0 } else { 0.0 }]);
        p.extend_from_slice(&[size.x as f32, size.y as f32, 1.0 / size.x as f32, 1.0 / size.y as f32]);
        let (gi_k, refl_k) = if self.mode == RtMode::Pathtrace { (1.0, 0.9) } else { (0.0, 0.9) };
        p.extend_from_slice(&[1.2, 0.85, gi_k, refl_k]);
        p.extend_from_slice(&[scale as f32, hsize.x as f32, hsize.y as f32, 0.0]);
        p.extend_from_slice(&[self.debug as f32, 0.0, 0.0, 0.0]);
        let params = self.params.unwrap();
        let pb = bytes_f32(&p);
        rd.buffer_update(params, 0, pb.len() as u32, &pb);
        let g = self.geom.as_ref().unwrap();
        let u = |ty: UniformType, binding: i32, ids: &[Rid]| {
            let mut un = RdUniform::new_gd();
            un.set_uniform_type(ty);
            un.set_binding(binding);
            for id in ids {
                un.add_id(*id);
            }
            un
        };
        let (trace_shader, trace_pipe, trace_sbt, _) = self.trace.unwrap();
        let mut uniforms: Array<Gd<RdUniform>> = Array::new();
        if self.backend == RtBackend::Hardware {
            uniforms.push(&u(UniformType::ACCELERATION_STRUCTURE, 0, &[g.tlas]));
        } else {
            uniforms.push(&u(UniformType::STORAGE_BUFFER, 0, &[g.bvh]));
        }
        uniforms.push(&u(UniformType::SAMPLER_WITH_TEXTURE, 1, &[nearest, depth]));
        uniforms.push(&u(UniformType::SAMPLER_WITH_TEXTURE, 2, &[nearest, normal]));
        uniforms.push(&u(UniformType::STORAGE_BUFFER, 3, &[g.idx]));
        uniforms.push(&u(UniformType::STORAGE_BUFFER, 4, &[g.pos]));
        uniforms.push(&u(UniformType::STORAGE_BUFFER, 5, &[g.rad]));
        uniforms.push(&u(UniformType::STORAGE_BUFFER, 6, &[g.alb]));
        uniforms.push(&u(UniformType::SAMPLER_WITH_TEXTURE, 7, &[linear, gi[prev]]));
        uniforms.push(&u(UniformType::SAMPLER_WITH_TEXTURE, 8, &[linear, refl[prev]]));
        uniforms.push(&u(UniformType::SAMPLER_WITH_TEXTURE, 9, &[nearest, z[prev]]));
        uniforms.push(&u(UniformType::IMAGE, 10, &[gi[cur]]));
        uniforms.push(&u(UniformType::IMAGE, 11, &[refl[cur]]));
        uniforms.push(&u(UniformType::IMAGE, 12, &[z[cur]]));
        uniforms.push(&u(UniformType::UNIFORM_BUFFER, 13, &[params]));
        let set0 = UniformSetCacheRd::get_cache(trace_shader, 0, &uniforms);
        let (compose_shader, compose_pipe) = self.compose.unwrap();
        let mut uniforms: Array<Gd<RdUniform>> = Array::new();
        uniforms.push(&u(UniformType::IMAGE, 0, &[color]));
        uniforms.push(&u(UniformType::SAMPLER_WITH_TEXTURE, 1, &[nearest, gi[cur]]));
        uniforms.push(&u(UniformType::SAMPLER_WITH_TEXTURE, 2, &[nearest, refl[cur]]));
        uniforms.push(&u(UniformType::SAMPLER_WITH_TEXTURE, 3, &[nearest, z[cur]]));
        uniforms.push(&u(UniformType::SAMPLER_WITH_TEXTURE, 4, &[nearest, normal]));
        uniforms.push(&u(UniformType::UNIFORM_BUFFER, 5, &[params]));
        let set1 = UniformSetCacheRd::get_cache(compose_shader, 0, &uniforms);
        if self.backend == RtBackend::Hardware {
            let rl = rd.raytracing_list_begin();
            rd.raytracing_list_bind_raytracing_pipeline(rl, trace_pipe);
            rd.raytracing_list_bind_uniform_set(rl, set0, 0);
            rd.raytracing_list_trace_rays(rl, 0, trace_sbt, hsize.x as u32, hsize.y as u32, 1);
            rd.raytracing_list_end();
        } else {
            let list = rd.compute_list_begin();
            rd.compute_list_bind_compute_pipeline(list, trace_pipe);
            rd.compute_list_bind_uniform_set(list, set0, 0);
            rd.compute_list_dispatch(list, ((hsize.x + 7) / 8) as u32, ((hsize.y + 7) / 8) as u32, 1);
            rd.compute_list_end();
        }
        let list = rd.compute_list_begin();
        rd.compute_list_bind_compute_pipeline(list, compose_pipe);
        rd.compute_list_bind_uniform_set(list, set1, 0);
        rd.compute_list_dispatch(list, ((size.x + 7) / 8) as u32, ((size.y + 7) / 8) as u32, 1);
        rd.compute_list_end();
        let hist = self.hist.as_mut().unwrap();
        hist.cur = cur;
        hist.valid = true;
        self.prev_view = cam;
        self.prev_proj = proj;
        self.frame = self.frame.wrapping_add(1);
        self.status = "running".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every triangle is in exactly one leaf, and every leaf's box holds its triangles.
    #[test]
    fn bvh_covers_every_triangle() {
        let mut pos = Vec::new();
        let mut idx = Vec::new();
        let mut s = 12345u32;
        let mut r = || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as f32 / 16777216.0 * 100.0
        };
        for t in 0..1000u32 {
            for _ in 0..3 {
                pos.push([r(), r(), r()]);
            }
            idx.extend_from_slice(&[3 * t, 3 * t + 1, 3 * t + 2]);
        }
        let (nodes, order) = build_bvh(&pos, &idx);
        let mut seen = vec![0u32; 1000];
        for (i, n) in nodes.iter().enumerate() {
            let count = n[7].to_bits();
            let first = n[3].to_bits();
            if count == 0 {
                assert!((first as usize) > i && (first as usize) < nodes.len());
                continue;
            }
            for k in first..first + count {
                let t = order[k as usize] as usize;
                seen[t] += 1;
                for v in 0..3 {
                    let p = pos[idx[3 * t + v] as usize];
                    for a in 0..3 {
                        assert!(p[a] >= n[a] - 1e-4 && p[a] <= n[4 + a] + 1e-4);
                    }
                }
            }
        }
        assert!(seen.iter().all(|&c| c == 1));
    }

    #[test]
    fn both_trace_shaders_assemble() {
        for b in [RtBackend::Hardware, RtBackend::Software] {
            let s = trace_source(b);
            assert!(s.contains("void run(ivec2 px)") && s.contains("Hit trace("));
            assert!(!s.contains("@@"));
        }
    }
}
