/*
    SPDX-License-Identifier: Apache-2.0
    Copyright 2023-2026 Yingwei Zheng and contributors
*/
#pragma once

#include "shadertoy/Result.hpp"
#include "shadertoy/STTF.hpp"

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

SHADERTOY_NAMESPACE_BEGIN

enum class ProjectPassKind { Image, Buffer, Mesh, CubeMap, Compute };
inline constexpr uint32_t ProjectDepthOutput = UINT32_MAX;
enum class ProjectInputKind { Pass, Texture, CubeMap, Volume, Keyboard, Music };

struct ProjectInput final {
    uint32_t channel{};
    ProjectInputKind kind{ ProjectInputKind::Pass };
    std::string source;
    bool previousFrame{};
    Filter filter{ Filter::Linear };
    Wrap wrap{ Wrap::Repeat };
    uint32_t sourceOutput{};
};

struct ProjectPass final {
    std::string name;
    ProjectPassKind kind{ ProjectPassKind::Buffer };
    std::string source;
    std::vector<ProjectInput> inputs;
    uint32_t width{};
    uint32_t height{};
    RenderFormat renderFormat{ RenderFormat::RGBA32F };
    uint32_t iterations{ 1 };
    uint32_t localSizeX{ 8 };
    uint32_t localSizeY{ 8 };
    uint32_t localSizeZ{ 1 };
    std::vector<StorageBufferBinding> storageBuffers;
    std::vector<RenderFormat> extraRenderFormats;
    std::optional<MeshConfig> mesh;
};

struct ProjectTexture final {
    std::string name;
    uint32_t width{};
    uint32_t height{};
    std::vector<uint32_t> rgba;
};

struct ProjectCubeMap final {
    std::string name;
    uint32_t size{};
    std::vector<uint32_t> rgba;
};

struct ProjectVolume final {
    std::string name;
    uint32_t size{};
    uint32_t channels{};
    std::vector<uint8_t> data;
};

struct ProjectDescription final {
    std::string name;
    std::vector<ProjectPass> passes;
    std::vector<ProjectTexture> textures;
    std::vector<ProjectCubeMap> cubeMaps;
    std::vector<ProjectVolume> volumes;
};

/// Convert the language-neutral project model into the renderer's document graph.
///
/// This is the canonical semantic validation seam for directory projects. Frontends
/// (the Rust CLI today, the native GUI later) may parse their own storage syntax,
/// but pass/channel graph rules live here.
Result<ShaderDocument> makeProjectDocument(const ProjectDescription& project);

SHADERTOY_NAMESPACE_END
