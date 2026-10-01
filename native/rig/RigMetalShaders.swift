import Foundation

/// The small, source-pinned shader program used by RigMetalRenderer.
///
/// The renderer intentionally compiles this source through Metal's public
/// runtime API.  Keeping the source here (rather than shipping a precompiled
/// metallib) makes the native target independent of a separate Metal command
/// line toolchain while still using the real GPU path.
enum RigMetalShaders {
    static let source = """
    #include <metal_stdlib>
    using namespace metal;

    struct RigVertexUniforms {
        float2 resolution;
    };

    struct RigFragmentUniforms {
        float alpha;
        float cut;
    };

    struct RigLayerVertexOut {
        float4 position [[position]];
        float2 uv;
    };

    vertex RigLayerVertexOut rigLayerVertex(
        uint vertexID [[vertex_id]],
        const device float2 *positions [[buffer(0)]],
        const device float2 *uvs [[buffer(1)]],
        constant RigVertexUniforms &uniforms [[buffer(2)]])
    {
        float2 point = positions[vertexID];
        float2 clip = point / uniforms.resolution * 2.0 - 1.0;
        RigLayerVertexOut output;
        output.position = float4(clip.x, -clip.y, 0.0, 1.0);
        output.uv = uvs[vertexID];
        return output;
    }

    fragment float4 rigLayerFragment(
        RigLayerVertexOut input [[stage_in]],
        texture2d<float> layerTexture [[texture(0)]],
        sampler textureSampler [[sampler(0)]],
        constant RigFragmentUniforms &uniforms [[buffer(0)]])
    {
        float4 color = layerTexture.sample(textureSampler, input.uv);
        if (color.a < uniforms.cut) {
            discard_fragment();
        }
        // Input textures are premultiplied once at upload.  Multiplying all
        // four components preserves premultiplied source-over semantics.
        return color * uniforms.alpha;
    }

    struct RigCompositeVertexOut {
        float4 position [[position]];
        float2 uv;
    };

    vertex RigCompositeVertexOut rigCompositeVertex(uint vertexID [[vertex_id]])
    {
        constexpr float2 positions[6] = {
            float2(-1.0,  1.0),
            float2( 1.0,  1.0),
            float2(-1.0, -1.0),
            float2(-1.0, -1.0),
            float2( 1.0,  1.0),
            float2( 1.0, -1.0)
        };
        constexpr float2 uvs[6] = {
            float2(0.0, 0.0),
            float2(1.0, 0.0),
            float2(0.0, 1.0),
            float2(0.0, 1.0),
            float2(1.0, 0.0),
            float2(1.0, 1.0)
        };
        RigCompositeVertexOut output;
        output.position = float4(positions[vertexID], 0.0, 1.0);
        output.uv = uvs[vertexID];
        return output;
    }

    struct RigCompositeUniforms {
        float weight;
    };

    fragment float4 rigCompositeFragment(
        RigCompositeVertexOut input [[stage_in]],
        texture2d<float> sourceTexture [[texture(0)]],
        sampler textureSampler [[sampler(0)]],
        constant RigCompositeUniforms &uniforms [[buffer(0)]])
    {
        // Each model target is already a complete premultiplied image.  The
        // caller uses additive blending to match ModelCompositor's weighted
        // independent-model composition.
        return sourceTexture.sample(textureSampler, input.uv) * uniforms.weight;
    }
    """
}
