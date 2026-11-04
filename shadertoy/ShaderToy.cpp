/*
    SPDX-License-Identifier: Apache-2.0
    Copyright 2023-2026 Yingwei Zheng and contributors
*/

#include "shadertoy/ShaderToy.hpp"
#include "shadertoy/Compiler.hpp"
#include "shadertoy/ShaderToyContext.hpp"
#include "shadertoy/Support.hpp"

#include <memory>
#include <utility>

SHADERTOY_NAMESPACE_BEGIN

class Runtime::Impl final {
public:
    ShaderToyContext context;
    std::optional<ShaderDocument> document;
};

Runtime::Runtime() : mImpl(new Impl) {}
Runtime::~Runtime() {
    delete mImpl;
}

Result<void> Runtime::setDocument(ShaderDocument document) {
    try {
        auto pipeline = compilePipeline(document);
        mImpl->context.setPipeline(std::move(pipeline));
        mImpl->context.setCustomUniforms(document.uniforms);
        mImpl->document.emplace(std::move(document));
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::loadSTTF(const std::string& path) {
    try {
        ShaderDocument document;
        document.load(path);
        return setDocument(std::move(document));
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::saveSTTF(const std::string& path) const {
    try {
        if(!mImpl->document)
            throw Error("No shader document is loaded");
        mImpl->document->save(path);
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::loadImageShader(std::string name, std::string source, const std::optional<uint32_t> audioChannel) {
    auto document = makeImageShader(std::move(name), std::move(source), audioChannel);
    if(!document)
        return std::unexpected(document.error());
    return setDocument(std::move(*document));
}

Result<void> Runtime::loadFromShaderToy(const std::string_view shaderUrlOrId) {
    auto document = importFromShaderToy(shaderUrlOrId);
    if(!document)
        return std::unexpected(document.error());
    return setDocument(std::move(*document));
}

Result<void> Runtime::loadFromShaderToyResponse(const std::string_view shaderId, const std::string_view responseBody) {
    auto document = importFromShaderToyResponse(shaderId, responseBody);
    if(!document)
        return std::unexpected(document.error());
    return setDocument(std::move(*document));
}

const ShaderDocument* Runtime::document() const noexcept {
    return mImpl->document ? &*mImpl->document : nullptr;
}

void Runtime::tick(const float frameRate) {
    mImpl->context.tick(frameRate);
}

void Runtime::tickFixed(const float deltaSeconds, const float frameRate) {
    mImpl->context.tickFixed(deltaSeconds, frameRate);
}

void Runtime::pause() {
    mImpl->context.pause();
}

void Runtime::resume() {
    mImpl->context.resume();
}

void Runtime::resetTime() {
    mImpl->context.resetTime();
}

bool Runtime::isRunning() const noexcept {
    return mImpl->context.isRunning();
}

bool Runtime::isValid() const noexcept {
    return mImpl->context.isValid();
}

float Runtime::time() const noexcept {
    return mImpl->context.getTime();
}

float Runtime::timeDelta() const noexcept {
    return mImpl->context.getTimeDelta();
}

float Runtime::frameRate() const noexcept {
    return mImpl->context.getFrameRate();
}

float Runtime::timeScale() const noexcept {
    return mImpl->context.getTimeScale();
}

void Runtime::setTimeScale(const float log2Scale) noexcept {
    mImpl->context.setTimeScale(log2Scale);
}

void Runtime::setMouseInput(const std::optional<MouseInput>& input) {
    mImpl->context.setMouseInput(input);
}

void Runtime::setKeyboardInput(const KeyboardInput& input) {
    mImpl->context.setKeyboardInput(input);
}

void Runtime::setAudioInput(const AudioInput& input) {
    mImpl->context.setAudioInput(input);
}

Result<void> Runtime::setUniformFloats(std::string name, const float* values, const uint32_t count) {
    try {
        mImpl->context.setUniformFloats(std::move(name), values, count);
        if(mImpl->document)
            mImpl->document->uniforms = mImpl->context.customUniforms();
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::setUniformInt(std::string name, const int32_t value) {
    try {
        mImpl->context.setUniformInt(std::move(name), value);
        if(mImpl->document)
            mImpl->document->uniforms = mImpl->context.customUniforms();
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

void Runtime::render(const RenderRegion& region) {
    mImpl->context.render(region);
}

std::vector<uint8_t> Runtime::renderToBuffer(const Vec2 size) {
    return mImpl->context.renderToBuffer(size);
}

Result<std::vector<uint8_t>> Runtime::snapshotPassRgb(const std::string_view passName, const uint32_t output) {
    try {
        return mImpl->context.snapshotPassRgb(passName, output);
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<std::vector<float>> Runtime::snapshotPassRgba32f(const std::string_view passName, const uint32_t output) {
    try {
        return mImpl->context.snapshotPassRgba32f(passName, output);
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<std::vector<uint8_t>> Runtime::snapshotStorageBuffer(const std::string_view name) {
    try {
        return mImpl->context.snapshotStorageBuffer(name);
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::restoreStorageBuffer(const std::string_view name, const std::vector<uint8_t>& data) {
    try {
        mImpl->context.restoreStorageBuffer(name, data.data(), data.size());
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::updateTexture(const std::string_view name, const uint32_t width, const uint32_t height,
                                    const std::vector<uint32_t>& rgba) {
    try {
        if(width == 0 || height == 0)
            throw Error("Texture update dimensions must be positive");
        if(rgba.size() != checkedSizeProduct({ width, height }, "Texture update"))
            throw Error("Texture update must contain width * height RGBA8 pixels");
        mImpl->context.updateTexture(name, width, height, rgba.data());
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::reloadPassSource(const std::string_view passName, std::string source) {
    try {
        mImpl->context.reloadPassSource(passName, source);
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::setWireframePass(const std::string_view passName, const bool enabled) {
    try {
        mImpl->context.setWireframePass(passName, enabled);
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::overridePassRgba8(const std::string_view passName, const uint32_t width, const uint32_t height,
                                        const std::vector<uint8_t>& rgba) {
    try {
        if(width == 0 || height == 0)
            throw Error("Pass override dimensions must be positive");
        if(rgba.size() != checkedSizeProduct({ width, height, 4U }, "Pass override"))
            throw Error("Pass override must contain width * height * 4 RGBA8 bytes");
        mImpl->context.overridePassRgba8(passName, width, height, rgba.data());
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

Result<void> Runtime::restorePassRgba32f(const std::string_view passName, const uint32_t width, const uint32_t height,
                                         const std::vector<float>& rgba) {
    try {
        if(width == 0 || height == 0)
            throw Error("Pass restore dimensions must be positive");
        if(rgba.size() != checkedSizeProduct({ width, height, 4U }, "Pass restore"))
            throw Error("Pass restore must contain width * height * 4 RGBA32F values");
        mImpl->context.restorePassRgba32f(passName, width, height, rgba.data());
        return {};
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

void Runtime::setProfilingEnabled(const bool enabled) {
    mImpl->context.setProfilingEnabled(enabled);
}

void Runtime::setProfilingSyncPerPass(const bool enabled) {
    mImpl->context.setProfilingSyncPerPass(enabled);
}

const std::vector<PassTiming>& Runtime::lastPassTimings() const {
    return mImpl->context.lastPassTimings();
}

const std::vector<PassProfileSample>& Runtime::lastPassProfileSamples() const {
    return mImpl->context.lastPassProfileSamples();
}

uint64_t Runtime::lastFrameGpuNanoseconds() const {
    return mImpl->context.lastFrameGpuNanoseconds();
}

uint64_t Runtime::lastFrameGpuTimestampNanoseconds() const {
    return mImpl->context.lastFrameGpuTimestampNanoseconds();
}

void Runtime::setFixedState(const float timeSeconds, const int32_t frameValue, const float frameRate) {
    mImpl->context.setFixedState(timeSeconds, frameValue, frameRate);
}

void Runtime::setReplayState(const float timeSeconds, const float timeDelta, const int32_t frameValue, const float frameRate) {
    mImpl->context.setReplayState(timeSeconds, timeDelta, frameValue, frameRate);
}

int32_t Runtime::frame() const noexcept {
    return mImpl->context.getFrame();
}

Vec4 Runtime::mouseStatus() const noexcept {
    return mImpl->context.getMouseStatus();
}

SHADERTOY_NAMESPACE_END
