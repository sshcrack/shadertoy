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

#pragma once

#include "shadertoy/Config.hpp"
#include "shadertoy/Types.hpp"
#include <array>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

SHADERTOY_NAMESPACE_BEGIN

enum class NodeClass { RenderOutput, SoundOutput, GLSLShader, Texture, CubeMap, LastFrame, Keyboard, Music, Volume, Unknown };
enum class NodeType { Image, Mesh, CubeMap, Volume, Sound, Compute };
enum class RenderFormat { R32F, RG32F, RGBA16F, RGBA32F };
enum class CustomUniformType { Float, Int, Vec2, Vec3, Vec4 };
enum class MeshCull { None, Back, Front };

struct MeshConfig final {
    std::string vertexSource;
    uint32_t width{};
    uint32_t height{};
    uint32_t instances{ 1 };
    bool depth{ true };
    MeshCull cull{ MeshCull::None };
    std::array<float, 4> clear{};
};

struct CustomUniformValue final {
    CustomUniformType type{ CustomUniformType::Float };
    Vec4 value;
    int32_t intValue{};
};

using CustomUniformMap = std::unordered_map<std::string, CustomUniformValue>;

struct StorageBufferBinding final {
    std::string name;
    uint32_t binding{};
    uint64_t size{};
};
enum class Filter { Mipmap, Linear, Nearest };
enum class Wrap { Clamp, Repeat };

struct Node {
    std::string name;

    Node() = default;
    Node(const Node&) = delete;
    Node(Node&&) = delete;
    Node& operator=(const Node&) = delete;
    Node& operator=(Node&&) = delete;
    virtual ~Node() = default;
    [[nodiscard]] virtual NodeClass getNodeClass() const noexcept = 0;
    [[nodiscard]] virtual NodeType getNodeType() const noexcept = 0;
};

struct RenderOutput final : Node {
    [[nodiscard]] NodeClass getNodeClass() const noexcept override {
        return NodeClass::RenderOutput;
    }
    [[nodiscard]] NodeType getNodeType() const noexcept override {
        return NodeType::Image;
    }
};

/*
class SoundOutput final : public Node {
public:
    [[nodiscard]] NodeClass getNodeClass() const noexcept override {
        return NodeClass::SoundOutput;
    }
    [[nodiscard]] NodeType getNodeType() const noexcept override {
        return NodeType::Sound;
    }
};
*/

struct GLSLShader final : Node {
    std::string source;
    NodeType nodeType;
    uint32_t fixedWidth{};
    uint32_t fixedHeight{};
    RenderFormat renderFormat{ RenderFormat::RGBA32F };
    uint32_t iterations{ 1 };
    uint32_t localSizeX{ 8 };
    uint32_t localSizeY{ 8 };
    uint32_t localSizeZ{ 1 };
    std::vector<StorageBufferBinding> storageBuffers;
    std::vector<RenderFormat> extraRenderFormats;
    std::optional<MeshConfig> mesh;

    GLSLShader(std::string src, const NodeType type) : source{ std::move(src) }, nodeType{ type } {}
    [[nodiscard]] NodeClass getNodeClass() const noexcept override {
        return NodeClass::GLSLShader;
    }
    [[nodiscard]] NodeType getNodeType() const noexcept override {
        return nodeType;
    }
};

struct LastFrame final : Node {
    std::string refNodeName;
    Node* refNode = nullptr;
    NodeType nodeType;
    uint32_t refOutput{};

    LastFrame(std::string refNodeNameVal, const NodeType nodeTypeVal, const uint32_t refOutputVal = 0)
        : refNodeName{ std::move(refNodeNameVal) }, nodeType{ nodeTypeVal }, refOutput{ refOutputVal } {}
    [[nodiscard]] NodeClass getNodeClass() const noexcept override {
        return NodeClass::LastFrame;
    }
    [[nodiscard]] NodeType getNodeType() const noexcept override {
        return nodeType;
    }
};

struct Texture final : Node {
    uint32_t width;
    uint32_t height;
    std::vector<uint32_t> pixel;  // R8G8B8A8

    Texture(const uint32_t w, const uint32_t h, std::vector<uint32_t> data) : width{ w }, height{ h }, pixel{ std::move(data) } {}
    [[nodiscard]] NodeClass getNodeClass() const noexcept override {
        return NodeClass::Texture;
    }
    [[nodiscard]] NodeType getNodeType() const noexcept override {
        return NodeType::Image;
    }
};

struct CubeMap final : Node {
    uint32_t size;
    std::vector<uint32_t> pixel;  // R8G8B8A8 * 6

    CubeMap(const uint32_t x, std::vector<uint32_t> data) : size{ x }, pixel{ std::move(data) } {}
    [[nodiscard]] NodeClass getNodeClass() const noexcept override {
        return NodeClass::CubeMap;
    }
    [[nodiscard]] NodeType getNodeType() const noexcept override {
        return NodeType::CubeMap;
    }
};

struct Volume final : Node {
    uint32_t size;
    uint32_t channels;
    std::vector<uint8_t> pixel;  // R8 or R8G8B8A8 voxels

    Volume(uint32_t x, uint32_t channels, std::vector<uint8_t> data)
        : size{ x }, channels{ channels }, pixel{ std::move(data) } {}
    [[nodiscard]] NodeClass getNodeClass() const noexcept override {
        return NodeClass::Volume;
    }
    [[nodiscard]] NodeType getNodeType() const noexcept override {
        return NodeType::Volume;
    }
};

struct Keyboard final : Node {
    [[nodiscard]] NodeClass getNodeClass() const noexcept override {
        return NodeClass::Keyboard;
    }
    [[nodiscard]] NodeType getNodeType() const noexcept override {
        return NodeType::Image;
    }
};

struct Music final : Node {
    [[nodiscard]] NodeClass getNodeClass() const noexcept override {
        return NodeClass::Music;
    }
    [[nodiscard]] NodeType getNodeType() const noexcept override {
        return NodeType::Image;
    }
};

struct Link final {
    Node* start;
    Node* end;
    Filter filter;
    Wrap wrapMode;
    uint32_t slot;
    uint32_t sourceOutput{};
};

struct ShaderToyTransmissionFormat final {
    using Metadata = std::unordered_map<std::string, std::string>;

    ShaderToyTransmissionFormat() = default;
    ShaderToyTransmissionFormat(const ShaderToyTransmissionFormat&) = delete;
    ShaderToyTransmissionFormat& operator=(const ShaderToyTransmissionFormat&) = delete;
    ShaderToyTransmissionFormat(ShaderToyTransmissionFormat&&) noexcept = default;
    ShaderToyTransmissionFormat& operator=(ShaderToyTransmissionFormat&&) noexcept = default;

    Metadata metadata;
    CustomUniformMap uniforms;
    std::vector<std::unique_ptr<Node>> nodes;
    std::vector<Link> links;

    void load(const std::string& filePath);
    void save(const std::string& filePath) const;
};

using ShaderDocument = ShaderToyTransmissionFormat;

SHADERTOY_NAMESPACE_END
