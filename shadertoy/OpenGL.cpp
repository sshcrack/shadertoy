/*
    SPDX-License-Identifier: Apache-2.0
    Copyright 2023-2025 Yingwei Zheng
    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at
        http://www.apache.org/licenses/LICENSE-2.0
    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.
*/

#include "shadertoy/Backend.hpp"
#include "shadertoy/Support.hpp"
#include <algorithm>
#include <array>
#include <cassert>
#include <chrono>
#include <cmath>
#include <limits>

#include "shadertoy/SuppressWarningPush.hpp"

#include <glad/glad.h>

#include "shadertoy/SuppressWarningPop.hpp"

#include <cmath>

SHADERTOY_NAMESPACE_BEGIN

namespace {
    uint32_t checkedPixelDimension(const float value, const char* label) {
        if(!std::isfinite(value) || value < 1.0f ||
           static_cast<double>(value) > static_cast<double>(std::numeric_limits<GLsizei>::max()) || std::floor(value) != value)
            throw Error(std::string(label) + " must be a positive integer within the OpenGL dimension range");
        return static_cast<uint32_t>(value);
    }

    void validateFramebufferDimensions(const uint32_t width, const uint32_t height) {
        if(width == 0 || height == 0 || width > static_cast<uint32_t>(std::numeric_limits<GLsizei>::max()) ||
           height > static_cast<uint32_t>(std::numeric_limits<GLsizei>::max()))
            throw Error("Framebuffer dimensions are outside the OpenGL dimension range");
    }

    class ScopedPixelStoreAlignment final {
        GLenum mParameter;
        GLint mPreviousAlignment = 0;

    public:
        ScopedPixelStoreAlignment(const GLenum parameter, const GLint alignment) : mParameter(parameter) {
            glGetIntegerv(parameter, &mPreviousAlignment);
            glPixelStorei(parameter, alignment);
        }
        ScopedPixelStoreAlignment(const ScopedPixelStoreAlignment&) = delete;
        ScopedPixelStoreAlignment& operator=(const ScopedPixelStoreAlignment&) = delete;
        ~ScopedPixelStoreAlignment() {
            glPixelStorei(mParameter, mPreviousAlignment);
        }
    };
}  // namespace

static const char* const shaderVersionDirective = "#version 410 core\n";
static const char* const computeShaderVersionDirective = "#version 430 core\n";

struct GLRenderFormat final {
    GLint internalFormat;
    GLenum format;
    GLenum uploadType;
    const char* imageQualifier;
};

[[nodiscard]] constexpr GLRenderFormat renderFormatInfo(const RenderFormat format) {
    switch(format) {
        case RenderFormat::R32F:
            return { GL_R32F, GL_RED, GL_FLOAT, "r32f" };
        case RenderFormat::RG32F:
            return { GL_RG32F, GL_RG, GL_FLOAT, "rg32f" };
        case RenderFormat::RGBA16F:
            return { GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT, "rgba16f" };
        case RenderFormat::RGBA32F:
            return { GL_RGBA32F, GL_RGBA, GL_FLOAT, "rgba32f" };
    }
    SHADERTOY_UNREACHABLE();
}
static const char* const shaderCubeMapDef = "#define INTERFACE_SHADERTOY_CUBE_MAP\n";
static const char* const shaderVertexSrc = R"(
layout (location = 0) in vec2 pos;
layout (location = 1) in vec2 texCoord;
#ifdef INTERFACE_SHADERTOY_CUBE_MAP
layout (location = 2) in vec3 point;
#endif

layout (location = 0) out vec2 f_fragCoord;
#ifdef INTERFACE_SHADERTOY_CUBE_MAP
layout (location = 1) out vec3 f_point;
#endif

void main() {
    gl_Position = vec4(pos, 0.0f, 1.0f);
    f_fragCoord = texCoord;
#ifdef INTERFACE_SHADERTOY_CUBE_MAP
    f_point = point;
#endif
}

)";

static const char* const shaderMeshCommonHeader = R"(
uniform vec3      iResolution;
uniform float     iTime;
uniform float     iTimeDelta;
uniform float     iFrameRate;
uniform int       iFrame;
uniform vec4      iMouse;
uniform vec4      iDate;
uniform vec3 iChannelResolution[16];
uniform float iChannelTime[16];

uniform vec4 iMusicBands;
uniform vec4 iMusicHits;
uniform vec4 iMusicBeat;
uniform vec4 iMusicStereo;
uniform vec4 iMusicStructure;
uniform vec4 iMusicMeta;

#define iAudioLoudness          (iMusicBands.x)
#define iAudioBass              (iMusicBands.y)
#define iAudioMid               (iMusicBands.z)
#define iAudioTreble            (iMusicBands.w)
#define iAudioOnset             (iMusicHits.x)
#define iAudioKick              (iMusicHits.y)
#define iAudioSnare             (iMusicHits.z)
#define iAudioHihat             (iMusicHits.w)
#define iAudioBpm               (iMusicBeat.x)
#define iAudioBeatPhase         (iMusicBeat.y)
#define iAudioBeatConfidence    (iMusicBeat.z)
#define iAudioBeatStrength      (iMusicBeat.w)
#define iAudioStereoWidth       (iMusicStereo.x)
#define iAudioStereoBalance     (iMusicStereo.y)
#define iAudioStereoCorrelation (iMusicStereo.z)
#define iAudioEnergyTrend       (iMusicStereo.w)
#define iAudioDrop              (iMusicStructure.x)
#define iAudioSectionChange     (iMusicStructure.y)
#define iAudioSpectralCentroid  (iMusicStructure.z)
#define iAudioSpectralFlux      (iMusicStructure.w)
#define iAudioAvailable         (iMusicMeta.x)
#define iAudioSilence           (iMusicMeta.y)
#define iAudioSampleRate        (iMusicMeta.z)

float sampleAudioSpectrum(sampler2D channel, float x) {
    return texture(channel, vec2(clamp(x, 0.0, 1.0), 0.25)).r;
}
float sampleAudioWaveform(sampler2D channel, float x) {
    return texture(channel, vec2(clamp(x, 0.0, 1.0), 0.75)).r * 2.0 - 1.0;
}

#define char char_
)";

static const char* const shaderPixelHeader = R"(
layout (location = 0) in vec2 f_fragCoord;
#ifdef INTERFACE_SHADERTOY_CUBE_MAP
layout (location = 1) in vec3 f_point;
#endif

layout (location = 0) out vec4 out_frag_color;

uniform vec3      iResolution;           // viewport resolution (in pixels)
uniform float     iTime;                 // shader playback time (in seconds)
uniform float     iTimeDelta;            // render time (in seconds)
uniform float     iFrameRate;            // shader frame rate
uniform int       iFrame;                // shader playback frame
uniform vec4      iMouse;                // mouse pixel coords. xy: current (if MLB down), zw: click
uniform vec4      iDate;                 // Year, month, day, time in seconds in .xyzw
uniform vec3 iChannelResolution[16];
uniform float iChannelTime[16];

// Host-provided semantic music analysis. The packed vec4 uniforms keep the
// renderer interface small; the aliases below are the shader-facing contract.
uniform vec4 iMusicBands;                // loudness, bass, mid, treble
uniform vec4 iMusicHits;                 // onset, kick, snare, hihat
uniform vec4 iMusicBeat;                 // bpm, phase, confidence, strength
uniform vec4 iMusicStereo;               // width, balance, correlation, energy trend
uniform vec4 iMusicStructure;            // drop, section change, spectral centroid, spectral flux
uniform vec4 iMusicMeta;                 // available, silence, sample rate, reserved

#define iAudioLoudness          (iMusicBands.x)
#define iAudioBass              (iMusicBands.y)
#define iAudioMid               (iMusicBands.z)
#define iAudioTreble            (iMusicBands.w)
#define iAudioOnset             (iMusicHits.x)
#define iAudioKick              (iMusicHits.y)
#define iAudioSnare             (iMusicHits.z)
#define iAudioHihat             (iMusicHits.w)
#define iAudioBpm               (iMusicBeat.x)
#define iAudioBeatPhase         (iMusicBeat.y)
#define iAudioBeatConfidence    (iMusicBeat.z)
#define iAudioBeatStrength      (iMusicBeat.w)
#define iAudioStereoWidth       (iMusicStereo.x)
#define iAudioStereoBalance     (iMusicStereo.y)
#define iAudioStereoCorrelation (iMusicStereo.z)
#define iAudioEnergyTrend       (iMusicStereo.w)
#define iAudioDrop              (iMusicStructure.x)
#define iAudioSectionChange     (iMusicStructure.y)
#define iAudioSpectralCentroid  (iMusicStructure.z)
#define iAudioSpectralFlux      (iMusicStructure.w)
#define iAudioAvailable         (iMusicMeta.x)
#define iAudioSilence           (iMusicMeta.y)
#define iAudioSampleRate        (iMusicMeta.z)

float sampleAudioSpectrum(sampler2D channel, float x) {
    return texture(channel, vec2(clamp(x, 0.0, 1.0), 0.25)).r;
}
float sampleAudioWaveform(sampler2D channel, float x) {
    return texture(channel, vec2(clamp(x, 0.0, 1.0), 0.75)).r * 2.0 - 1.0;
}

#define char char_
)";

static const char* const shaderPixelFooter = R"(
void main() {
#ifdef SHADERTOY_CLAMP_OUTPUT
    out_frag_color = vec4(0.0f, 0.0f, 0.0f, 1.0f);
#endif
    vec4 output_color = vec4(1e20f);
#ifndef INTERFACE_SHADERTOY_CUBE_MAP
    mainImage(output_color, gl_FragCoord.xy);
#else
    mainCubemap(output_color, gl_FragCoord.xy, vec3(0.0), normalize(f_point));
#endif
#ifdef SHADERTOY_CLAMP_OUTPUT
    out_frag_color = vec4(clamp(output_color.xyz, vec3(0.0f), vec3(1.0f)), 1.0f);
#else
    out_frag_color = output_color;
#endif
}
)";

static const char* const shaderComputeHeader = R"(
uniform vec3      iResolution;
uniform float     iTime;
uniform float     iTimeDelta;
uniform float     iFrameRate;
uniform int       iFrame;
uniform int       iIteration;
uniform vec4      iMouse;
uniform vec4      iDate;
uniform vec3      iChannelResolution[16];
uniform float     iChannelTime[16];

uniform vec4 iMusicBands;
uniform vec4 iMusicHits;
uniform vec4 iMusicBeat;
uniform vec4 iMusicStereo;
uniform vec4 iMusicStructure;
uniform vec4 iMusicMeta;

#define iAudioLoudness          (iMusicBands.x)
#define iAudioBass              (iMusicBands.y)
#define iAudioMid               (iMusicBands.z)
#define iAudioTreble            (iMusicBands.w)
#define iAudioOnset             (iMusicHits.x)
#define iAudioKick              (iMusicHits.y)
#define iAudioSnare             (iMusicHits.z)
#define iAudioHihat             (iMusicHits.w)
#define iAudioBpm               (iMusicBeat.x)
#define iAudioBeatPhase         (iMusicBeat.y)
#define iAudioBeatConfidence    (iMusicBeat.z)
#define iAudioBeatStrength      (iMusicBeat.w)
#define iAudioStereoWidth       (iMusicStereo.x)
#define iAudioStereoBalance     (iMusicStereo.y)
#define iAudioStereoCorrelation (iMusicStereo.z)
#define iAudioEnergyTrend       (iMusicStereo.w)
#define iAudioDrop              (iMusicStructure.x)
#define iAudioSectionChange     (iMusicStructure.y)
#define iAudioSpectralCentroid  (iMusicStructure.z)
#define iAudioSpectralFlux      (iMusicStructure.w)
#define iAudioAvailable         (iMusicMeta.x)
#define iAudioSilence           (iMusicMeta.y)
#define iAudioSampleRate        (iMusicMeta.z)

float sampleAudioSpectrum(sampler2D channel, float x) {
    return texture(channel, vec2(clamp(x, 0.0, 1.0), 0.25)).r;
}
float sampleAudioWaveform(sampler2D channel, float x) {
    return texture(channel, vec2(clamp(x, 0.0, 1.0), 0.75)).r * 2.0 - 1.0;
}

#define char char_
)";

static const char* const shaderComputeFooter = R"(
void main() {
    ivec2 coord = ivec2(gl_GlobalInvocationID.xy);
    if(any(greaterThanEqual(coord, ivec2(iResolution.xy))))
        return;
    mainCompute(coord);
}
)";


struct Vertex final {
    Vec2 pos;
    Vec2 coord;
};

using Vec3 = std::array<float, 3>;

constexpr Vec3 cubeMapVertexPos[8] = { { -1.0f, -1.0f, -1.0f }, { -1.0f, -1.0f, 1.0f },  //
                                       { -1.0f, 1.0f, -1.0f },  { -1.0f, 1.0f, 1.0f },   //
                                       { 1.0f, -1.0f, -1.0f },  { 1.0f, -1.0f, 1.0f },   //
                                       { 1.0f, 1.0f, -1.0f },   { 1.0f, 1.0f, 1.0f } };
// OpenGL cubemap face order: +X, -X, +Y, -Y, +Z, -Z (0,1,2,3,4,5)
// Vertices are ordered: left-bottom, left-top, right-top, right-bottom
// NOTE: The render code swaps +Y (idx=2) and -Y (idx=3) for CubeMapFlippedY
// So we need to design indices accounting for this swap
constexpr uint32_t cubeMapVertexIndex[6][4] = {
    { 5, 7, 6, 4 },  // +X (right face)
    { 0, 2, 3, 1 },  // -X (left face)
    { 0, 1, 5, 4 },  // +Y (top face)
    { 3, 2, 6, 7 },  // -Y (bottom face)
    { 1, 3, 7, 5 },  // +Z (back face)
    { 4, 6, 2, 0 }   // -Z (front face)
};

struct VertexCubeMap final {  // NOLINT(cppcoreguidelines-pro-type-member-init)
    Vec2 pos;
    Vec2 coord;
    Vec3 point;
};

