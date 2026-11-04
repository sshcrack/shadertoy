/*
    SPDX-License-Identifier: Apache-2.0
    Copyright 2023-2026 Yingwei Zheng and contributors
*/
#pragma once

#include "shadertoy/AudioInput.hpp"
#include "shadertoy/STTF.hpp"
#include "shadertoy/Types.hpp"

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

SHADERTOY_NAMESPACE_BEGIN

using TextureId = uintptr_t;
using BufferId = uintptr_t;

enum class TexType {
    Tex2D,
    Tex3D,
    CubeMap,
};

struct DoubleBufferedTex final {
    TextureId t1{};
    TextureId t2{};
    TexType type{ TexType::Tex2D };

    explicit DoubleBufferedTex(const TextureId t, const TexType texType) : t1{ t }, t2{ t }, type{ texType } {}
    DoubleBufferedTex(const TextureId t1Val, const TextureId t2Val, const TexType texType)
        : t1{ t1Val }, t2{ t2Val }, type{ texType } {}

    TextureId get() {
        std::swap(t1, t2);
        return t1;
    }
};

struct PassTiming final {
    std::string name;
    uint64_t gpuNanoseconds{};
    uint32_t width{};
    uint32_t height{};
};

struct PassProfileSample final {
    std::string name;
    uint64_t gpuExecutionNanoseconds{};
    uint64_t attributedNanoseconds{};
    uint64_t completionWaitNanoseconds{};
    uint32_t width{};
    uint32_t height{};
    bool sampleValid{};
};

struct ShaderToyUniform final {
    float time{};
    float timeDelta{};
    float frameRate{};
    int32_t frame{};
    Vec4 mouse;
    Vec4 date;
    Vec4 audioBands;
    Vec4 audioHits;
    Vec4 audioBeat;
    Vec4 audioStereo;
    Vec4 audioStructure;
    Vec4 audioMeta;
    const CustomUniformMap* customUniforms{};
};

class TextureObject {
public:
    TextureObject() = default;
    TextureObject(const TextureObject&) = delete;
    TextureObject(TextureObject&&) = delete;
    TextureObject& operator=(const TextureObject&) = delete;
    TextureObject& operator=(TextureObject&&) = delete;
    virtual ~TextureObject() = default;

    [[nodiscard]] virtual TextureId getTexture() const = 0;
    [[nodiscard]] virtual Vec2 size() const = 0;
};

class FrameBuffer {
public:
    FrameBuffer() = default;
    FrameBuffer(const FrameBuffer&) = delete;
    FrameBuffer(FrameBuffer&&) = delete;
    FrameBuffer& operator=(const FrameBuffer&) = delete;
    FrameBuffer& operator=(FrameBuffer&&) = delete;
    virtual ~FrameBuffer() = default;

    virtual void bind(uint32_t width, uint32_t height) = 0;
    virtual void unbind() = 0;
    [[nodiscard]] virtual TextureId getTexture() const = 0;
    [[nodiscard]] virtual std::vector<uint8_t> readRgb() = 0;
    [[nodiscard]] virtual std::vector<float> readRgba32f() = 0;
    virtual void writeRgba8(uint32_t width, uint32_t height, const uint8_t* data) = 0;
    virtual void writeRgba32f(uint32_t width, uint32_t height, const float* data) = 0;
};

struct DoubleBufferedFB final {
    FrameBuffer* t1{};
    FrameBuffer* t2{};

    explicit DoubleBufferedFB(FrameBuffer* t) : t1{ t }, t2{ t } {}
    DoubleBufferedFB(FrameBuffer* t1Val, FrameBuffer* t2Val) : t1{ t1Val }, t2{ t2Val } {}

    FrameBuffer* get() {
        std::swap(t1, t2);
        return t1;
    }
};

struct Channel final {
    uint32_t slot{};
    DoubleBufferedTex tex;
    Filter filter{ Filter::Linear };
    Wrap wrapMode{ Wrap::Repeat };
    std::optional<Vec2> size;
};

