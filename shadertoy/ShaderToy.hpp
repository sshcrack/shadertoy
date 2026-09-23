/*
    SPDX-License-Identifier: Apache-2.0
    Copyright 2023-2026 Yingwei Zheng and contributors
*/
#pragma once

#include "shadertoy/AudioInput.hpp"
#include "shadertoy/Backend.hpp"
#include "shadertoy/Importer.hpp"
#include "shadertoy/Result.hpp"
#include "shadertoy/STTF.hpp"
#include "shadertoy/Types.hpp"

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

SHADERTOY_NAMESPACE_BEGIN

/// Standalone ShaderToy runtime.
///
/// Runtime never creates a window or graphics context. The embedding
/// application owns the OpenGL context and must make it current before loading
/// or rendering a document.
class Runtime final {
public:
    Runtime();
    ~Runtime();

    Runtime(const Runtime&) = delete;
    Runtime(Runtime&&) = delete;
    Runtime& operator=(const Runtime&) = delete;
    Runtime& operator=(Runtime&&) = delete;

    Result<void> setDocument(ShaderDocument document);
    Result<void> loadSTTF(const std::string& path);
    Result<void> saveSTTF(const std::string& path) const;

    Result<void> loadImageShader(std::string name, std::string source, std::optional<uint32_t> audioChannel = std::nullopt);
    Result<void> loadFromShaderToy(std::string_view shaderUrlOrId);
    Result<void> loadFromShaderToyResponse(std::string_view shaderId, std::string_view responseBody);

    [[nodiscard]] const ShaderDocument* document() const noexcept;

    void tick(float frameRate = 60.0f);
    void tickFixed(float deltaSeconds, float frameRate = 60.0f);
    void pause();
    void resume();
    void resetTime();

    [[nodiscard]] bool isRunning() const noexcept;
    [[nodiscard]] bool isValid() const noexcept;
    [[nodiscard]] float time() const noexcept;
    [[nodiscard]] float timeDelta() const noexcept;
    [[nodiscard]] float frameRate() const noexcept;
    [[nodiscard]] float timeScale() const noexcept;
    void setTimeScale(float log2Scale) noexcept;

    void setMouseInput(const std::optional<MouseInput>& input);
    void setKeyboardInput(const KeyboardInput& input);
    void setAudioInput(const AudioInput& input);
    Result<void> setUniformFloats(std::string name, const float* values, uint32_t count);
    Result<void> setUniformInt(std::string name, int32_t value);

    void render(const RenderRegion& region);
    [[nodiscard]] std::vector<uint8_t> renderToBuffer(Vec2 size);
    [[nodiscard]] Result<std::vector<uint8_t>> snapshotPassRgb(std::string_view passName, uint32_t output = 0);
    [[nodiscard]] Result<std::vector<float>> snapshotPassRgba32f(std::string_view passName, uint32_t output = 0);
    [[nodiscard]] Result<std::vector<uint8_t>> snapshotStorageBuffer(std::string_view name);
    Result<void> restoreStorageBuffer(std::string_view name, const std::vector<uint8_t>& data);
    Result<void> updateTexture(std::string_view name, uint32_t width, uint32_t height,
                               const std::vector<uint32_t>& rgba);
    Result<void> reloadPassSource(std::string_view passName, std::string source);
    Result<void> setWireframePass(std::string_view passName, bool enabled);
    Result<void> overridePassRgba8(std::string_view passName, uint32_t width, uint32_t height, const std::vector<uint8_t>& rgba);
    Result<void> restorePassRgba32f(std::string_view passName, uint32_t width, uint32_t height, const std::vector<float>& rgba);
    void setProfilingEnabled(bool enabled);
    void setProfilingSyncPerPass(bool enabled);
    [[nodiscard]] const std::vector<PassTiming>& lastPassTimings() const;
    [[nodiscard]] const std::vector<PassProfileSample>& lastPassProfileSamples() const;
    [[nodiscard]] uint64_t lastFrameGpuNanoseconds() const;
    [[nodiscard]] uint64_t lastFrameGpuTimestampNanoseconds() const;
    void setFixedState(float timeSeconds, int32_t frame, float frameRate);
    void setReplayState(float timeSeconds, float timeDelta, int32_t frame, float frameRate);
    [[nodiscard]] int32_t frame() const noexcept;
    [[nodiscard]] Vec4 mouseStatus() const noexcept;

private:
    class Impl;
    Impl* mImpl;
};

SHADERTOY_NAMESPACE_END