static void checkShaderCompileError(const GLuint shader, const std::string_view type) {
    GLint success;
    std::vector<GLchar> buffer;
    GLint size;
    if(!type.ends_with("PROGRAM")) {
        glGetShaderiv(shader, GL_COMPILE_STATUS, &success);
        if(!success) {
            glGetShaderiv(shader, GL_INFO_LOG_LENGTH, &size);
            buffer.resize(static_cast<size_t>(size));
            glGetShaderInfoLog(shader, static_cast<GLsizei>(buffer.size()), nullptr, buffer.data());
            throw std::runtime_error(buffer.data());
        }
    } else {
        glGetProgramiv(shader, GL_LINK_STATUS, &success);
        if(!success) {
            glGetProgramiv(shader, GL_INFO_LOG_LENGTH, &size);
            buffer.resize(static_cast<size_t>(size));
            glGetProgramInfoLog(shader, static_cast<GLsizei>(buffer.size()), nullptr, buffer.data());
            throw std::runtime_error(buffer.data());
        }
    }
}

class GLFrameBuffer final : public FrameBuffer {
    GLuint mFBO{};
    GLuint mTexture{};
    RenderFormat mFormat{ RenderFormat::RGBA32F };
    uint32_t mWidth = 0, mHeight = 0;

    void allocate(const uint32_t width, const uint32_t height, const GLenum type, const void* data) {
        const auto info = renderFormatInfo(mFormat);
        glBindTexture(GL_TEXTURE_2D, mTexture);
        glTexImage2D(GL_TEXTURE_2D, 0, info.internalFormat, static_cast<GLsizei>(width), static_cast<GLsizei>(height), 0,
                     GL_RGBA, type, data);
        glBindTexture(GL_TEXTURE_2D, GL_NONE);
        mWidth = width;
        mHeight = height;
    }

public:
    explicit GLFrameBuffer(const RenderFormat format = RenderFormat::RGBA32F) : mFormat{ format } {
        glGenFramebuffers(1, &mFBO);
        glGenTextures(1, &mTexture);
        glBindFramebuffer(GL_FRAMEBUFFER, mFBO);
        glBindTexture(GL_TEXTURE_2D, mTexture);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, mTexture, 0);
        glBindTexture(GL_TEXTURE_2D, GL_NONE);
        glBindFramebuffer(GL_FRAMEBUFFER, GL_NONE);
    }
    GLFrameBuffer(const GLFrameBuffer&) = delete;
    GLFrameBuffer(GLFrameBuffer&&) = delete;
    GLFrameBuffer& operator=(const GLFrameBuffer&) = delete;
    GLFrameBuffer& operator=(GLFrameBuffer&&) = delete;
    ~GLFrameBuffer() override {
        glDeleteFramebuffers(1, &mFBO);
        glDeleteTextures(1, &mTexture);
    }
    void bind(const uint32_t width, const uint32_t height) override {
        validateFramebufferDimensions(width, height);
        if(width != mWidth || height != mHeight) {
            const auto info = renderFormatInfo(mFormat);
            glBindTexture(GL_TEXTURE_2D, mTexture);
            glTexImage2D(GL_TEXTURE_2D, 0, info.internalFormat, static_cast<GLsizei>(width), static_cast<GLsizei>(height), 0,
                         info.format, info.uploadType, nullptr);
            glBindTexture(GL_TEXTURE_2D, GL_NONE);
            mWidth = width;
            mHeight = height;
        }
        glBindFramebuffer(GL_FRAMEBUFFER, mFBO);
        if(glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
            throw Error("Framebuffer is incomplete for the selected render format");
    }
    void unbind() override {
        glBindFramebuffer(GL_FRAMEBUFFER, GL_NONE);
    }

    [[nodiscard]] uintptr_t getTexture() const override {
        return mTexture;
    }
    [[nodiscard]] std::vector<uint8_t> readRgb() override {
        if(mWidth == 0 || mHeight == 0)
            throw Error("Framebuffer has not been rendered yet");
        GLint previousFramebuffer = 0;
        glGetIntegerv(GL_READ_FRAMEBUFFER_BINDING, &previousFramebuffer);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, mFBO);
        std::vector<uint8_t> result(checkedSizeProduct({ mWidth, mHeight, 3U }, "Framebuffer RGB readback"));
        const ScopedPixelStoreAlignment tightlyPacked(GL_PACK_ALIGNMENT, 1);
        glReadPixels(0, 0, static_cast<GLsizei>(mWidth), static_cast<GLsizei>(mHeight), GL_RGB, GL_UNSIGNED_BYTE, result.data());
        glBindFramebuffer(GL_READ_FRAMEBUFFER, static_cast<GLuint>(previousFramebuffer));
        return result;
    }
    [[nodiscard]] std::vector<float> readRgba32f() override {
        if(mWidth == 0 || mHeight == 0)
            throw Error("Framebuffer has not been rendered yet");
        GLint previousFramebuffer = 0;
        glGetIntegerv(GL_READ_FRAMEBUFFER_BINDING, &previousFramebuffer);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, mFBO);
        std::vector<float> result(checkedSizeProduct({ mWidth, mHeight, 4U }, "Framebuffer RGBA readback"));
        glReadPixels(0, 0, static_cast<GLsizei>(mWidth), static_cast<GLsizei>(mHeight), GL_RGBA, GL_FLOAT, result.data());
        glBindFramebuffer(GL_READ_FRAMEBUFFER, static_cast<GLuint>(previousFramebuffer));
        return result;
    }
    void writeRgba8(const uint32_t width, const uint32_t height, const uint8_t* data) override {
        validateFramebufferDimensions(width, height);
        allocate(width, height, GL_UNSIGNED_BYTE, data);
    }
    void writeRgba32f(const uint32_t width, const uint32_t height, const float* data) override {
        validateFramebufferDimensions(width, height);
        allocate(width, height, GL_FLOAT, data);
    }
};
static constexpr uint32_t cubeMapRenderTargetSize = 1024;
class GLCubeMapRenderTarget final {
    GLuint mTex{};

public:
    GLCubeMapRenderTarget() {
        glGenTextures(1, &mTex);
        glBindTexture(GL_TEXTURE_CUBE_MAP, mTex);
        for(int32_t idx = 0; idx < 6; ++idx) {
            glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + idx, 0, GL_RGBA16F, static_cast<GLsizei>(cubeMapRenderTargetSize),
                         static_cast<GLsizei>(cubeMapRenderTargetSize), 0, GL_RGBA, GL_HALF_FLOAT, nullptr);
        }
        glBindTexture(GL_TEXTURE_CUBE_MAP, GL_NONE);
    }
    GLCubeMapRenderTarget(const GLCubeMapRenderTarget&) = delete;
    GLCubeMapRenderTarget(GLCubeMapRenderTarget&&) = delete;
    GLCubeMapRenderTarget& operator=(const GLCubeMapRenderTarget&) = delete;
    GLCubeMapRenderTarget& operator=(GLCubeMapRenderTarget&&) = delete;
    ~GLCubeMapRenderTarget() {
        glDeleteTextures(1, &mTex);
    }
    [[nodiscard]] GLuint getTexture() const {
        return mTex;
    }
};

class GLCubeMapFrameBuffer final : public FrameBuffer {
    GLuint mFBO{};
    GLuint mTexture{};

public:
    GLCubeMapFrameBuffer(GLuint texture, uint32_t idx) : mTexture{ texture } {
        glGenFramebuffers(1, &mFBO);
        glBindFramebuffer(GL_FRAMEBUFFER, mFBO);
        glBindTexture(GL_TEXTURE_CUBE_MAP, mTexture);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_X + idx, mTexture, 0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, GL_NONE);
        glBindFramebuffer(GL_FRAMEBUFFER, GL_NONE);
    }
    GLCubeMapFrameBuffer(const GLCubeMapFrameBuffer&) = delete;
    GLCubeMapFrameBuffer(GLCubeMapFrameBuffer&&) = delete;
    GLCubeMapFrameBuffer& operator=(const GLCubeMapFrameBuffer&) = delete;
    GLCubeMapFrameBuffer& operator=(GLCubeMapFrameBuffer&&) = delete;
    ~GLCubeMapFrameBuffer() override {
        glDeleteFramebuffers(1, &mFBO);
    }
    void bind(const uint32_t, const uint32_t) override {
        glBindFramebuffer(GL_FRAMEBUFFER, mFBO);
        assert(glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE);
    }
    void unbind() override {
        glBindFramebuffer(GL_FRAMEBUFFER, GL_NONE);
    }

    [[nodiscard]] uintptr_t getTexture() const override {
        return mTexture;
    }
    [[nodiscard]] std::vector<uint8_t> readRgb() override {
        GLint previousFramebuffer = 0;
        glGetIntegerv(GL_READ_FRAMEBUFFER_BINDING, &previousFramebuffer);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, mFBO);
        std::vector<uint8_t> result(
            checkedSizeProduct({ cubeMapRenderTargetSize, cubeMapRenderTargetSize, 3U }, "Cubemap RGB readback"));
        const ScopedPixelStoreAlignment tightlyPacked(GL_PACK_ALIGNMENT, 1);
        glReadPixels(0, 0, static_cast<GLsizei>(cubeMapRenderTargetSize), static_cast<GLsizei>(cubeMapRenderTargetSize), GL_RGB,
                     GL_UNSIGNED_BYTE, result.data());
        glBindFramebuffer(GL_READ_FRAMEBUFFER, static_cast<GLuint>(previousFramebuffer));
        return result;
    }
    [[nodiscard]] std::vector<float> readRgba32f() override {
        throw Error("Cubemap pass state snapshots are not supported");
    }
    void writeRgba8(const uint32_t, const uint32_t, const uint8_t*) override {
        throw Error("Cubemap pass overrides are not supported");
    }
    void writeRgba32f(const uint32_t, const uint32_t, const float*) override {
        throw Error("Cubemap pass state restores are not supported");
    }
};

class RenderPass final {
    std::string mName;
    GLuint mProgram{};
    std::vector<DoubleBufferedFB> mBuffers;
    NodeType mType;
    GLint mLocationResolution{};
    GLint mLocationTime{};
    GLint mLocationTimeDelta{};
    GLint mLocationFrameRate{};
    GLint mLocationFrame{};
    GLint mLocationIteration{};
    GLint mLocationMouse{};
    GLint mLocationDate{};
    GLint mLocationMusicBands{};
    GLint mLocationMusicHits{};
    GLint mLocationMusicBeat{};
    GLint mLocationMusicStereo{};
    GLint mLocationMusicStructure{};
    GLint mLocationMusicMeta{};
    std::array<GLint, MaxInputChannels> mLocationChannel{};
    std::array<GLint, MaxInputChannels> mLocationChannelResolution{};
    std::array<GLint, MaxInputChannels> mLocationChannelTime{};
    std::vector<Channel> mChannels;
    std::array<GLuint, MaxInputChannels> mSamplers{};
    std::optional<Vec2> mFixedResolution;
    bool mClampOutput{};
    RenderFormat mFormat{ RenderFormat::RGBA32F };
    std::vector<RenderFormat> mExtraFormats;
    GLuint mMrtFBO{};
    uint32_t mIterations{ 1 };
    uint32_t mLocalSizeX{ 8 };
    uint32_t mLocalSizeY{ 8 };
    uint32_t mLocalSizeZ{ 1 };
    std::vector<std::pair<uint32_t, BufferId>> mStorageBuffers;
    std::optional<MeshConfig> mMesh;
    GLuint mMeshVAO{};
    GLuint mMeshVBO{};
    GLuint mMeshEBO{};
    GLuint mMeshDepthRBO{};
    GLsizei mMeshIndexCount{};
    uint32_t mMeshDepthWidth{};
    uint32_t mMeshDepthHeight{};
    GLint mLocationMeshSize{ -1 };
    GLint mLocationMeshInstances{ -1 };
    GLint mLocationMeshWireframe{ -1 };
    GLuint mMeshWireframeFBO{};
    GLuint mMeshWireframeTexture{};
    GLuint mMeshCompositeProgram{};
    uint32_t mMeshWireframeWidth{};
    uint32_t mMeshWireframeHeight{};
    bool mWireframe{};
    uint32_t mLastWidth{};
    uint32_t mLastHeight{};

    void applyCustomUniforms(const ShaderToyUniform& uniform) const {
        if(!uniform.customUniforms)
            return;
        for(const auto& [name, value] : *uniform.customUniforms) {
            const auto location = glGetUniformLocation(mProgram, name.c_str());
            if(location == -1)
                continue;
            switch(value.type) {
                case CustomUniformType::Float:
                    glUniform1f(location, value.value.x);
                    break;
                case CustomUniformType::Int:
                    glUniform1i(location, value.intValue);
                    break;
                case CustomUniformType::Vec2:
                    glUniform2f(location, value.value.x, value.value.y);
                    break;
                case CustomUniformType::Vec3:
                    glUniform3f(location, value.value.x, value.value.y, value.value.z);
                    break;
                case CustomUniformType::Vec4:
                    glUniform4f(location, value.value.x, value.value.y, value.value.z, value.value.w);
                    break;
            }
        }
    }

