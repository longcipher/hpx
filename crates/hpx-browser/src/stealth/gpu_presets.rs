//! GPU configuration presets — real-world WebGL fingerprints.

use super::{GpuProfile, WebGL1Surface};

// ── Internal helpers ─────────────────────────────────────────────────

pub(crate) fn common_params_desktop() -> Vec<(u32, serde_json::Value)> {
    use serde_json::json;
    vec![
        (0x0D33, json!(16384)),
        (0x851C, json!(16384)),
        (0x84E8, json!(16384)),
        (0x8073, json!(2048)),
        (0x8869, json!(16)),
        (0x8DFB, json!(1024)),
        (0x8DFD, json!(15)),
        (0x8DFC, json!(1024)),
        (0x8872, json!(16)),
        (0x8B4D, json!(16)),
        (0x8B4C, json!(32)),
        (0x846D, json!([1.0, 8190.0])),
        (0x846E, json!([1.0, 1.0])),
        (0x0D3A, json!([32767, 32767])),
        (0x0D56, json!(8)),
        (0x0D57, json!(8)),
        (0x80AA, json!(2)),
        (0x80A9, json!(4)),
    ]
}

pub(crate) fn standard_shader_precision() -> Vec<(u32, u32, [i32; 3])> {
    let mut out = Vec::with_capacity(12);
    for &shader_type in &[0x8B31u32, 0x8B30u32] {
        out.push((shader_type, 0x8DF0, [127, 127, 23]));
        out.push((shader_type, 0x8DF1, [127, 127, 23]));
        out.push((shader_type, 0x8DF2, [127, 127, 23]));
        out.push((shader_type, 0x8DF3, [15, 14, 0]));
        out.push((shader_type, 0x8DF4, [31, 30, 0]));
        out.push((shader_type, 0x8DF5, [31, 30, 0]));
    }
    out
}

// ── NVIDIA presets ───────────────────────────────────────────────────

/// Chrome on Windows with NVIDIA GeForce RTX 3060.
pub fn nvidia_rtx_3060_windows() -> GpuProfile {
    GpuProfile {
        vendor: "WebKit".into(),
        renderer: "WebKit WebGL".into(),
        version: "WebGL 1.0 (OpenGL ES 2.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)".into(),
        unmasked_vendor: "Google Inc. (NVIDIA)".into(),
        unmasked_renderer:
            "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)".into(),
        extensions: vec![
            "ANGLE_instanced_arrays".into(),
            "EXT_blend_minmax".into(),
            "EXT_clip_control".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_depth_clamp".into(),
            "EXT_disjoint_timer_query".into(),
            "EXT_float_blend".into(),
            "EXT_frag_depth".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_shader_texture_lod".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "EXT_sRGB".into(),
            "KHR_parallel_shader_compile".into(),
            "OES_element_index_uint".into(),
            "OES_fbo_render_mipmap".into(),
            "OES_standard_derivatives".into(),
            "OES_texture_float".into(),
            "OES_texture_float_linear".into(),
            "OES_texture_half_float".into(),
            "OES_texture_half_float_linear".into(),
            "OES_vertex_array_object".into(),
            "WEBGL_blend_func_extended".into(),
            "WEBGL_color_buffer_float".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_depth_texture".into(),
            "WEBGL_draw_buffers".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
            "WEBGL_polygon_mode".into(),
        ],
        params: common_params_desktop(),
        shader_precision: standard_shader_precision(),
        webgl1: None,
    }
}

// ── Apple presets ────────────────────────────────────────────────────

pub(crate) fn apple_m3_family_profile(chip_name: &str) -> GpuProfile {
    GpuProfile {
        vendor: "WebKit".into(),
        renderer: "WebKit WebGL".into(),
        version: "WebGL 2.0 (OpenGL ES 3.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 3.00 (OpenGL ES GLSL ES 3.0 Chromium)".into(),
        unmasked_vendor: "Google Inc. (Apple)".into(),
        unmasked_renderer: format!(
            "ANGLE (Apple, ANGLE Metal Renderer: {chip_name}, Unspecified Version)"
        ),
        extensions: vec![
            "EXT_clip_control".into(),
            "EXT_color_buffer_float".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_conservative_depth".into(),
            "EXT_depth_clamp".into(),
            "EXT_disjoint_timer_query_webgl2".into(),
            "EXT_float_blend".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_render_snorm".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "EXT_texture_norm16".into(),
            "KHR_parallel_shader_compile".into(),
            "NV_shader_noperspective_interpolation".into(),
            "OES_draw_buffers_indexed".into(),
            "OES_sample_variables".into(),
            "OES_shader_multisample_interpolation".into(),
            "OES_texture_float_linear".into(),
            "WEBGL_blend_func_extended".into(),
            "WEBGL_clip_cull_distance".into(),
            "WEBGL_compressed_texture_astc".into(),
            "WEBGL_compressed_texture_etc".into(),
            "WEBGL_compressed_texture_etc1".into(),
            "WEBGL_compressed_texture_pvrtc".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
            "WEBGL_polygon_mode".into(),
            "WEBGL_provoking_vertex".into(),
            "WEBGL_render_shared_exponent".into(),
            "WEBGL_stencil_texturing".into(),
        ],
        params: apple_m3_params(),
        shader_precision: standard_shader_precision(),
        webgl1: Some(apple_m3_webgl1_surface()),
    }
}

