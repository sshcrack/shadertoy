/*
    SPDX-License-Identifier: Apache-2.0
    Copyright 2023-2026 Yingwei Zheng and contributors
*/
#pragma once

#include "shadertoy/Backend.hpp"
#include "shadertoy/Config.hpp"
#include "shadertoy/Types.hpp"

#include <chrono>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

SHADERTOY_NAMESPACE_BEGIN

class ShaderToyContext final {
    using SystemClock = std::chrono::system_clock;

public:
    ShaderToyContext();
    ShaderToyContext(const ShaderToyContext&) = delete;
    ShaderToyContext(ShaderToyContext&&) = delete;
    ShaderToyContext& operator=(const ShaderToyContext&) = delete;
    ShaderToyContext& operator=(ShaderToyContext&&) = delete;
    ~ShaderToyContext() = default;

    void tick(float frameRate = 60.0f);
    void tickFixed(float deltaSeconds, float frameRate = 60.0f);

    [[nodiscard]] bool isRunning() const noexcept {
        return mRunning;
    }
    [[nodiscard]] float getTime() const noexcept {
        return mTime;
    }
    [[nodiscard]] float getFrameRate() const noexcept {
        return mFrameRate;
    }

    void pause();
    void resume();
    void resetTime();
    void setPipeline(std::unique_ptr<Pipeline> pipeline);

    void setMouseInput(const std::optional<MouseInput>& mouse);
    void setKeyboardInput(const KeyboardInput& input);
    void setAudioInput(const AudioInput& input);
    void setUniformFloats(std::string name, const float* values, uint32_t count);
    void setUniformInt(std::string name, int32_t value);
    void setCustomUniforms(const CustomUniformMap& uniforms);
    [[nodiscard]] const CustomUniformMap& customUniforms() const noexcept {
        return mCustomUniforms;
    }

    void render(const RenderRegion& region);
    [[nodiscard]] std::vector<uint8_t> renderToBuffer(Vec2 size);
    [[nodiscard]] std::vector<uint8_t> snapshotPassRgb(std::string_view passName, uint32_t output = 0);
    [[nodiscard]] std::vector<float> snapshotPassRgba32f(std::string_view passName, uint32_t output = 0);
    [[nodiscard]] std::vector<uint8_t> snapshotStorageBuffer(std::string_view name);
    void restoreStorageBuffer(std::string_view name, const uint8_t* data, uint64_t size);
    void updateTexture(std::string_view name, uint32_t width, uint32_t height, const uint32_t* data);
    void reloadPassSource(std::string_view passName, const std::string& source);
    void setWireframePass(std::string_view passName, bool enabled);
    void overridePassRgba8(std::string_view passName, uint32_t width, uint32_t height, const uint8_t* data);
    void restorePassRgba32f(std::string_view passName, uint32_t width, uint32_t height, const float* data);
    void setProfilingEnabled(bool enabled);
    void setProfilingSyncPerPass(bool enabled);
    [[nodiscard]] const std::vector<PassTiming>& lastPassTimings() const;
    [[nodiscard]] const std::vector<PassProfileSample>& lastPassProfileSamples() const;
    [[nodiscard]] uint64_t lastFrameGpuNanoseconds() const;
    [[nodiscard]] uint64_t lastFrameGpuTimestampNanoseconds() const;
    void setFixedState(float timeSeconds, int32_t frame, float frameRate);
    void setReplayState(float timeSeconds, float timeDelta, int32_t frame, float frameRate);
    [[nodiscard]] float getTimeDelta() const noexcept {
        return mTimeDelta;
    }
    [[nodiscard]] int32_t getFrame() const noexcept {
        return mFrameCount;
    }

    [[nodiscard]] Vec4 getMouseStatus() const noexcept {
        return mMouse;
    }

    [[nodiscard]] float getTimeScale() const noexcept {
        return mTimeScale;
    }
    void setTimeScale(const float log2Scale) noexcept {
        mTimeScale = log2Scale;
    }

    [[nodiscard]] bool isValid() const noexcept {
        return static_cast<bool>(mPipeline);
    }

private:
    [[nodiscard]] ShaderToyUniform makeUniform() const;
    void updateDate();

    SystemClock::time_point mStartTime;
    SystemClock::time_point mPauseTime;
    float mTime{};
    float mTimeScale{};
    float mTimeDelta{};
    int32_t mFrameCount{};
    float mFrameRate{};
    bool mRunning{ true };
    Vec4 mMouse{ 0.0f, 0.0f, -1.0f, -1.0f };
    Vec4 mDate;
    AudioInput mAudioInput;
    KeyboardInput mKeyboardInput;
    CustomUniformMap mCustomUniforms;
    std::unique_ptr<Pipeline> mPipeline;
};

SHADERTOY_NAMESPACE_END