    [[nodiscard]] GLuint compileProgram(const std::string& src) const {
        if(mType == NodeType::Compute) {
            if(!GLAD_GL_VERSION_4_3)
                throw Error("Compute passes require OpenGL 4.3 or newer");
            std::string computeSrc = computeShaderVersionDirective;
            computeSrc += "layout(local_size_x = " + std::to_string(mLocalSizeX) + ", local_size_y = " +
                std::to_string(mLocalSizeY) + ", local_size_z = " + std::to_string(mLocalSizeZ) + ") in;\n";
            std::vector<RenderFormat> outputFormats;
            outputFormats.reserve(1 + mExtraFormats.size());
            outputFormats.push_back(mFormat);
            outputFormats.insert(outputFormats.end(), mExtraFormats.begin(), mExtraFormats.end());
            for(uint32_t output = 0; output < outputFormats.size(); ++output) {
                computeSrc += "layout(";
                computeSrc += renderFormatInfo(outputFormats[output]).imageQualifier;
                computeSrc += ", binding = " + std::to_string(output) + ") uniform image2D iOutput";
                if(output != 0)
                    computeSrc += std::to_string(output);
                computeSrc += ";\n";
            }
            computeSrc += shaderComputeHeader;
            for(const auto& channel : mChannels) {
                computeSrc += "uniform sampler";
                computeSrc += channel.tex.type == TexType::CubeMap ? "Cube" : channel.tex.type == TexType::Tex2D ? "2D" : "3D";
                computeSrc += " iChannel";
                computeSrc += std::to_string(channel.slot);
                computeSrc += ";\n";
            }
            computeSrc += "#line 1\n";
            computeSrc += src;
            computeSrc += shaderComputeFooter;

            const auto* computeSrcData = computeSrc.c_str();
            const auto shaderCompute = glCreateShader(GL_COMPUTE_SHADER);
            auto computeGuard = scopeExit([&] { glDeleteShader(shaderCompute); });
            glShaderSource(shaderCompute, 1, &computeSrcData, nullptr);
            glCompileShader(shaderCompute);
            checkShaderCompileError(shaderCompute, "COMPUTE");

            const auto program = glCreateProgram();
            auto programGuard = scopeFail([&] { glDeleteProgram(program); });
            glAttachShader(program, shaderCompute);
            auto computeBindGuard = scopeExit([&] { glDetachShader(program, shaderCompute); });
            glLinkProgram(program);
            checkShaderCompileError(program, "PROGRAM");
            return program;
        }

        if(!mStorageBuffers.empty() && !GLAD_GL_VERSION_4_3)
            throw Error("Shader storage buffers require OpenGL 4.3 or newer");
        const auto* graphicsVersion = mStorageBuffers.empty() ? shaderVersionDirective : computeShaderVersionDirective;

        if(mType == NodeType::Mesh) {
            if(!mMesh)
                throw Error("Mesh pass is missing mesh configuration");
            std::string vertexSrc = graphicsVersion;
            vertexSrc += "layout(location = 0) in vec2 a_grid;\n";
            vertexSrc += "uniform ivec2 iMeshSize;\n";
            vertexSrc += "uniform int iMeshInstances;\n";
            vertexSrc += shaderMeshCommonHeader;
            for(const auto& channel : mChannels) {
                vertexSrc += "uniform sampler";
                vertexSrc += channel.tex.type == TexType::CubeMap ? "Cube" : channel.tex.type == TexType::Tex2D ? "2D" : "3D";
                vertexSrc += " iChannel" + std::to_string(channel.slot) + ";\n";
            }
            vertexSrc += "#line 1\n";
            vertexSrc += mMesh->vertexSource;

            std::string pixelSrc = graphicsVersion;
            pixelSrc += "layout(location = 0) out vec4 out_frag_color;\n";
            pixelSrc += "uniform int iMeshWireframe;\n";
            pixelSrc += shaderMeshCommonHeader;
            for(const auto& channel : mChannels) {
                pixelSrc += "uniform sampler";
                pixelSrc += channel.tex.type == TexType::CubeMap ? "Cube" : channel.tex.type == TexType::Tex2D ? "2D" : "3D";
                pixelSrc += " iChannel" + std::to_string(channel.slot) + ";\n";
            }
            if(mMesh->depth) {
                pixelSrc += "layout(location = " + std::to_string(1 + mExtraFormats.size()) +
                    ") out float out_mesh_depth;\n";
            }
            pixelSrc += "#line 1\n";
            pixelSrc += src;
            pixelSrc += R"(
void main() {
    vec4 output_color = vec4(1e20f);
    if(iMeshWireframe != 0) {
        output_color = vec4(1.0, 0.0, 1.0, 1.0);
    } else {
        mainImage(output_color, gl_FragCoord.xy);
    }
    out_frag_color = output_color;
)";
            if(mMesh->depth)
                pixelSrc += "    if(iMeshWireframe == 0) out_mesh_depth = gl_FragCoord.z;\n";
            pixelSrc += "}\n";

            const auto* vertexData = vertexSrc.c_str();
            const auto* pixelData = pixelSrc.c_str();
            const auto shaderVertex = glCreateShader(GL_VERTEX_SHADER);
            auto vertGuard = scopeExit([&] { glDeleteShader(shaderVertex); });
            glShaderSource(shaderVertex, 1, &vertexData, nullptr);
            glCompileShader(shaderVertex);
            checkShaderCompileError(shaderVertex, "VERTEX");

            const auto shaderPixel = glCreateShader(GL_FRAGMENT_SHADER);
            auto pixelGuard = scopeExit([&] { glDeleteShader(shaderPixel); });
            glShaderSource(shaderPixel, 1, &pixelData, nullptr);
            glCompileShader(shaderPixel);
            checkShaderCompileError(shaderPixel, "PIXEL");

            const auto program = glCreateProgram();
            auto programGuard = scopeFail([&] { glDeleteProgram(program); });
            glAttachShader(program, shaderVertex);
            auto vertBindGuard = scopeExit([&] { glDetachShader(program, shaderVertex); });
            glAttachShader(program, shaderPixel);
            auto pixelBindGuard = scopeExit([&] { glDetachShader(program, shaderPixel); });
            glLinkProgram(program);
            checkShaderCompileError(program, "PROGRAM");
            return program;
        }

        std::string vertexSrc = graphicsVersion;
        std::string pixelSrc = graphicsVersion;
        if(mType == NodeType::CubeMap) {
            vertexSrc += shaderCubeMapDef;
            pixelSrc += shaderCubeMapDef;
        }

        vertexSrc += shaderVertexSrc;
        pixelSrc += shaderPixelHeader;
        for(const auto& channel : mChannels) {
            pixelSrc += "uniform sampler";
            pixelSrc += channel.tex.type == TexType::CubeMap ? "Cube" : channel.tex.type == TexType::Tex2D ? "2D" : "3D";
            pixelSrc += " iChannel";
            pixelSrc += std::to_string(channel.slot);
            pixelSrc += ";\n";
        }
        if(mClampOutput)
            pixelSrc += "#define SHADERTOY_CLAMP_OUTPUT\n";
        pixelSrc += "#line 1\n";
        pixelSrc += src;
        pixelSrc += shaderPixelFooter;

        const auto* vertexSrcData = vertexSrc.c_str();
        const auto* pixelSrcData = pixelSrc.c_str();

        const auto shaderVertex = glCreateShader(GL_VERTEX_SHADER);
        auto vertGuard = scopeExit([&] { glDeleteShader(shaderVertex); });
        glShaderSource(shaderVertex, 1, &vertexSrcData, nullptr);
        glCompileShader(shaderVertex);
        checkShaderCompileError(shaderVertex, "VERTEX");

        const auto shaderPixel = glCreateShader(GL_FRAGMENT_SHADER);
        auto pixelGuard = scopeExit([&] { glDeleteShader(shaderPixel); });
        glShaderSource(shaderPixel, 1, &pixelSrcData, nullptr);
        glCompileShader(shaderPixel);
        checkShaderCompileError(shaderPixel, "PIXEL");

        const auto program = glCreateProgram();
        auto programGuard = scopeFail([&] { glDeleteProgram(program); });
        glAttachShader(program, shaderVertex);
        auto vertBindGuard = scopeExit([&] { glDetachShader(program, shaderVertex); });
        glAttachShader(program, shaderPixel);
        auto pixelBindGuard = scopeExit([&] { glDetachShader(program, shaderPixel); });
        glLinkProgram(program);
        checkShaderCompileError(program, "PROGRAM");
        return program;
    }
    [[nodiscard]] GLuint compileMeshCompositeProgram() const {
        const std::string vertexSrc = std::string(shaderVersionDirective) + R"(
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
)";
        const std::string pixelSrc = std::string(shaderVersionDirective) + R"(
in vec2 v_uv;
layout(location = 0) out vec4 out_color;
uniform sampler2D u_overlay;
void main() {
    out_color = texture(u_overlay, v_uv);
}
)";
        const auto* vertexData = vertexSrc.c_str();
        const auto* pixelData = pixelSrc.c_str();
        const auto shaderVertex = glCreateShader(GL_VERTEX_SHADER);
        auto vertGuard = scopeExit([&] { glDeleteShader(shaderVertex); });
        glShaderSource(shaderVertex, 1, &vertexData, nullptr);
        glCompileShader(shaderVertex);
        checkShaderCompileError(shaderVertex, "MESH-WIREFRAME-VERTEX");

        const auto shaderPixel = glCreateShader(GL_FRAGMENT_SHADER);
        auto pixelGuard = scopeExit([&] { glDeleteShader(shaderPixel); });
        glShaderSource(shaderPixel, 1, &pixelData, nullptr);
        glCompileShader(shaderPixel);
        checkShaderCompileError(shaderPixel, "MESH-WIREFRAME-PIXEL");

        const auto program = glCreateProgram();
        auto programGuard = scopeFail([&] { glDeleteProgram(program); });
        glAttachShader(program, shaderVertex);
        auto vertBindGuard = scopeExit([&] { glDetachShader(program, shaderVertex); });
        glAttachShader(program, shaderPixel);
        auto pixelBindGuard = scopeExit([&] { glDetachShader(program, shaderPixel); });
        glLinkProgram(program);
        checkShaderCompileError(program, "MESH-WIREFRAME-PROGRAM");
        return program;
    }

    void refreshUniformLocations() {
#define SHADERTOY_GET_UNIFORM_LOCATION(NAME) mLocation##NAME = glGetUniformLocation(mProgram, "i" #NAME)
        SHADERTOY_GET_UNIFORM_LOCATION(Resolution);
        SHADERTOY_GET_UNIFORM_LOCATION(Time);
        SHADERTOY_GET_UNIFORM_LOCATION(TimeDelta);
        SHADERTOY_GET_UNIFORM_LOCATION(FrameRate);
        SHADERTOY_GET_UNIFORM_LOCATION(Frame);
        SHADERTOY_GET_UNIFORM_LOCATION(Iteration);
        SHADERTOY_GET_UNIFORM_LOCATION(Mouse);
        SHADERTOY_GET_UNIFORM_LOCATION(Date);
        SHADERTOY_GET_UNIFORM_LOCATION(MusicBands);
        SHADERTOY_GET_UNIFORM_LOCATION(MusicHits);
        SHADERTOY_GET_UNIFORM_LOCATION(MusicBeat);
        SHADERTOY_GET_UNIFORM_LOCATION(MusicStereo);
        SHADERTOY_GET_UNIFORM_LOCATION(MusicStructure);
        SHADERTOY_GET_UNIFORM_LOCATION(MusicMeta);
#undef SHADERTOY_GET_UNIFORM_LOCATION
        mLocationMeshSize = glGetUniformLocation(mProgram, "iMeshSize");
        mLocationMeshInstances = glGetUniformLocation(mProgram, "iMeshInstances");
        mLocationMeshWireframe = glGetUniformLocation(mProgram, "iMeshWireframe");
        for(uint32_t slot = 0; slot < MaxInputChannels; ++slot) {
            const auto suffix = std::to_string(slot);
            mLocationChannel[slot] = glGetUniformLocation(mProgram, ("iChannel" + suffix).c_str());
            mLocationChannelResolution[slot] =
                glGetUniformLocation(mProgram, ("iChannelResolution[" + suffix + "]").c_str());
            mLocationChannelTime[slot] = glGetUniformLocation(mProgram, ("iChannelTime[" + suffix + "]").c_str());
        }
    }