pub(crate) fn apple_m3_webgl1_surface() -> WebGL1Surface {
    WebGL1Surface {
        version: "WebGL 1.0 (OpenGL ES 2.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)".into(),
        extensions: vec![
            "ANGLE_instanced_arrays".into(),
            "EXT_blend_minmax".into(),
            "EXT_clip_control".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_depth_clamp".into(),
            "EXT_disjoint_timer_query".into(),
            "EXT_float_blend".into(),
            "EXT_frag_depth".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_sRGB".into(),
            "EXT_shader_texture_lod".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "KHR_parallel_shader_compile".into(),
            "OES_element_index_uint".into(),
            "OES_fbo_render_mipmap".into(),
            "OES_standard_derivatives".into(),
            "OES_texture_float".into(),
            "OES_texture_float_linear".into(),
            "OES_texture_half_float".into(),
            "OES_texture_half_float_linear".into(),
            "OES_vertex_array_object".into(),
            "WEBGL_blend_func_extended".into(),
            "WEBGL_color_buffer_float".into(),
            "WEBGL_compressed_texture_astc".into(),
            "WEBGL_compressed_texture_etc".into(),
            "WEBGL_compressed_texture_etc1".into(),
            "WEBGL_compressed_texture_pvrtc".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_depth_texture".into(),
            "WEBGL_draw_buffers".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
            "WEBGL_polygon_mode".into(),
        ],
    }
}

pub(crate) fn apple_m3_params() -> Vec<(u32, serde_json::Value)> {
    use serde_json::json;
    let mut params = common_params_desktop();
    for (pname, value) in params.iter_mut() {
        match *pname {
            0x0D3A => *value = json!([16384, 16384]),
            0x846D => *value = json!([1.0, 511.0]),
            _ => {}
        }
    }
    params
}

/// Apple M3 GPU profile.
pub fn apple_m3_macos() -> GpuProfile {
    apple_m3_family_profile("Apple M3")
}

/// Apple M3 Pro GPU profile.
pub fn apple_m3_pro_macos() -> GpuProfile {
    apple_m3_family_profile("Apple M3 Pro")
}

/// Apple M3 Max GPU profile.
pub fn apple_m3_max_macos() -> GpuProfile {
    apple_m3_family_profile("Apple M3 Max")
}

/// Apple M2 Pro GPU profile.
pub fn apple_m2_pro_macos() -> GpuProfile {
    GpuProfile {
        vendor: "WebKit".into(),
        renderer: "WebKit WebGL".into(),
        version: "WebGL 1.0 (OpenGL ES 2.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)".into(),
        unmasked_vendor: "Google Inc. (Apple)".into(),
        unmasked_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2 Pro, Unspecified Version)"
            .into(),
        extensions: vec![
            "ANGLE_instanced_arrays".into(),
            "EXT_blend_minmax".into(),
            "EXT_clip_control".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_depth_clamp".into(),
            "EXT_float_blend".into(),
            "EXT_frag_depth".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_shader_texture_lod".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "EXT_sRGB".into(),
            "KHR_parallel_shader_compile".into(),
            "OES_element_index_uint".into(),
            "OES_fbo_render_mipmap".into(),
            "OES_standard_derivatives".into(),
            "OES_texture_float".into(),
            "OES_texture_float_linear".into(),
            "OES_texture_half_float".into(),
            "OES_texture_half_float_linear".into(),
            "OES_vertex_array_object".into(),
            "WEBGL_blend_func_extended".into(),
            "WEBGL_color_buffer_float".into(),
            "WEBGL_compressed_texture_astc".into(),
            "WEBGL_compressed_texture_etc".into(),
            "WEBGL_compressed_texture_etc1".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_depth_texture".into(),
            "WEBGL_draw_buffers".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
        ],
        params: common_params_desktop(),
        shader_precision: standard_shader_precision(),
        webgl1: None,
    }
}

// ── Intel presets ────────────────────────────────────────────────────

/// Intel UHD 630 on Linux.
pub fn intel_uhd_630_linux() -> GpuProfile {
    GpuProfile {
        vendor: "WebKit".into(),
        renderer: "WebKit WebGL".into(),
        version: "WebGL 1.0 (OpenGL ES 2.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)".into(),
        unmasked_vendor: "Google Inc. (Intel)".into(),
        unmasked_renderer: "ANGLE (Intel, Mesa Intel(R) UHD Graphics 630 (CFL GT2), OpenGL 4.6)"
            .into(),
        extensions: vec![
            "ANGLE_instanced_arrays".into(),
            "EXT_blend_minmax".into(),
            "EXT_clip_control".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_depth_clamp".into(),
            "EXT_disjoint_timer_query".into(),
            "EXT_float_blend".into(),
            "EXT_frag_depth".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_shader_texture_lod".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "EXT_sRGB".into(),
            "KHR_parallel_shader_compile".into(),
            "OES_element_index_uint".into(),
            "OES_fbo_render_mipmap".into(),
            "OES_standard_derivatives".into(),
            "OES_texture_float".into(),
            "OES_texture_float_linear".into(),
            "OES_texture_half_float".into(),
            "OES_texture_half_float_linear".into(),
            "OES_vertex_array_object".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_depth_texture".into(),
            "WEBGL_draw_buffers".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
        ],
        params: common_params_desktop(),
        shader_precision: standard_shader_precision(),
        webgl1: None,
    }
}