class Pipeline {
public:
    Pipeline() = default;
    Pipeline(const Pipeline&) = delete;
    Pipeline& operator=(const Pipeline&) = delete;
    Pipeline(Pipeline&&) = delete;
    Pipeline& operator=(Pipeline&&) = delete;
    virtual ~Pipeline() = default;

    virtual FrameBuffer* createFrameBuffer(RenderFormat format = RenderFormat::RGBA32F) = 0;
    virtual std::vector<FrameBuffer*> createCubeMapFrameBuffer() = 0;
    virtual BufferId createStorageBuffer(std::string name, uint64_t size) = 0;
    virtual void addPass(std::string name, const std::string& src, NodeType type, std::vector<DoubleBufferedFB> target,
                         std::vector<Channel> channels, std::optional<Vec2> fixedResolution, bool clampOutput,
                         RenderFormat format, std::vector<RenderFormat> extraFormats, uint32_t iterations, uint32_t localSizeX,
                         uint32_t localSizeY, uint32_t localSizeZ, std::vector<std::pair<uint32_t, BufferId>> storageBuffers,
                         std::optional<MeshConfig> mesh) = 0;
    virtual void reloadPassSource(std::string_view passName, const std::string& src) = 0;
    virtual void setWireframePass(std::string_view passName, bool enabled) = 0;
    virtual void render(Vec2 frameBufferSize, Vec2 clipMin, Vec2 clipMax, Vec2 size, const ShaderToyUniform& uniform) = 0;
    virtual void setProfilingEnabled(bool enabled) = 0;
    virtual void setProfilingSyncPerPass(bool enabled) = 0;
    [[nodiscard]] virtual const std::vector<PassTiming>& lastPassTimings() const = 0;
    [[nodiscard]] virtual const std::vector<PassProfileSample>& lastPassProfileSamples() const = 0;
    [[nodiscard]] virtual uint64_t lastFrameGpuNanoseconds() const = 0;
    [[nodiscard]] virtual uint64_t lastFrameGpuTimestampNanoseconds() const = 0;

    virtual TextureId createTexture(std::string name, uint32_t width, uint32_t height, const uint32_t* data) = 0;
    virtual void updateTexture(std::string_view name, uint32_t width, uint32_t height, const uint32_t* data) = 0;
    virtual TextureId createCubeMap(uint32_t size, const uint32_t* data) = 0;
    virtual TextureId createVolume(uint32_t size, uint32_t channels, const uint8_t* data) = 0;
    virtual TextureId createKeyboardTexture() = 0;
    virtual TextureId createAudioTexture() = 0;

    virtual void setKeyboardInput(const KeyboardInput& input) = 0;
    virtual void setAudioInput(const AudioInput& input) = 0;

    virtual std::vector<uint8_t> renderToBuffer(Vec2 size, const ShaderToyUniform& uniform) = 0;
    virtual std::vector<uint8_t> snapshotPassRgb(std::string_view passName, uint32_t output = 0) = 0;
    virtual std::vector<float> snapshotPassRgba32f(std::string_view passName, uint32_t output = 0) = 0;
    virtual std::vector<uint8_t> snapshotStorageBuffer(std::string_view name) = 0;
    virtual void restoreStorageBuffer(std::string_view name, const uint8_t* data, uint64_t size) = 0;
    virtual void overridePassRgba8(std::string_view passName, uint32_t width, uint32_t height, const uint8_t* data) = 0;
    virtual void restorePassRgba32f(std::string_view passName, uint32_t width, uint32_t height, const float* data) = 0;
};

std::unique_ptr<TextureObject> loadTexture(uint32_t width, uint32_t height, const uint32_t* data);
std::unique_ptr<TextureObject> loadCubeMap(uint32_t size, const uint32_t* data);
std::unique_ptr<TextureObject> loadVolume(uint32_t size, uint32_t channels, const uint8_t* data);
std::unique_ptr<Pipeline> createPipeline();

SHADERTOY_NAMESPACE_END