public:
    RenderPass(std::string name, const std::string& src, NodeType type, std::vector<DoubleBufferedFB> buffer,
               std::vector<Channel> channels, std::optional<Vec2> fixedResolution, const bool clampOutput,
               const RenderFormat format, std::vector<RenderFormat> extraFormats, const uint32_t iterations,
               const uint32_t localSizeX, const uint32_t localSizeY, const uint32_t localSizeZ,
               std::vector<std::pair<uint32_t, BufferId>> storageBuffers, std::optional<MeshConfig> mesh)
        : mName{ std::move(name) }, mBuffers{ std::move(buffer) }, mType{ type }, mChannels{ std::move(channels) },
          mFixedResolution{ fixedResolution }, mClampOutput{ clampOutput }, mFormat{ format },
          mExtraFormats{ std::move(extraFormats) }, mIterations{ iterations }, mLocalSizeX{ localSizeX },
          mLocalSizeY{ localSizeY }, mLocalSizeZ{ localSizeZ }, mStorageBuffers{ std::move(storageBuffers) },
          mMesh{ std::move(mesh) } {
        const auto expectedTargets = 1 + mExtraFormats.size() +
            (mType == NodeType::Mesh && mMesh && mMesh->depth ? 1U : 0U);
        if(mBuffers.size() != expectedTargets && mType != NodeType::CubeMap)
            throw Error("Render target count does not match pass output formats");
        if((mType == NodeType::Image && mBuffers.size() > 1) || mType == NodeType::Mesh) {
            GLint maxDrawBuffers = 0;
            GLint maxColorAttachments = 0;
            glGetIntegerv(GL_MAX_DRAW_BUFFERS, &maxDrawBuffers);
            glGetIntegerv(GL_MAX_COLOR_ATTACHMENTS, &maxColorAttachments);
            if(mBuffers.size() > static_cast<size_t>(std::min(maxDrawBuffers, maxColorAttachments)))
                throw Error("Pass render-target count exceeds the OpenGL MRT limit");
            glGenFramebuffers(1, &mMrtFBO);
        }
        if(mType == NodeType::Mesh) {
            if(!mMesh)
                throw Error("Mesh configuration is required");
            if(mMesh->width < 2 || mMesh->height < 2 || mMesh->instances == 0 ||
               mMesh->instances > static_cast<uint32_t>(std::numeric_limits<GLsizei>::max()))
                throw Error("Invalid mesh grid dimensions or instance count");
            const uint64_t indexCount = static_cast<uint64_t>(mMesh->width - 1) * (mMesh->height - 1) * 6U;
            if(indexCount > static_cast<uint64_t>(std::numeric_limits<GLsizei>::max()))
                throw Error("Mesh index count exceeds OpenGL draw range");

            std::vector<Vec2> vertices;
            vertices.reserve(static_cast<size_t>(mMesh->width) * mMesh->height);
            for(uint32_t y = 0; y < mMesh->height; ++y) {
                for(uint32_t x = 0; x < mMesh->width; ++x) {
                    vertices.push_back(Vec2{
                        static_cast<float>(x) / static_cast<float>(mMesh->width - 1),
                        static_cast<float>(y) / static_cast<float>(mMesh->height - 1),
                    });
                }
            }
            std::vector<uint32_t> indices;
            indices.reserve(static_cast<size_t>(indexCount));
            for(uint32_t y = 0; y + 1 < mMesh->height; ++y) {
                for(uint32_t x = 0; x + 1 < mMesh->width; ++x) {
                    const uint32_t i0 = y * mMesh->width + x;
                    const uint32_t i1 = i0 + 1;
                    const uint32_t i3 = i0 + mMesh->width;
                    const uint32_t i2 = i3 + 1;
                    indices.insert(indices.end(), { i0, i1, i2, i0, i2, i3 });
                }
            }
            mMeshIndexCount = static_cast<GLsizei>(indices.size());
            glGenVertexArrays(1, &mMeshVAO);
            glGenBuffers(1, &mMeshVBO);
            glGenBuffers(1, &mMeshEBO);
            glBindVertexArray(mMeshVAO);
            glBindBuffer(GL_ARRAY_BUFFER, mMeshVBO);
            glBufferData(GL_ARRAY_BUFFER, static_cast<GLsizeiptr>(vertices.size() * sizeof(Vec2)), vertices.data(), GL_STATIC_DRAW);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, mMeshEBO);
            glBufferData(GL_ELEMENT_ARRAY_BUFFER, static_cast<GLsizeiptr>(indices.size() * sizeof(uint32_t)), indices.data(), GL_STATIC_DRAW);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(Vec2), nullptr);
            glBindVertexArray(0);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            if(mMesh->depth) {
                glGenRenderbuffers(1, &mMeshDepthRBO);
                glGenFramebuffers(1, &mMeshWireframeFBO);
                glGenTextures(1, &mMeshWireframeTexture);
                glBindTexture(GL_TEXTURE_2D, mMeshWireframeTexture);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
                glBindTexture(GL_TEXTURE_2D, 0);
                mMeshCompositeProgram = compileMeshCompositeProgram();
            }
        }

        mProgram = compileProgram(src);
        refreshUniformLocations();

        for(const auto& channel : mChannels) {
            const GLint wrapMode = [&] {
                switch(channel.wrapMode) {
                    case Wrap::Clamp:
                        return GL_CLAMP_TO_EDGE;
                    case Wrap::Repeat:
                        return GL_REPEAT;
                }
                SHADERTOY_UNREACHABLE();
            }();
            const GLint minFilter = [&] {
                switch(channel.filter) {
                    case Filter::Mipmap:
                        return GL_LINEAR_MIPMAP_LINEAR;
                    case Filter::Nearest:
                        return GL_NEAREST;
                    case Filter::Linear:
                        return GL_LINEAR;
                }
                SHADERTOY_UNREACHABLE();
            }();
            const GLint magFilter = [&] {
                switch(channel.filter) {
                    case Filter::Nearest:
                        return GL_NEAREST;
                    case Filter::Mipmap:
                        [[fallthrough]];
                    case Filter::Linear:
                        return GL_LINEAR;
                }
                SHADERTOY_UNREACHABLE();
            }();

            auto& sampler = mSamplers[channel.slot];
            glGenSamplers(1, &sampler);
            glSamplerParameteri(sampler, GL_TEXTURE_WRAP_S, wrapMode);
            glSamplerParameteri(sampler, GL_TEXTURE_WRAP_T, wrapMode);
            glSamplerParameteri(sampler, GL_TEXTURE_WRAP_R, wrapMode);
            glSamplerParameteri(sampler, GL_TEXTURE_MIN_FILTER, minFilter);
            glSamplerParameteri(sampler, GL_TEXTURE_MAG_FILTER, magFilter);
        }
    }

    void reloadSource(const std::string& src) {
        const GLuint replacement = compileProgram(src);
        const GLuint previous = mProgram;
        mProgram = replacement;
        refreshUniformLocations();
        glDeleteProgram(previous);
    }

    [[nodiscard]] uint32_t lastWidth() const noexcept {
        return mLastWidth;
    }
    [[nodiscard]] uint32_t lastHeight() const noexcept {
        return mLastHeight;
    }
    RenderPass(const RenderPass&) = delete;
    RenderPass(RenderPass&&) = delete;
    RenderPass& operator=(const RenderPass&) = delete;
    RenderPass& operator=(RenderPass&&) = delete;
    ~RenderPass() {
        if(mMrtFBO != 0)
            glDeleteFramebuffers(1, &mMrtFBO);
        if(mMeshDepthRBO != 0)
            glDeleteRenderbuffers(1, &mMeshDepthRBO);
        if(mMeshWireframeFBO != 0)
            glDeleteFramebuffers(1, &mMeshWireframeFBO);
        if(mMeshWireframeTexture != 0)
            glDeleteTextures(1, &mMeshWireframeTexture);
        if(mMeshCompositeProgram != 0)
            glDeleteProgram(mMeshCompositeProgram);
        if(mMeshEBO != 0)
            glDeleteBuffers(1, &mMeshEBO);
        if(mMeshVBO != 0)
            glDeleteBuffers(1, &mMeshVBO);
        if(mMeshVAO != 0)
            glDeleteVertexArrays(1, &mMeshVAO);
        glDeleteSamplers(static_cast<GLsizei>(mSamplers.size()), mSamplers.data());
        glDeleteProgram(mProgram);
    }
    void setWireframe(const bool enabled) {
        if(enabled && (mType != NodeType::Mesh || !mMesh || !mMesh->depth))
            throw Error("Wireframe overlay requires a depth-enabled mesh pass");
        mWireframe = enabled;
    }
    [[nodiscard]] bool wireframeEnabled() const noexcept {
        return mWireframe;
    }
    void renderWireframeOverlay(const GLuint framebuffer, const Vec2 frameBufferSize, const Vec2 clipMin,
                                const Vec2 clipMax) const {
        if(!mWireframe)
            return;
        if(mMeshWireframeTexture == 0 || mMeshCompositeProgram == 0)
            throw Error("Wireframe overlay has no rendered mesh target");

        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, framebuffer);
        glViewport(0, 0, static_cast<GLsizei>(frameBufferSize.x), static_cast<GLsizei>(frameBufferSize.y));
        glEnable(GL_SCISSOR_TEST);
        glScissor(static_cast<GLint>(clipMin.x), static_cast<GLint>(frameBufferSize.y - clipMax.y),
                  static_cast<GLint>(clipMax.x - clipMin.x), static_cast<GLint>(clipMax.y - clipMin.y));
        glDisable(GL_DEPTH_TEST);
        glDepthMask(GL_FALSE);
        glDisable(GL_CULL_FACE);
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

        glUseProgram(mMeshCompositeProgram);
        const auto location = glGetUniformLocation(mMeshCompositeProgram, "u_overlay");
        if(location != -1)
            glUniform1i(location, 0);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, mMeshWireframeTexture);
        glBindVertexArray(mMeshVAO);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glBindVertexArray(0);
        glBindTexture(GL_TEXTURE_2D, 0);

        glDisable(GL_BLEND);
        glDisable(GL_SCISSOR_TEST);
        glDepthMask(GL_TRUE);
    }
    [[nodiscard]] bool isMesh() const noexcept {
        return mType == NodeType::Mesh;
    }
    [[nodiscard]] NodeType getType() const noexcept {
        return mType;
    }
    [[nodiscard]] std::string_view getName() const noexcept {
        return mName;
    }
    [[nodiscard]] bool hasOffscreenTarget() const noexcept {
        return !mBuffers.empty() && mBuffers.front().t1 != nullptr;
    }
    [[nodiscard]] uint32_t resolveReadOutput(const uint32_t output) const {
        if(output != std::numeric_limits<uint32_t>::max())
            return output;
        if(mType != NodeType::Mesh || !mMesh || !mMesh->depth)
            throw Error("Pass has no readable mesh depth output");
        return static_cast<uint32_t>(1 + mExtraFormats.size());
    }
    [[nodiscard]] std::vector<uint8_t> readRgb(const uint32_t output = 0) {
        if(mType != NodeType::Image && mType != NodeType::Compute && mType != NodeType::Mesh)
            throw Error("Only image/buffer/compute/mesh passes can be read as RGB");
        const auto resolvedOutput = resolveReadOutput(output);
        if(resolvedOutput >= mBuffers.size() || !mBuffers[resolvedOutput].t1)
            throw Error("Pass render-target index is out of range or has no offscreen state");
        return mBuffers[resolvedOutput].t1->readRgb();
    }
    [[nodiscard]] std::vector<float> readRgba32f(const uint32_t output = 0) {
        if(mType != NodeType::Image && mType != NodeType::Compute && mType != NodeType::Mesh)
            throw Error("Only image/buffer/compute/mesh passes can be snapshotted as RGBA32F");
        const auto resolvedOutput = resolveReadOutput(output);
        if(resolvedOutput >= mBuffers.size() || !mBuffers[resolvedOutput].t1)
            throw Error("Pass render-target index is out of range or has no offscreen state");
        return mBuffers[resolvedOutput].t1->readRgba32f();
    }
    void overrideRgba8(const uint32_t width, const uint32_t height, const uint8_t* data) {
        if(mType != NodeType::Image && mType != NodeType::Compute && mType != NodeType::Mesh)
            throw Error("Only image/buffer/compute/mesh passes can be overridden with a 2D image");
        if(!hasOffscreenTarget())
            throw Error("The final image pass cannot be used as a persistent buffer override");
        if(mFixedResolution &&
           (width != static_cast<uint32_t>(mFixedResolution->x) || height != static_cast<uint32_t>(mFixedResolution->y)))
            throw Error("Pass override dimensions do not match the pass fixed resolution");
        auto* first = mBuffers.front().t1;
        auto* second = mBuffers.front().t2;
        first->writeRgba8(width, height, data);
        if(second && second != first)
            second->writeRgba8(width, height, data);
    }
    void restoreRgba32f(const uint32_t width, const uint32_t height, const float* data) {
        if(mType != NodeType::Image && mType != NodeType::Compute && mType != NodeType::Mesh)
            throw Error("Only image/buffer/compute/mesh passes can restore RGBA32F state");
        if(!hasOffscreenTarget())
            throw Error("The final image pass has no persistent buffer state");
        if(mFixedResolution &&
           (width != static_cast<uint32_t>(mFixedResolution->x) || height != static_cast<uint32_t>(mFixedResolution->y)))
            throw Error("Pass restore dimensions do not match the pass fixed resolution");
        auto* first = mBuffers.front().t1;
        auto* second = mBuffers.front().t2;
        first->writeRgba32f(width, height, data);
        if(second && second != first)
            second->writeRgba32f(width, height, data);
    }
    void render(const Vec2 frameBufferSize, const Vec2 clipMin, const Vec2 clipMax, const Vec2 canvasSize,
                const ShaderToyUniform& uniform, const GLuint vao, const GLuint vbo) {
        if(mType == NodeType::Compute) {
            if(!mFixedResolution || mBuffers.empty())
                throw Error("Compute pass requires a fixed offscreen target");
            const auto width = static_cast<uint32_t>(mFixedResolution->x);
            const auto height = static_cast<uint32_t>(mFixedResolution->y);
            mLastWidth = width;
            mLastHeight = height;
            std::vector<FrameBuffer*> outputs;
            outputs.reserve(mBuffers.size());
            for(auto& target : mBuffers) {
                auto* buffer = target.get();
                if(!buffer)
                    throw Error("Compute pass has no output texture");
                buffer->bind(width, height);
                buffer->unbind();
                outputs.push_back(buffer);
            }

            glUseProgram(mProgram);
            const Vec2 computeSize{ static_cast<float>(width), static_cast<float>(height) };
            for(auto& channel : mChannels) {
                if(mLocationChannelResolution[channel.slot] != -1) {
                    if(channel.tex.type != TexType::Tex3D) {
                        const auto texSize =
                            channel.size.value_or(channel.tex.type == TexType::CubeMap ?
                                                     Vec2{ static_cast<float>(cubeMapRenderTargetSize),
                                                           static_cast<float>(cubeMapRenderTargetSize) } :
                                                     canvasSize);
                        glUniform3f(mLocationChannelResolution[channel.slot], texSize.x, texSize.y, 1.0f);
                    } else {
                        const auto x = channel.size->x;
                        glUniform3f(mLocationChannelResolution[channel.slot], x, x, x);
                    }
                }
                if(mLocationChannelTime[channel.slot] != -1)
                    glUniform1f(mLocationChannelTime[channel.slot], uniform.time);
                if(mLocationChannel[channel.slot] == -1)
                    continue;
                glUniform1i(mLocationChannel[channel.slot], static_cast<GLint>(channel.slot));
                glActiveTexture(GL_TEXTURE0 + channel.slot);
                const auto type = channel.tex.type == TexType::CubeMap ? GL_TEXTURE_CUBE_MAP :
                    channel.tex.type == TexType::Tex2D                 ? GL_TEXTURE_2D :
                                                                         GL_TEXTURE_3D;
                glBindTexture(type, static_cast<GLuint>(channel.tex.get()));
                if(channel.filter == Filter::Mipmap)
                    glGenerateMipmap(type);
                glBindSampler(channel.slot, mSamplers[channel.slot]);
            }

            for(uint32_t output = 0; output < outputs.size(); ++output) {
                const auto format = output == 0 ? mFormat : mExtraFormats[output - 1];
                const auto info = renderFormatInfo(format);
                glBindImageTexture(output, static_cast<GLuint>(outputs[output]->getTexture()), 0, GL_FALSE, 0, GL_READ_WRITE,
                                   static_cast<GLenum>(info.internalFormat));
            }
            for(const auto& [binding, id] : mStorageBuffers)
                glBindBufferBase(GL_SHADER_STORAGE_BUFFER, binding, static_cast<GLuint>(id));

            if(mLocationResolution != -1)
                glUniform3f(mLocationResolution, computeSize.x, computeSize.y, 0.0f);
            if(mLocationTime != -1)
                glUniform1f(mLocationTime, uniform.time);
            if(mLocationTimeDelta != -1)
                glUniform1f(mLocationTimeDelta, uniform.timeDelta);
            if(mLocationFrameRate != -1)
                glUniform1f(mLocationFrameRate, uniform.frameRate);
            if(mLocationFrame != -1)
                glUniform1i(mLocationFrame, uniform.frame);
            if(mLocationIteration != -1)
                glUniform1i(mLocationIteration, 0);
            if(mLocationMouse != -1)
                glUniform4f(mLocationMouse, uniform.mouse.x, uniform.mouse.y, uniform.mouse.z, uniform.mouse.w);
            if(mLocationDate != -1)
                glUniform4f(mLocationDate, uniform.date.x, uniform.date.y, uniform.date.z, uniform.date.w);
            if(mLocationMusicBands != -1)
                glUniform4f(mLocationMusicBands, uniform.audioBands.x, uniform.audioBands.y, uniform.audioBands.z,
                            uniform.audioBands.w);
            if(mLocationMusicHits != -1)
                glUniform4f(mLocationMusicHits, uniform.audioHits.x, uniform.audioHits.y, uniform.audioHits.z,
                            uniform.audioHits.w);
            if(mLocationMusicBeat != -1)
                glUniform4f(mLocationMusicBeat, uniform.audioBeat.x, uniform.audioBeat.y, uniform.audioBeat.z,
                            uniform.audioBeat.w);
            if(mLocationMusicStereo != -1)
                glUniform4f(mLocationMusicStereo, uniform.audioStereo.x, uniform.audioStereo.y, uniform.audioStereo.z,
                            uniform.audioStereo.w);
            if(mLocationMusicStructure != -1)
                glUniform4f(mLocationMusicStructure, uniform.audioStructure.x, uniform.audioStructure.y,
                            uniform.audioStructure.z, uniform.audioStructure.w);
            if(mLocationMusicMeta != -1)
                glUniform4f(mLocationMusicMeta, uniform.audioMeta.x, uniform.audioMeta.y, uniform.audioMeta.z,
                            uniform.audioMeta.w);
            applyCustomUniforms(uniform);

            const auto groupsX = (width + mLocalSizeX - 1U) / mLocalSizeX;
            const auto groupsY = (height + mLocalSizeY - 1U) / mLocalSizeY;
            for(uint32_t iteration = 0; iteration < mIterations; ++iteration) {
                if(mLocationIteration != -1)
                    glUniform1i(mLocationIteration, static_cast<GLint>(iteration));
                glDispatchCompute(groupsX, groupsY, 1);
                glMemoryBarrier(GL_SHADER_IMAGE_ACCESS_BARRIER_BIT | GL_SHADER_STORAGE_BARRIER_BIT |
                                GL_TEXTURE_FETCH_BARRIER_BIT);
            }

            for(uint32_t output = 0; output < outputs.size(); ++output)
                glBindImageTexture(output, 0, 0, GL_FALSE, 0, GL_READ_WRITE, GL_RGBA32F);
            for(const auto& [binding, id] : mStorageBuffers) {
                SHADERTOY_UNUSED(id);
                glBindBufferBase(GL_SHADER_STORAGE_BUFFER, binding, 0);
            }
            for(const auto& channel : mChannels)
                glBindSampler(channel.slot, 0);
            glActiveTexture(GL_TEXTURE0);
            return;
        }

        if(mType == NodeType::Mesh) {
            if(!mMesh || mBuffers.empty())
                throw Error("Mesh pass has no render targets");
            const auto screenSize = Vec2{ clipMax.x - clipMin.x, clipMax.y - clipMin.y };
            const auto size = mFixedResolution.value_or(screenSize);
            const auto uniformSize = mFixedResolution.value_or(canvasSize);
            const auto width = checkedPixelDimension(size.x, "Mesh target width");
            const auto height = checkedPixelDimension(size.y, "Mesh target height");
            mLastWidth = width;
            mLastHeight = height;

            std::vector<FrameBuffer*> outputs;
            outputs.reserve(mBuffers.size());
            for(auto& target : mBuffers) {
                auto* output = target.get();
                if(!output)
                    throw Error("Mesh pass requires offscreen render targets");
                output->bind(width, height);
                output->unbind();
                outputs.push_back(output);
            }

            glBindFramebuffer(GL_FRAMEBUFFER, mMrtFBO);
            std::array<GLenum, 8> drawBuffers{};
            if(outputs.size() > drawBuffers.size())
                throw Error("Mesh pass exceeds OpenGL color attachment limit");
            for(uint32_t output = 0; output < outputs.size(); ++output) {
                const auto attachment = GL_COLOR_ATTACHMENT0 + output;
                glFramebufferTexture2D(GL_FRAMEBUFFER, attachment, GL_TEXTURE_2D,
                                       static_cast<GLuint>(outputs[output]->getTexture()), 0);
                drawBuffers[output] = attachment;
            }
            glDrawBuffers(static_cast<GLsizei>(outputs.size()), drawBuffers.data());

            if(mMesh->depth) {
                if(mMeshDepthWidth != width || mMeshDepthHeight != height) {
                    glBindRenderbuffer(GL_RENDERBUFFER, mMeshDepthRBO);
                    glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT24,
                                          static_cast<GLsizei>(width), static_cast<GLsizei>(height));
                    glBindRenderbuffer(GL_RENDERBUFFER, 0);
                    mMeshDepthWidth = width;
                    mMeshDepthHeight = height;
                }
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, mMeshDepthRBO);
            }
            if(glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
                throw Error("Mesh framebuffer is incomplete");

            glViewport(0, 0, static_cast<GLsizei>(width), static_cast<GLsizei>(height));
            glDisable(GL_SCISSOR_TEST);
            glDisable(GL_BLEND);
            for(uint32_t output = 0; output < outputs.size(); ++output) {
                if(mMesh->depth && output + 1 == outputs.size()) {
                    constexpr GLfloat depthClear[4]{ 1.0f, 0.0f, 0.0f, 0.0f };
                    glClearBufferfv(GL_COLOR, static_cast<GLint>(output), depthClear);
                } else {
                    glClearBufferfv(GL_COLOR, static_cast<GLint>(output), mMesh->clear.data());
                }
            }
            if(mMesh->depth) {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LESS);
                glDepthMask(GL_TRUE);
                glClearDepth(1.0);
                glClear(GL_DEPTH_BUFFER_BIT);
            } else {
                glDisable(GL_DEPTH_TEST);
            }
            switch(mMesh->cull) {
                case MeshCull::None:
                    glDisable(GL_CULL_FACE);
                    break;
                case MeshCull::Back:
                    glEnable(GL_CULL_FACE);
                    glCullFace(GL_BACK);
                    break;
                case MeshCull::Front:
                    glEnable(GL_CULL_FACE);
                    glCullFace(GL_FRONT);
                    break;
            }

            glUseProgram(mProgram);
            glBindVertexArray(mMeshVAO);
            for(auto& channel : mChannels) {
                if(mLocationChannelResolution[channel.slot] != -1) {
                    if(channel.tex.type != TexType::Tex3D) {
                        const auto texSize = channel.size.value_or(
                            channel.tex.type == TexType::CubeMap ?
                                Vec2{ static_cast<float>(cubeMapRenderTargetSize), static_cast<float>(cubeMapRenderTargetSize) } :
                                canvasSize);
                        glUniform3f(mLocationChannelResolution[channel.slot], texSize.x, texSize.y, 1.0f);
                    } else {
                        const auto x = channel.size->x;
                        glUniform3f(mLocationChannelResolution[channel.slot], x, x, x);
                    }
                }
                if(mLocationChannelTime[channel.slot] != -1)
                    glUniform1f(mLocationChannelTime[channel.slot], uniform.time);
                if(mLocationChannel[channel.slot] == -1)
                    continue;
                glUniform1i(mLocationChannel[channel.slot], static_cast<GLint>(channel.slot));
                glActiveTexture(GL_TEXTURE0 + channel.slot);
                const auto type = channel.tex.type == TexType::CubeMap ? GL_TEXTURE_CUBE_MAP :
                    channel.tex.type == TexType::Tex2D ? GL_TEXTURE_2D : GL_TEXTURE_3D;
                glBindTexture(type, static_cast<GLuint>(channel.tex.get()));
                if(channel.filter == Filter::Mipmap)
                    glGenerateMipmap(type);
                glBindSampler(channel.slot, mSamplers[channel.slot]);
            }

            if(mLocationResolution != -1)
                glUniform3f(mLocationResolution, uniformSize.x, uniformSize.y, 0.0f);
            if(mLocationTime != -1)
                glUniform1f(mLocationTime, uniform.time);
            if(mLocationTimeDelta != -1)
                glUniform1f(mLocationTimeDelta, uniform.timeDelta);
            if(mLocationFrameRate != -1)
                glUniform1f(mLocationFrameRate, uniform.frameRate);
            if(mLocationFrame != -1)
                glUniform1i(mLocationFrame, uniform.frame);
            if(mLocationIteration != -1)
                glUniform1i(mLocationIteration, 0);
            if(mLocationMouse != -1)
                glUniform4f(mLocationMouse, uniform.mouse.x, uniform.mouse.y, uniform.mouse.z, uniform.mouse.w);
            if(mLocationDate != -1)
                glUniform4f(mLocationDate, uniform.date.x, uniform.date.y, uniform.date.z, uniform.date.w);
            if(mLocationMusicBands != -1)
                glUniform4f(mLocationMusicBands, uniform.audioBands.x, uniform.audioBands.y, uniform.audioBands.z, uniform.audioBands.w);
            if(mLocationMusicHits != -1)
                glUniform4f(mLocationMusicHits, uniform.audioHits.x, uniform.audioHits.y, uniform.audioHits.z, uniform.audioHits.w);
            if(mLocationMusicBeat != -1)
                glUniform4f(mLocationMusicBeat, uniform.audioBeat.x, uniform.audioBeat.y, uniform.audioBeat.z, uniform.audioBeat.w);
            if(mLocationMusicStereo != -1)
                glUniform4f(mLocationMusicStereo, uniform.audioStereo.x, uniform.audioStereo.y, uniform.audioStereo.z, uniform.audioStereo.w);
            if(mLocationMusicStructure != -1)
                glUniform4f(mLocationMusicStructure, uniform.audioStructure.x, uniform.audioStructure.y,
                            uniform.audioStructure.z, uniform.audioStructure.w);
            if(mLocationMusicMeta != -1)
                glUniform4f(mLocationMusicMeta, uniform.audioMeta.x, uniform.audioMeta.y, uniform.audioMeta.z, uniform.audioMeta.w);
            if(mLocationMeshSize != -1)
                glUniform2i(mLocationMeshSize, static_cast<GLint>(mMesh->width), static_cast<GLint>(mMesh->height));
            if(mLocationMeshInstances != -1)
                glUniform1i(mLocationMeshInstances, static_cast<GLint>(mMesh->instances));
            if(mLocationMeshWireframe != -1)
                glUniform1i(mLocationMeshWireframe, 0);
            applyCustomUniforms(uniform);

            for(const auto& [binding, id] : mStorageBuffers)
                glBindBufferBase(GL_SHADER_STORAGE_BUFFER, binding, static_cast<GLuint>(id));
            glPolygonMode(GL_FRONT_AND_BACK, GL_FILL);
            glDrawElementsInstanced(GL_TRIANGLES, mMeshIndexCount, GL_UNSIGNED_INT, nullptr,
                                    static_cast<GLsizei>(mMesh->instances));

            if(mWireframe) {
                if(mMeshWireframeWidth != width || mMeshWireframeHeight != height) {
                    glBindTexture(GL_TEXTURE_2D, mMeshWireframeTexture);
                    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, static_cast<GLsizei>(width),
                                 static_cast<GLsizei>(height), 0, GL_RGBA, GL_UNSIGNED_BYTE, nullptr);
                    glBindTexture(GL_TEXTURE_2D, 0);
                    mMeshWireframeWidth = width;
                    mMeshWireframeHeight = height;
                }
                glBindFramebuffer(GL_FRAMEBUFFER, mMeshWireframeFBO);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, mMeshWireframeTexture, 0);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, mMeshDepthRBO);
                constexpr GLenum wireframeBuffer = GL_COLOR_ATTACHMENT0;
                glDrawBuffers(1, &wireframeBuffer);
                if(glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
                    throw Error("Mesh wireframe framebuffer is incomplete");

                constexpr GLfloat transparent[4]{ 0.0f, 0.0f, 0.0f, 0.0f };
                glClearBufferfv(GL_COLOR, 0, transparent);
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LEQUAL);
                glDepthMask(GL_FALSE);
                if(mLocationMeshWireframe != -1)
                    glUniform1i(mLocationMeshWireframe, 1);
                glPolygonMode(GL_FRONT_AND_BACK, GL_LINE);
                glDrawElementsInstanced(GL_TRIANGLES, mMeshIndexCount, GL_UNSIGNED_INT, nullptr,
                                        static_cast<GLsizei>(mMesh->instances));
                glPolygonMode(GL_FRONT_AND_BACK, GL_FILL);
                if(mLocationMeshWireframe != -1)
                    glUniform1i(mLocationMeshWireframe, 0);
                glDepthMask(GL_TRUE);
                glDepthFunc(GL_LESS);
            }

            if(!mStorageBuffers.empty())
                glMemoryBarrier(GL_SHADER_STORAGE_BARRIER_BIT | GL_TEXTURE_FETCH_BARRIER_BIT);
            for(const auto& [binding, id] : mStorageBuffers) {
                SHADERTOY_UNUSED(id);
                glBindBufferBase(GL_SHADER_STORAGE_BUFFER, binding, 0);
            }
            for(const auto& channel : mChannels)
                glBindSampler(channel.slot, 0);
            glBindVertexArray(0);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDisable(GL_CULL_FACE);
            glDisable(GL_DEPTH_TEST);
            glActiveTexture(GL_TEXTURE0);
            return;
        }

        glDisable(GL_BLEND);
        constexpr Vec2 cubeMapSize{ static_cast<float>(cubeMapRenderTargetSize), static_cast<float>(cubeMapRenderTargetSize) };
        const auto screenBase = clipMin;
        const auto screenSize = Vec2{ clipMax.x - clipMin.x, clipMax.y - clipMin.y };
        const bool useMrt = mType == NodeType::Image && mBuffers.size() > 1;
        std::vector<FrameBuffer*> mrtOutputs;
        if(useMrt) {
            const auto mrtSize = mFixedResolution.value_or(screenSize);
            mrtOutputs.reserve(mBuffers.size());
            for(auto& target : mBuffers) {
                auto* output = target.get();
                if(!output)
                    throw Error("MRT pass requires offscreen render targets");
                output->bind(static_cast<uint32_t>(mrtSize.x), static_cast<uint32_t>(mrtSize.y));
                output->unbind();
                mrtOutputs.push_back(output);
            }
        }

        const auto renderTargetCount = useMrt ? 1U : static_cast<uint32_t>(mBuffers.size());
        for(const auto& [binding, id] : mStorageBuffers)
            glBindBufferBase(GL_SHADER_STORAGE_BUFFER, binding, static_cast<GLuint>(id));
        for(uint32_t idx = 0; idx < renderTargetCount; ++idx) {
            const auto buffer = useMrt ? mrtOutputs.front() : mBuffers[idx].get();
            Vec2 size, base, fbSize, uniformSize;
            if(buffer) {
                base = { 0, 0 };
                size = mType == NodeType::CubeMap ? cubeMapSize : mFixedResolution.value_or(screenSize);
                fbSize = size;
                uniformSize = mType == NodeType::CubeMap ? cubeMapSize : mFixedResolution.value_or(canvasSize);
                glViewport(0, 0, static_cast<GLsizei>(size.x), static_cast<GLsizei>(size.y));
                glDisable(GL_SCISSOR_TEST);
                if(useMrt) {
                    glBindFramebuffer(GL_FRAMEBUFFER, mMrtFBO);
                    std::array<GLenum, 8> drawBuffers{};
                    for(uint32_t output = 0; output < mrtOutputs.size(); ++output) {
                        const auto attachment = GL_COLOR_ATTACHMENT0 + output;
                        glFramebufferTexture2D(GL_FRAMEBUFFER, attachment, GL_TEXTURE_2D,
                                               static_cast<GLuint>(mrtOutputs[output]->getTexture()), 0);
                        drawBuffers[output] = attachment;
                    }
                    glDrawBuffers(static_cast<GLsizei>(mrtOutputs.size()), drawBuffers.data());
                    if(glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
                        throw Error("Multiple-render-target framebuffer is incomplete");
                } else {
                    buffer->bind(static_cast<uint32_t>(size.x), static_cast<uint32_t>(size.y));
                }
            } else {
                glViewport(0, 0, static_cast<GLsizei>(frameBufferSize.x), static_cast<GLsizei>(frameBufferSize.y));
                glEnable(GL_SCISSOR_TEST);
                glScissor(static_cast<GLint>(clipMin.x), static_cast<GLint>(frameBufferSize.y - clipMax.y),
                          static_cast<GLint>(clipMax.x - clipMin.x), static_cast<GLint>(clipMax.y - clipMin.y));
                base = screenBase;
                size = screenSize;
                fbSize = frameBufferSize;
                uniformSize = canvasSize;
            }
            mLastWidth = static_cast<uint32_t>(size.x);
            mLastHeight = static_cast<uint32_t>(size.y);
            glUseProgram(mProgram);
            // update vertex array
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBindVertexArray(vao);
            if(mType == NodeType::Image) {
                std::array vertices{
                    Vertex{ Vec2{ base.x, base.y + size.y }, Vec2{ 0.0, 0.0 } },                      // left-bottom
                    Vertex{ Vec2{ base.x, base.y }, Vec2{ 0.0, uniformSize.y } },                     // left-top
                    Vertex{ Vec2{ base.x + size.x, base.y }, Vec2{ uniformSize.x, uniformSize.y } },  // right-top
                    Vertex{ Vec2{ base.x + size.x, base.y + size.y }, Vec2{ uniformSize.x, 0.0 } },   // right-bottom
                };
                for(auto& [pos, coord] : vertices) {
                    pos.x = pos.x / fbSize.x * 2.0f - 1.0f;
                    pos.y = 1.0f - pos.y / fbSize.y * 2.0f;
                }
                glBufferData(GL_ARRAY_BUFFER, 4 * sizeof(Vertex), vertices.data(), GL_STREAM_DRAW);
            } else {
                // For flipped Y cubemaps, swap +Y (idx=2) and -Y (idx=3) face geometry
                // AND flip Y coordinates to match ShaderToy's vflip behavior
                std::array vertices{
                    VertexCubeMap{ Vec2{ base.x, base.y + size.y }, Vec2{ 0.0, 0.0 },
                                   cubeMapVertexPos[cubeMapVertexIndex[idx][0]] },  // left-bottom
                    VertexCubeMap{ Vec2{ base.x, base.y }, Vec2{ 0.0, uniformSize.y },
                                   cubeMapVertexPos[cubeMapVertexIndex[idx][1]] },  // left-top
                    VertexCubeMap{ Vec2{ base.x + size.x, base.y }, Vec2{ uniformSize.x, uniformSize.y },
                                   cubeMapVertexPos[cubeMapVertexIndex[idx][2]] },  // right-top
                    VertexCubeMap{ Vec2{ base.x + size.x, base.y + size.y }, Vec2{ uniformSize.x, 0.0 },
                                   cubeMapVertexPos[cubeMapVertexIndex[idx][3]] },  // right-bottom
                };

                // For Y-flipped cubemaps, also flip Y coordinates of all faces
                // to match ShaderToy's UNPACK_FLIP_Y_WEBGL behavior
                // CubeMaps are always Y-flipped
                for(auto& [pos, coord, point] : vertices) {
                    // Flip the 3D point's Y coordinate
                    point[1] = -point[1];
                }

                for(auto& [pos, coord, point] : vertices) {
                    pos.x = pos.x / fbSize.x * 2.0f - 1.0f;
                    pos.y = 1.0f - pos.y / fbSize.y * 2.0f;
                }
                glBufferData(GL_ARRAY_BUFFER, 4 * sizeof(VertexCubeMap), vertices.data(), GL_STREAM_DRAW);
            }

            // update texture
            for(auto& channel : mChannels) {
                if(mLocationChannelResolution[channel.slot] != -1) {
                    if(channel.tex.type != TexType::Tex3D) {
                        const auto texSize =
                            channel.size.value_or(channel.tex.type == TexType::CubeMap ? cubeMapSize : canvasSize);
                        glUniform3f(mLocationChannelResolution[channel.slot], texSize.x, texSize.y, 1.0f);
                    } else {
                        const auto x = channel.size->x;
                        glUniform3f(mLocationChannelResolution[channel.slot], x, x, x);
                    }
                }
                if(mLocationChannelTime[channel.slot] != -1)
                    glUniform1f(mLocationChannelTime[channel.slot], uniform.time);
            }
            for(auto& channel : mChannels) {
                if(mLocationChannel[channel.slot] == -1)
                    continue;
                glUniform1i(mLocationChannel[channel.slot], static_cast<GLint>(channel.slot));
                glActiveTexture(GL_TEXTURE0 + channel.slot);
                const auto type = channel.tex.type == TexType::CubeMap ? GL_TEXTURE_CUBE_MAP :
                    channel.tex.type == TexType::Tex2D                 ? GL_TEXTURE_2D :
                                                                         GL_TEXTURE_3D;
                glBindTexture(type, static_cast<GLuint>(channel.tex.get()));
                // updating
                if(glGetError() != GL_NO_ERROR)
                    continue;
                if(channel.filter == Filter::Mipmap)
                    glGenerateMipmap(type);
                glBindSampler(channel.slot, mSamplers[channel.slot]);
            }

            // update uniform
            if(mLocationResolution != -1)
                glUniform3f(mLocationResolution, uniformSize.x, uniformSize.y, 0.0f);
            if(mLocationTime != -1)
                glUniform1f(mLocationTime, uniform.time);
            if(mLocationTimeDelta != -1)
                glUniform1f(mLocationTimeDelta, uniform.timeDelta);
            if(mLocationFrameRate != -1)
                glUniform1f(mLocationFrameRate, uniform.frameRate);
            if(mLocationFrame != -1)
                glUniform1i(mLocationFrame, uniform.frame);
            if(mLocationIteration != -1)
                glUniform1i(mLocationIteration, 0);
            if(mLocationMouse != -1)
                glUniform4f(mLocationMouse, uniform.mouse.x, uniform.mouse.y, uniform.mouse.z, uniform.mouse.w);
            if(mLocationDate != -1)
                glUniform4f(mLocationDate, uniform.date.x, uniform.date.y, uniform.date.z, uniform.date.w);
            if(mLocationMusicBands != -1)
                glUniform4f(mLocationMusicBands, uniform.audioBands.x, uniform.audioBands.y, uniform.audioBands.z,
                            uniform.audioBands.w);
            if(mLocationMusicHits != -1)
                glUniform4f(mLocationMusicHits, uniform.audioHits.x, uniform.audioHits.y, uniform.audioHits.z,
                            uniform.audioHits.w);
            if(mLocationMusicBeat != -1)
                glUniform4f(mLocationMusicBeat, uniform.audioBeat.x, uniform.audioBeat.y, uniform.audioBeat.z,
                            uniform.audioBeat.w);
            if(mLocationMusicStereo != -1)
                glUniform4f(mLocationMusicStereo, uniform.audioStereo.x, uniform.audioStereo.y, uniform.audioStereo.z,
                            uniform.audioStereo.w);
            if(mLocationMusicStructure != -1)
                glUniform4f(mLocationMusicStructure, uniform.audioStructure.x, uniform.audioStructure.y, uniform.audioStructure.z,
                            uniform.audioStructure.w);
            if(mLocationMusicMeta != -1)
                glUniform4f(mLocationMusicMeta, uniform.audioMeta.x, uniform.audioMeta.y, uniform.audioMeta.z,
                            uniform.audioMeta.w);
            applyCustomUniforms(uniform);

            glDrawArrays(GL_TRIANGLE_FAN, 0, 4);
            if(!mStorageBuffers.empty())
                glMemoryBarrier(GL_SHADER_STORAGE_BARRIER_BIT);
            if(buffer) {
                if(useMrt)
                    glBindFramebuffer(GL_FRAMEBUFFER, GL_NONE);
                else
                    buffer->unbind();
            }
        }

        for(const auto& [binding, id] : mStorageBuffers) {
            SHADERTOY_UNUSED(id);
            glBindBufferBase(GL_SHADER_STORAGE_BUFFER, binding, 0);
        }
        for(const auto& channel : mChannels)
            glBindSampler(channel.slot, 0);
        glActiveTexture(GL_TEXTURE0);  // restore
    }
};

class GLTextureObject final : public TextureObject {
    GLuint mTex{};
    Vec2 mSize;

public:
    GLTextureObject(const uint32_t width, const uint32_t height, const uint32_t* data)
        : mSize{ static_cast<float>(width), static_cast<float>(height) } {
        glGenTextures(1, &mTex);
        glBindTexture(GL_TEXTURE_2D, mTex);
        if(data) {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, static_cast<GLsizei>(width), static_cast<GLsizei>(height), 0, GL_RGBA,
                         GL_UNSIGNED_BYTE, data);  // R8G8B8A8
            glGenerateMipmap(GL_TEXTURE_2D);
        }
        glBindTexture(GL_TEXTURE_2D, GL_NONE);
    }
    GLTextureObject(const GLTextureObject&) = delete;
    GLTextureObject(GLTextureObject&&) = delete;
    GLTextureObject& operator=(const GLTextureObject&) = delete;
    GLTextureObject& operator=(GLTextureObject&&) = delete;
    ~GLTextureObject() override {
        glDeleteTextures(1, &mTex);
    }
    void update(const uint32_t width, const uint32_t height, const uint32_t* data) {
        if(!data)
            throw Error("Texture update data is null");
        if(width != static_cast<uint32_t>(mSize.x) || height != static_cast<uint32_t>(mSize.y))
            throw Error("Texture update dimensions do not match the declared texture");
        glBindTexture(GL_TEXTURE_2D, mTex);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, static_cast<GLsizei>(width), static_cast<GLsizei>(height), GL_RGBA,
                        GL_UNSIGNED_BYTE, data);
        glBindTexture(GL_TEXTURE_2D, GL_NONE);
    }
    [[nodiscard]] TextureId getTexture() const override {
        return mTex;
    }
    [[nodiscard]] Vec2 size() const override {
        return mSize;
    }
};

std::unique_ptr<TextureObject> loadTexture(uint32_t width, uint32_t height, const uint32_t* data) {
    return std::make_unique<GLTextureObject>(width, height, data);
}

class GLCubeMapObject final : public TextureObject {
    GLuint mTex{};
    Vec2 mSize;

public:
    GLCubeMapObject(const uint32_t size, const uint32_t* data) : mSize{ static_cast<float>(size), static_cast<float>(size) } {
        glGenTextures(1, &mTex);
        glBindTexture(GL_TEXTURE_CUBE_MAP, mTex);
        assert(data);
        const auto facePixels = checkedSizeProduct({ size, size }, "Cubemap face");
        for(std::size_t idx = 0; idx < 6; ++idx) {
            glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + static_cast<GLenum>(idx), 0, GL_RGBA, static_cast<GLsizei>(size),
                         static_cast<GLsizei>(size), 0, GL_RGBA, GL_UNSIGNED_BYTE, data + idx * facePixels);  // R8G8B8A8
        }
        glGenerateMipmap(GL_TEXTURE_CUBE_MAP);
        glBindTexture(GL_TEXTURE_CUBE_MAP, GL_NONE);
    }
    GLCubeMapObject(const GLCubeMapObject&) = delete;
    GLCubeMapObject(GLCubeMapObject&&) = delete;
    GLTextureObject& operator=(const GLCubeMapObject&) = delete;
    GLCubeMapObject& operator=(GLCubeMapObject&&) = delete;
    ~GLCubeMapObject() override {
        glDeleteTextures(1, &mTex);
    }
    [[nodiscard]] TextureId getTexture() const override {
        return mTex;
    }
    [[nodiscard]] Vec2 size() const override {
        return mSize;
    }
};

std::unique_ptr<TextureObject> loadCubeMap(uint32_t size, const uint32_t* data) {
    return std::make_unique<GLCubeMapObject>(size, data);
}

class GLVolumeObject final : public TextureObject {
    GLuint mTex{};
    Vec2 mSize;

public:
    GLVolumeObject(uint32_t size, uint32_t channels, const uint8_t* data)
        : mSize{ static_cast<float>(size), static_cast<float>(size) } {
        glGenTextures(1, &mTex);
        glBindTexture(GL_TEXTURE_3D, mTex);
        assert(data);
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_BASE_LEVEL, 0);
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAX_LEVEL, static_cast<GLint>(std::log2(size)));
        GLenum internalFormat = channels == 1 ? GL_R8 : GL_RGBA;
        GLenum format = channels == 1 ? GL_RED : GL_RGBA;
        const ScopedPixelStoreAlignment tightlyPacked(GL_UNPACK_ALIGNMENT, 1);
        glTexImage3D(GL_TEXTURE_3D, 0, internalFormat, static_cast<GLsizei>(size), static_cast<GLsizei>(size),
                     static_cast<GLsizei>(size), 0, format, GL_UNSIGNED_BYTE, data);  // R8G8B8A8
        glGenerateMipmap(GL_TEXTURE_3D);
        glBindTexture(GL_TEXTURE_3D, GL_NONE);
    }
    GLVolumeObject(const GLVolumeObject&) = delete;
    GLVolumeObject(GLVolumeObject&&) = delete;
    GLVolumeObject& operator=(const GLVolumeObject&) = delete;
    GLVolumeObject& operator=(GLVolumeObject&&) = delete;
    ~GLVolumeObject() override {
        glDeleteTextures(1, &mTex);
    }
    [[nodiscard]] TextureId getTexture() const override {
        return mTex;
    }
    [[nodiscard]] Vec2 size() const override {
        return mSize;
    }
};

std::unique_ptr<TextureObject> loadVolume(uint32_t size, uint32_t channels, const uint8_t* data) {
    return std::make_unique<GLVolumeObject>(size, channels, data);
}

struct DynamicTexture final {
    std::unique_ptr<GLTextureObject> tex;
    std::vector<uint32_t> data;
    std::function<void(uint32_t*)> update;
};

class OpenGLPipeline final : public Pipeline {
    GLuint mVAOImage{};
    GLuint mVAOCubeMap{};
    GLuint mVBO{};
    std::vector<std::unique_ptr<FrameBuffer>> mFrameBuffers;
    std::vector<std::unique_ptr<GLCubeMapRenderTarget>> mCubeMapRenderTargets;
    std::vector<std::unique_ptr<RenderPass>> mRenderPasses;
    std::vector<DynamicTexture> mDynamicTextures;
    std::vector<std::unique_ptr<TextureObject>> mTextures;
    std::unordered_map<std::string, GLTextureObject*> mNamedTextures;
    std::vector<GLuint> mStorageBuffers;
    std::unordered_map<std::string, std::pair<GLuint, uint64_t>> mNamedStorageBuffers;
    AudioInput mAudioInput;
    KeyboardInput mKeyboardInput;
    struct PendingPassTiming final {
        std::string name;
        std::array<GLuint, 3> queries{};
        uint64_t completionWaitNanoseconds{};
        bool computePass{};
        bool completionObserved{};
        uint32_t width{};
        uint32_t height{};
    };

    bool mProfilingEnabled{};
    bool mProfilingSyncPerPass{ true };
    std::vector<PendingPassTiming> mPendingPassTimings;
    std::vector<PassTiming> mLastPassTimings;
    std::vector<PassProfileSample> mLastPassProfileSamples;
    std::array<GLuint, 2> mFrameProfileQueries{};
    bool mFrameProfileQueriesAllocated{};
    uint64_t mLastFrameGpuNanoseconds{};
    uint64_t mLastFrameGpuTimestampNanoseconds{};

    void clearProfilingQueries() {
        for(const auto& pending : mPendingPassTimings)
            glDeleteQueries(static_cast<GLsizei>(pending.queries.size()), pending.queries.data());
        mPendingPassTimings.clear();
    }

    void clearFrameProfilingQueries() {
        if(mFrameProfileQueriesAllocated) {
            glDeleteQueries(static_cast<GLsizei>(mFrameProfileQueries.size()), mFrameProfileQueries.data());
            mFrameProfileQueries = {};
            mFrameProfileQueriesAllocated = false;
        }
    }

    void beginProfileFrame() {
        mLastPassTimings.clear();
        mLastPassProfileSamples.clear();
        mLastFrameGpuNanoseconds = 0;
        mLastFrameGpuTimestampNanoseconds = 0;
        clearProfilingQueries();
        clearFrameProfilingQueries();
        if(mProfilingEnabled) {
            glGenQueries(static_cast<GLsizei>(mFrameProfileQueries.size()), mFrameProfileQueries.data());
            mFrameProfileQueriesAllocated = true;
            glQueryCounter(mFrameProfileQueries[0], GL_TIMESTAMP);
        }
    }

    void finishProfileFrame() {
        if(!mProfilingEnabled)
            return;

        if(mFrameProfileQueriesAllocated)
            glQueryCounter(mFrameProfileQueries[1], GL_TIMESTAMP);

        mLastPassTimings.reserve(mPendingPassTimings.size());
        mLastPassProfileSamples.reserve(mPendingPassTimings.size());
        uint64_t totalNanoseconds = 0;
        for(const auto& pending : mPendingPassTimings) {
            GLuint64 start{};
            GLuint64 executionEnd{};
            GLuint64 attributedEnd{};
            glGetQueryObjectui64v(pending.queries[0], GL_QUERY_RESULT, &start);
            glGetQueryObjectui64v(pending.queries[1], GL_QUERY_RESULT, &executionEnd);
            glGetQueryObjectui64v(pending.queries[2], GL_QUERY_RESULT, &attributedEnd);
            const auto execution =
                executionEnd >= start ? static_cast<uint64_t>(executionEnd - start) : 0U;
            const auto attributed =
                attributedEnd >= start ? static_cast<uint64_t>(attributedEnd - start) : 0U;
            totalNanoseconds += attributed;

            // A timer interval that is tiny compared with the isolated completion
            // wait has outrun asynchronous work and cannot be treated as an
            // execution measurement. Keep it, but mark it invalid.
            constexpr uint64_t minRelevantWaitNanoseconds = 50'000U;
            const bool timerOutranWork = pending.completionObserved &&
                pending.completionWaitNanoseconds >= minRelevantWaitNanoseconds &&
                execution < pending.completionWaitNanoseconds / 4U;
            // Without an isolated completion observation, a graphics-queue timer
            // cannot prove that an asynchronous compute dispatch has finished.
            const bool unverifiedAsyncCompute = pending.computePass && !pending.completionObserved;
            const bool sampleValid = execution > 0U && !timerOutranWork && !unverifiedAsyncCompute;

            mLastPassTimings.push_back(PassTiming{
                pending.name,
                attributed,
                pending.width,
                pending.height,
            });
            mLastPassProfileSamples.push_back(PassProfileSample{
                pending.name,
                execution,
                attributed,
                pending.completionWaitNanoseconds,
                pending.width,
                pending.height,
                sampleValid,
            });
        }
        mLastFrameGpuNanoseconds = totalNanoseconds;

        if(mFrameProfileQueriesAllocated) {
            GLuint64 frameStart{};
            GLuint64 frameEnd{};
            glGetQueryObjectui64v(mFrameProfileQueries[0], GL_QUERY_RESULT, &frameStart);
            glGetQueryObjectui64v(mFrameProfileQueries[1], GL_QUERY_RESULT, &frameEnd);
            mLastFrameGpuTimestampNanoseconds =
                frameEnd >= frameStart ? static_cast<uint64_t>(frameEnd - frameStart) : 0U;
        }

        clearProfilingQueries();
        clearFrameProfilingQueries();
    }

    static void waitForProfiledGpuCompletion() {
        // Timer timestamps may execute on a graphics queue while compute remains
        // outstanding on another engine. A full completion wait before the end
        // timestamp is intentionally intrusive, but it makes the attribution
        // boundary unambiguous across drivers.
        glFinish();
    }

    void renderProfiled(RenderPass& pass, const Vec2 frameBufferSize, const Vec2 clipMin, const Vec2 clipMax,
                        const Vec2 size, const ShaderToyUniform& uniform) {
        if(!mProfilingEnabled) {
            pass.render(frameBufferSize, clipMin, clipMax, size, uniform,
                        pass.getType() == NodeType::Image ? mVAOImage : mVAOCubeMap, mVBO);
            return;
        }

        PendingPassTiming pending;
        pending.name = std::string(pass.getName());
        glGenQueries(static_cast<GLsizei>(pending.queries.size()), pending.queries.data());
        glQueryCounter(pending.queries[0], GL_TIMESTAMP);
        pass.render(frameBufferSize, clipMin, clipMax, size, uniform,
                    pass.getType() == NodeType::Image ? mVAOImage : mVAOCubeMap, mVBO);

        // Close an uncontaminated timestamp interval before any CPU completion
        // wait. On drivers where asynchronous compute outruns this timestamp the
        // sample is retained but marked invalid below.
        glQueryCounter(pending.queries[1], GL_TIMESTAMP);
        pending.computePass = pass.getType() == NodeType::Compute;

        if(mProfilingSyncPerPass) {
            const auto waitStarted = std::chrono::steady_clock::now();
            waitForProfiledGpuCompletion();
            const auto waitFinished = std::chrono::steady_clock::now();
            pending.completionWaitNanoseconds =
                static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::nanoseconds>(
                                          waitFinished - waitStarted)
                                          .count());
            pending.completionObserved = true;
        }

        // Legacy attributed interval. In sync-per-pass mode it intentionally
        // spans completion for compatibility; in normal mode it is effectively
        // the raw timer interval and no host completion wait is inserted.
        glQueryCounter(pending.queries[2], GL_TIMESTAMP);
        pending.width = pass.lastWidth();
        pending.height = pass.lastHeight();
        mPendingPassTimings.push_back(std::move(pending));

        // Diagnostic mode retires the attribution timestamp before the next pass.
        // This second wait is outside every reported pass interval.
        if(mProfilingSyncPerPass)
            glFinish();
    }

    static uint8_t toByte(const float value) {
        return static_cast<uint8_t>(std::lround(std::clamp(value, 0.0f, 1.0f) * 255.0f));
    }

    static float resample(const std::vector<float>& values, const float position) {
        if(values.empty())
            return 0.0f;
        if(values.size() == 1)
            return values.front();
        const float scaled = std::clamp(position, 0.0f, 1.0f) * static_cast<float>(values.size() - 1);
        const auto lo = static_cast<size_t>(scaled);
        const auto hi = std::min(lo + 1, values.size() - 1);
        const float fraction = scaled - static_cast<float>(lo);
        return values[lo] + (values[hi] - values[lo]) * fraction;
    }

    void updateKeyboardTexture(uint32_t* data) const {
        const auto& pixels = mKeyboardInput.pixels();
        std::copy(pixels.begin(), pixels.end(), data);
    }

    void updateAudioTexture(uint32_t* data) const {
        for(uint32_t x = 0; x < AudioInput::TextureWidth; ++x) {
            const float position = static_cast<float>(x) / static_cast<float>(AudioInput::TextureWidth - 1);
            const float spectrum = mAudioInput.available ? resample(mAudioInput.spectrum, position) : 0.0f;
            const float waveform = mAudioInput.available ? resample(mAudioInput.waveform, position) * 0.5f + 0.5f : 0.5f;
            const uint8_t spectrumByte = toByte(spectrum);
            const uint8_t waveformByte = toByte(waveform);
            data[x] = static_cast<uint32_t>(spectrumByte) | (static_cast<uint32_t>(spectrumByte) << 8U) |
                (static_cast<uint32_t>(spectrumByte) << 16U) | 0xff000000U;
            data[AudioInput::TextureWidth + x] = static_cast<uint32_t>(waveformByte) |
                (static_cast<uint32_t>(waveformByte) << 8U) | (static_cast<uint32_t>(waveformByte) << 16U) | 0xff000000U;
        }
    }

public:
    explicit OpenGLPipeline() {
        glGenBuffers(1, &mVBO);
        glBindBuffer(GL_ARRAY_BUFFER, mVBO);

        glGenVertexArrays(1, &mVAOImage);
        glBindVertexArray(mVAOImage);
        glEnableVertexAttribArray(0);
        // NOLINTNEXTLINE(performance-no-int-to-ptr)
        glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(Vertex), reinterpret_cast<void*>(offsetof(Vertex, pos)));
        glEnableVertexAttribArray(1);
        // NOLINTNEXTLINE(performance-no-int-to-ptr)
        glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(Vertex), reinterpret_cast<void*>(offsetof(Vertex, coord)));
        glBindVertexArray(0);

        glGenVertexArrays(1, &mVAOCubeMap);
        glBindVertexArray(mVAOCubeMap);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(VertexCubeMap),
                              // NOLINTNEXTLINE(performance-no-int-to-ptr)
                              reinterpret_cast<void*>(offsetof(VertexCubeMap, pos)));
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(VertexCubeMap),
                              // NOLINTNEXTLINE(performance-no-int-to-ptr)
                              reinterpret_cast<void*>(offsetof(VertexCubeMap, coord)));
        glEnableVertexAttribArray(2);
        glVertexAttribPointer(2, 3, GL_FLOAT, GL_FALSE, sizeof(VertexCubeMap),
                              // NOLINTNEXTLINE(performance-no-int-to-ptr)
                              reinterpret_cast<void*>(offsetof(VertexCubeMap, point)));
        glBindVertexArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, GL_NONE);
    }
    OpenGLPipeline(const OpenGLPipeline&) = delete;
    OpenGLPipeline(OpenGLPipeline&&) = delete;
    OpenGLPipeline& operator=(const OpenGLPipeline&) = delete;
    OpenGLPipeline& operator=(OpenGLPipeline&&) = delete;
    ~OpenGLPipeline() override {
        clearProfilingQueries();
        clearFrameProfilingQueries();
        if(!mStorageBuffers.empty())
            glDeleteBuffers(static_cast<GLsizei>(mStorageBuffers.size()), mStorageBuffers.data());
        glDeleteVertexArrays(1, &mVAOImage);
        glDeleteVertexArrays(1, &mVAOCubeMap);
        glDeleteBuffers(1, &mVBO);
    }

    FrameBuffer* createFrameBuffer(const RenderFormat format) override {
        mFrameBuffers.push_back(std::make_unique<GLFrameBuffer>(format));
        return mFrameBuffers.back().get();
    }
    GLCubeMapRenderTarget* createCubeMapRenderTarget() {
        mCubeMapRenderTargets.push_back(std::make_unique<GLCubeMapRenderTarget>());
        return mCubeMapRenderTargets.back().get();
    }
    std::vector<FrameBuffer*> createCubeMapFrameBuffer() override {
        std::vector<FrameBuffer*> buffers;
        const auto target = createCubeMapRenderTarget();

        for(uint32_t idx = 0; idx < 6; ++idx) {
            mFrameBuffers.push_back(std::make_unique<GLCubeMapFrameBuffer>(target->getTexture(), idx));
            buffers.emplace_back(mFrameBuffers.back().get());
        }
        return buffers;
    }

    BufferId createStorageBuffer(std::string name, const uint64_t size) override {
        if(!GLAD_GL_VERSION_4_3)
            throw Error("Shader storage buffers require OpenGL 4.3 or newer");
        if(name.empty())
            throw Error("Storage buffer name must not be empty");
        if(mNamedStorageBuffers.contains(name))
            throw Error("Duplicate storage buffer allocation: " + name);
        if(size == 0 || size > static_cast<uint64_t>(std::numeric_limits<GLsizeiptr>::max()))
            throw Error("Storage buffer size is outside the OpenGL range");
        GLuint buffer{};
        glGenBuffers(1, &buffer);
        glBindBuffer(GL_SHADER_STORAGE_BUFFER, buffer);
        glBufferData(GL_SHADER_STORAGE_BUFFER, static_cast<GLsizeiptr>(size), nullptr, GL_DYNAMIC_COPY);
        const uint8_t zero = 0;
        glClearBufferData(GL_SHADER_STORAGE_BUFFER, GL_R8UI, GL_RED_INTEGER, GL_UNSIGNED_BYTE, &zero);
        glBindBuffer(GL_SHADER_STORAGE_BUFFER, 0);
        mStorageBuffers.push_back(buffer);
        mNamedStorageBuffers.emplace(std::move(name), std::pair<GLuint, uint64_t>{ buffer, size });
        return buffer;
    }

    void addPass(std::string name, const std::string& src, NodeType type, std::vector<DoubleBufferedFB> target,
                 std::vector<Channel> channels, std::optional<Vec2> fixedResolution, bool clampOutput,
                 const RenderFormat format, std::vector<RenderFormat> extraFormats, const uint32_t iterations,
                 const uint32_t localSizeX, const uint32_t localSizeY, const uint32_t localSizeZ,
                 std::vector<std::pair<uint32_t, BufferId>> storageBuffers, std::optional<MeshConfig> mesh) override {
        mRenderPasses.push_back(std::make_unique<RenderPass>(
            std::move(name), src, type, std::move(target), std::move(channels), fixedResolution, clampOutput, format,
            std::move(extraFormats), iterations, localSizeX, localSizeY, localSizeZ, std::move(storageBuffers),
            std::move(mesh)));
    }

    void reloadPassSource(const std::string_view passName, const std::string& src) override {
        const auto selected = std::find_if(mRenderPasses.begin(), mRenderPasses.end(),
                                           [passName](const auto& pass) { return pass->getName() == passName; });
        if(selected == mRenderPasses.end())
            throw Error("Unknown shader pass: " + std::string(passName));
        (*selected)->reloadSource(src);
    }

    void setWireframePass(const std::string_view passName, const bool enabled) override {
        const auto selected = std::find_if(mRenderPasses.begin(), mRenderPasses.end(),
                                           [passName](const auto& pass) { return pass->getName() == passName; });
        if(selected == mRenderPasses.end())
            throw Error("Unknown shader pass: " + std::string(passName));
        if(!(*selected)->isMesh())
            throw Error("Wireframe overlay is only supported for mesh passes: " + std::string(passName));
        (*selected)->setWireframe(enabled);
    }

    void setProfilingEnabled(const bool enabled) override {
        mProfilingEnabled = enabled;
        if(!enabled) {
            clearProfilingQueries();
            clearFrameProfilingQueries();
            mLastPassTimings.clear();
            mLastPassProfileSamples.clear();
            mLastFrameGpuNanoseconds = 0;
            mLastFrameGpuTimestampNanoseconds = 0;
        }
    }

    void setProfilingSyncPerPass(const bool enabled) override {
        mProfilingSyncPerPass = enabled;
    }

    [[nodiscard]] const std::vector<PassTiming>& lastPassTimings() const override {
        return mLastPassTimings;
    }

    [[nodiscard]] const std::vector<PassProfileSample>& lastPassProfileSamples() const override {
        return mLastPassProfileSamples;
    }

    [[nodiscard]] uint64_t lastFrameGpuNanoseconds() const override {
        return mLastFrameGpuNanoseconds;
    }

    [[nodiscard]] uint64_t lastFrameGpuTimestampNanoseconds() const override {
        return mLastFrameGpuTimestampNanoseconds;
    }

    void render(const Vec2 frameBufferSize, const Vec2 clipMin, const Vec2 clipMax, Vec2 size,
                const ShaderToyUniform& uniform) override {
        GLint callerFramebuffer = 0;
        glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &callerFramebuffer);
        for(auto& [tex, data, update] : mDynamicTextures) {
            update(data.data());
            const auto texId = static_cast<GLuint>(tex->getTexture());
            glBindTexture(GL_TEXTURE_2D, texId);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, static_cast<GLsizei>(tex->size().x), static_cast<GLsizei>(tex->size().y), 0,
                         GL_RGBA, GL_UNSIGNED_BYTE, data.data());  // R8G8B8A8
            glBindTexture(GL_TEXTURE_2D, GL_NONE);
        }
        beginProfileFrame();
        for(const auto& pass : mRenderPasses) {
            if(!pass->hasOffscreenTarget())
                glBindFramebuffer(GL_DRAW_FRAMEBUFFER, static_cast<GLuint>(callerFramebuffer));
            renderProfiled(*pass, frameBufferSize, clipMin, clipMax, size, uniform);
        }
        for(const auto& pass : mRenderPasses) {
            if(pass->wireframeEnabled())
                pass->renderWireframeOverlay(static_cast<GLuint>(callerFramebuffer), frameBufferSize, clipMin, clipMax);
        }
        finishProfileFrame();
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, static_cast<GLuint>(callerFramebuffer));
    }

    TextureId createDynamicTexture(uint32_t width, uint32_t height, std::function<void(uint32_t*)> update) {
        mDynamicTextures.push_back(DynamicTexture{
            std::make_unique<GLTextureObject>(width, height, nullptr),
            std::vector<uint32_t>(checkedSizeProduct({ width, height }, "Dynamic texture")),
            std::move(update),
        });
        return mDynamicTextures.back().tex->getTexture();
    }
    TextureId createTexture(std::string name, const uint32_t width, const uint32_t height,
                            const uint32_t* data) override {
        if(name.empty())
            throw Error("Texture name must not be empty");
        if(mNamedTextures.contains(name))
            throw Error("Duplicate texture allocation: " + name);
        auto texture = std::make_unique<GLTextureObject>(width, height, data);
        auto* raw = texture.get();
        const auto id = raw->getTexture();
        mTextures.push_back(std::move(texture));
        mNamedTextures.emplace(std::move(name), raw);
        return id;
    }
    void updateTexture(const std::string_view name, const uint32_t width, const uint32_t height,
                       const uint32_t* data) override {
        const auto found = mNamedTextures.find(std::string(name));
        if(found == mNamedTextures.end())
            throw Error("Unknown texture: " + std::string(name));
        found->second->update(width, height, data);
    }
    TextureId createCubeMap(const uint32_t size, const uint32_t* data) override {
        mTextures.push_back(std::make_unique<GLCubeMapObject>(size, data));
        return mTextures.back()->getTexture();
    }
    TextureId createVolume(const uint32_t size, const uint32_t channels, const uint8_t* data) override {
        mTextures.push_back(std::make_unique<GLVolumeObject>(size, channels, data));
        return mTextures.back()->getTexture();
    }
    TextureId createKeyboardTexture() override {
        return createDynamicTexture(static_cast<uint32_t>(KeyboardInput::KeyCount), static_cast<uint32_t>(KeyboardInput::Rows),
                                    [this](uint32_t* data) { updateKeyboardTexture(data); });
    }
    TextureId createAudioTexture() override {
        return createDynamicTexture(AudioInput::TextureWidth, AudioInput::TextureHeight,
                                    [this](uint32_t* data) { updateAudioTexture(data); });
    }
    void setKeyboardInput(const KeyboardInput& input) override {
        mKeyboardInput = input;
    }
    void setAudioInput(const AudioInput& input) override {
        mAudioInput = input;
    }
    std::vector<uint8_t> renderToBuffer(Vec2 size, const ShaderToyUniform& uniform) override {
        const auto width = checkedPixelDimension(size.x, "Render width");
        const auto height = checkedPixelDimension(size.y, "Render height");

        // Update dynamic textures first, like in the regular render function
        for(auto& [tex, data, update] : mDynamicTextures) {
            update(data.data());
            const auto texId = static_cast<GLuint>(tex->getTexture());
            glBindTexture(GL_TEXTURE_2D, texId);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, static_cast<GLsizei>(tex->size().x), static_cast<GLsizei>(tex->size().y), 0,
                         GL_RGBA, GL_UNSIGNED_BYTE, data.data());  // R8G8B8A8
            glBindTexture(GL_TEXTURE_2D, GL_NONE);
        }

        auto fb = std::make_unique<GLFrameBuffer>();
        fb->bind(width, height);

        // Render all passes. Offscreen passes may bind and unbind their own framebuffer,
        // so explicitly restore the capture framebuffer before any pass that renders
        // directly to the caller target.
        beginProfileFrame();
        for(const auto& pass : mRenderPasses) {
            if(!pass->hasOffscreenTarget())
                fb->bind(width, height);
            renderProfiled(*pass, size, Vec2{ 0, 0 }, size, size, uniform);
        }
        fb->bind(width, height);
        GLint captureFramebuffer = 0;
        glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &captureFramebuffer);
        for(const auto& pass : mRenderPasses) {
            if(pass->wireframeEnabled())
                pass->renderWireframeOverlay(static_cast<GLuint>(captureFramebuffer), size, Vec2{ 0, 0 }, size);
        }
        finishProfileFrame();

        fb->bind(width, height);
        std::vector<uint8_t> buffer(checkedSizeProduct({ width, height, 3U }, "Render readback"));
        const ScopedPixelStoreAlignment tightlyPacked(GL_PACK_ALIGNMENT, 1);
        glReadPixels(0, 0, static_cast<GLsizei>(width), static_cast<GLsizei>(height), GL_RGB, GL_UNSIGNED_BYTE, buffer.data());
        fb->unbind();
        return buffer;
    }

    std::vector<uint8_t> snapshotPassRgb(const std::string_view passName, const uint32_t output) override {
        const auto selected = std::find_if(mRenderPasses.begin(), mRenderPasses.end(),
                                           [passName](const auto& pass) { return pass->getName() == passName; });
        if(selected == mRenderPasses.end())
            throw Error("Unknown shader pass: " + std::string(passName));
        return (*selected)->readRgb(output);
    }

    std::vector<float> snapshotPassRgba32f(const std::string_view passName, const uint32_t output) override {
        const auto selected = std::find_if(mRenderPasses.begin(), mRenderPasses.end(),
                                           [passName](const auto& pass) { return pass->getName() == passName; });
        if(selected == mRenderPasses.end())
            throw Error("Unknown shader pass: " + std::string(passName));
        return (*selected)->readRgba32f(output);
    }

    std::vector<uint8_t> snapshotStorageBuffer(const std::string_view name) override {
        const auto found = mNamedStorageBuffers.find(std::string(name));
        if(found == mNamedStorageBuffers.end())
            throw Error("Unknown storage buffer: " + std::string(name));
        const auto [buffer, size] = found->second;
        if(size > static_cast<uint64_t>(std::numeric_limits<size_t>::max()))
            throw Error("Storage buffer is too large to read back");
        std::vector<uint8_t> data(static_cast<size_t>(size));
        glBindBuffer(GL_SHADER_STORAGE_BUFFER, buffer);
        glGetBufferSubData(GL_SHADER_STORAGE_BUFFER, 0, static_cast<GLsizeiptr>(size), data.data());
        glBindBuffer(GL_SHADER_STORAGE_BUFFER, 0);
        return data;
    }

    void restoreStorageBuffer(const std::string_view name, const uint8_t* data, const uint64_t size) override {
        const auto found = mNamedStorageBuffers.find(std::string(name));
        if(found == mNamedStorageBuffers.end())
            throw Error("Unknown storage buffer: " + std::string(name));
        if(!data)
            throw Error("Storage restore data is null");
        const auto [buffer, expectedSize] = found->second;
        if(size != expectedSize)
            throw Error("Storage restore size does not match the declared buffer size");
        glBindBuffer(GL_SHADER_STORAGE_BUFFER, buffer);
        glBufferSubData(GL_SHADER_STORAGE_BUFFER, 0, static_cast<GLsizeiptr>(size), data);
        glBindBuffer(GL_SHADER_STORAGE_BUFFER, 0);
    }

    void overridePassRgba8(const std::string_view passName, const uint32_t width, const uint32_t height,
                           const uint8_t* data) override {
        const auto selected = std::find_if(mRenderPasses.begin(), mRenderPasses.end(),
                                           [passName](const auto& pass) { return pass->getName() == passName; });
        if(selected == mRenderPasses.end())
            throw Error("Unknown shader pass: " + std::string(passName));
        (*selected)->overrideRgba8(width, height, data);
    }

    void restorePassRgba32f(const std::string_view passName, const uint32_t width, const uint32_t height,
                            const float* data) override {
        const auto selected = std::find_if(mRenderPasses.begin(), mRenderPasses.end(),
                                           [passName](const auto& pass) { return pass->getName() == passName; });
        if(selected == mRenderPasses.end())
            throw Error("Unknown shader pass: " + std::string(passName));
        (*selected)->restoreRgba32f(width, height, data);
    }
};

std::unique_ptr<Pipeline> createPipeline() {
    // The embedding application owns the OpenGL context, while the renderer
    // owns its loader implementation. A compatible context must be current
    // before a document is compiled.
    if(gladLoadGL() == 0)
        throw Error("Failed to initialize OpenGL loader");
    return std::make_unique<OpenGLPipeline>();
}

SHADERTOY_NAMESPACE_END
