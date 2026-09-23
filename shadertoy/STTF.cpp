/*
    SPDX-License-Identifier: Apache-2.0
    Copyright 2023-2026 Yingwei Zheng and contributors
*/

#include "shadertoy/STTF.hpp"
#include "shadertoy/Support.hpp"

#include <cstring>
#include <fstream>
#include <unordered_map>
#include <unordered_set>

#include "shadertoy/SuppressWarningPush.hpp"
#include <cpp-base64/base64.h>
#include <gsl/gsl>
#include <magic_enum/magic_enum.hpp>
#include <nlohmann/json.hpp>
#include "shadertoy/SuppressWarningPop.hpp"

SHADERTOY_NAMESPACE_BEGIN

namespace {
    template <typename Enum>
    Enum parseEnum(const nlohmann::json& value, const std::string_view field) {
        const auto text = value.get<std::string>();
        if(const auto parsed = magic_enum::enum_cast<Enum>(text))
            return *parsed;
        throw Error("Unknown " + std::string(field) + ": " + text);
    }
}  // namespace

void ShaderToyTransmissionFormat::load(const std::string& filePath) {
    std::ifstream file{ filePath };
    if(!file)
        throw Error("Cannot open STTF file: " + filePath);

    try {
        nlohmann::json json;
        file >> json;

        ShaderToyTransmissionFormat parsed;
        json.at("metadata").get_to(parsed.metadata);
        if(json.contains("uniforms")) {
            const auto& uniformsJson = json.at("uniforms");
            if(!uniformsJson.is_object())
                throw Error("STTF uniforms must be an object");
            for(auto it = uniformsJson.begin(); it != uniformsJson.end(); ++it) {
                const auto& encoded = it.value();
                CustomUniformValue uniform;
                uniform.type = parseEnum<CustomUniformType>(encoded.at("type"), "custom uniform type");
                switch(uniform.type) {
                    case CustomUniformType::Int:
                        uniform.intValue = encoded.at("value").get<int32_t>();
                        break;
                    case CustomUniformType::Float:
                        uniform.value.x = encoded.at("value").get<float>();
                        break;
                    case CustomUniformType::Vec2: {
                        const auto value = encoded.at("value").get<std::vector<float>>();
                        if(value.size() != 2)
                            throw Error("Vec2 custom uniform must contain exactly 2 values");
                        uniform.value.x = value[0];
                        uniform.value.y = value[1];
                        break;
                    }
                    case CustomUniformType::Vec3: {
                        const auto value = encoded.at("value").get<std::vector<float>>();
                        if(value.size() != 3)
                            throw Error("Vec3 custom uniform must contain exactly 3 values");
                        uniform.value.x = value[0];
                        uniform.value.y = value[1];
                        uniform.value.z = value[2];
                        break;
                    }
                    case CustomUniformType::Vec4: {
                        const auto value = encoded.at("value").get<std::vector<float>>();
                        if(value.size() != 4)
                            throw Error("Vec4 custom uniform must contain exactly 4 values");
                        uniform.value.x = value[0];
                        uniform.value.y = value[1];
                        uniform.value.z = value[2];
                        uniform.value.w = value[3];
                        break;
                    }
                }
                parsed.uniforms.insert_or_assign(it.key(), uniform);
            }
        }

        std::unordered_map<std::string, Node*> nodeMap;
        for(const auto& node : json.at("nodes")) {
            std::unique_ptr<Node> nodeValue;
            switch(parseEnum<NodeClass>(node.at("class"), "node class")) {
                case NodeClass::RenderOutput:
                    nodeValue = std::make_unique<RenderOutput>();
                    break;
                case NodeClass::GLSLShader: {
                    auto shader = std::make_unique<GLSLShader>(node.at("source").get<std::string>(),
                                                               parseEnum<NodeType>(node.at("type"), "node type"));
                    const auto hasWidth = node.contains("width");
                    const auto hasHeight = node.contains("height");
                    if(hasWidth != hasHeight)
                        throw Error("Shader fixed resolution must specify both width and height");
                    if(hasWidth) {
                        shader->fixedWidth = node.at("width").get<uint32_t>();
                        shader->fixedHeight = node.at("height").get<uint32_t>();
                        if(shader->fixedWidth == 0 || shader->fixedHeight == 0)
                            throw Error("Shader fixed resolution must be positive");
                        if(shader->nodeType != NodeType::Image && shader->nodeType != NodeType::Compute &&
                           shader->nodeType != NodeType::Mesh)
                            throw Error("Fixed resolution is only supported for 2D image/buffer, mesh, or compute shaders");
                    }
                    if(shader->nodeType == NodeType::Compute && !hasWidth)
                        throw Error("Compute shaders require a fixed resolution");
                    if(node.contains("format"))
                        shader->renderFormat = parseEnum<RenderFormat>(node.at("format"), "render format");
                    if(node.contains("iterations"))
                        shader->iterations = node.at("iterations").get<uint32_t>();
                    if(shader->iterations == 0 || shader->iterations > 4096)
                        throw Error("Shader iterations must be in the range 1..4096");
                    if(node.contains("localSize")) {
                        const auto& local = node.at("localSize");
                        if(!local.is_array() || local.size() != 3)
                            throw Error("Compute localSize must contain exactly three integers");
                        shader->localSizeX = local.at(0).get<uint32_t>();
                        shader->localSizeY = local.at(1).get<uint32_t>();
                        shader->localSizeZ = local.at(2).get<uint32_t>();
                    }
                    if(shader->nodeType == NodeType::Compute) {
                        if(shader->localSizeX == 0 || shader->localSizeY == 0 || shader->localSizeZ == 0)
                            throw Error("Compute local sizes must be positive");
                        const uint64_t invocations =
                            static_cast<uint64_t>(shader->localSizeX) * shader->localSizeY * shader->localSizeZ;
                        if(invocations > 1024)
                            throw Error("Compute local workgroup size exceeds 1024 invocations");
                        if(shader->localSizeZ != 1)
                            throw Error("Compute local workgroup Z size must be 1 for the 2D mainCompute entrypoint");
                    }
                    if(shader->nodeType == NodeType::Mesh) {
                        const auto& value = node.at("mesh");
                        MeshConfig mesh;
                        mesh.vertexSource = value.at("vertexSource").get<std::string>();
                        const auto& size = value.at("size");
                        if(!size.is_array() || size.size() != 2)
                            throw Error("Mesh size must contain exactly two integers");
                        mesh.width = size.at(0).get<uint32_t>();
                        mesh.height = size.at(1).get<uint32_t>();
                        mesh.instances = value.value("instances", 1U);
                        mesh.depth = value.value("depth", true);
                        if(value.contains("cull"))
                            mesh.cull = parseEnum<MeshCull>(value.at("cull"), "mesh cull mode");
                        if(value.contains("clear")) {
                            const auto& clear = value.at("clear");
                            if(!clear.is_array() || clear.size() != 4)
                                throw Error("Mesh clear must contain exactly four values");
                            for(uint32_t i = 0; i < 4; ++i)
                                mesh.clear[i] = clear.at(i).get<float>();
                        }
                        if(mesh.vertexSource.empty() || mesh.width < 2 || mesh.height < 2 || mesh.instances == 0)
                            throw Error("Mesh requires a vertex source, size >= 2x2, and positive instances");
                        shader->mesh = std::move(mesh);
                    } else if(node.contains("mesh")) {
                        throw Error("Mesh configuration is only valid for mesh shaders");
                    }
                    if(node.contains("storageBuffers")) {
                        std::unordered_set<uint32_t> usedStorageBindings;
                        for(const auto& storage : node.at("storageBuffers")) {
                            StorageBufferBinding binding;
                            binding.name = storage.at("name").get<std::string>();
                            binding.binding = storage.at("binding").get<uint32_t>();
                            binding.size = storage.at("size").get<uint64_t>();
                            if(binding.name.empty() || binding.size == 0)
                                throw Error("Storage buffers require a name and positive size");
                            if(!usedStorageBindings.emplace(binding.binding).second)
                                throw Error("Duplicate storage buffer binding " + std::to_string(binding.binding));
                            shader->storageBuffers.push_back(std::move(binding));
                        }
                    }
                    if(node.contains("extraRenderFormats")) {
                        for(const auto& format : node.at("extraRenderFormats"))
                            shader->extraRenderFormats.push_back(parseEnum<RenderFormat>(format, "render format"));
                        if(shader->extraRenderFormats.size() > 7)
                            throw Error("A shader may expose at most 8 color render targets");
                        if(shader->nodeType == NodeType::Mesh && shader->mesh && shader->mesh->depth &&
                           shader->extraRenderFormats.size() > 6)
                            throw Error("A depth-enabled mesh may expose at most 7 color render targets");
                    }
                    nodeValue = std::move(shader);
                    break;
                }
                case NodeClass::Texture: {
                    const auto width = node.at("width").get<uint32_t>();
                    const auto height = node.at("height").get<uint32_t>();
                    if(width == 0 || height == 0)
                        throw Error("Texture dimensions must be positive");
                    const auto decoded = base64_decode(node.at("data").get<std::string>());
                    const auto pixelCount = checkedSizeProduct({ width, height }, "Texture");
                    const auto expectedBytes = checkedSizeProduct({ pixelCount, sizeof(uint32_t) }, "Texture payload");
                    if(decoded.size() != expectedBytes)
                        throw Error("Texture payload has an invalid size");
                    std::vector<uint32_t> pixels(pixelCount);
                    std::memcpy(pixels.data(), decoded.data(), expectedBytes);
                    nodeValue = std::make_unique<Texture>(width, height, std::move(pixels));
                    break;
                }
                case NodeClass::CubeMap: {
                    const auto size = node.at("size").get<uint32_t>();
                    if(size == 0)
                        throw Error("Cubemap size must be positive");
                    const auto decoded = base64_decode(node.at("data").get<std::string>());
                    const auto pixelCount = checkedSizeProduct({ size, size, 6U }, "Cubemap");
                    const auto expectedBytes = checkedSizeProduct({ pixelCount, sizeof(uint32_t) }, "Cubemap payload");
                    if(decoded.size() != expectedBytes)
                        throw Error("Cubemap payload has an invalid size");
                    std::vector<uint32_t> pixels(pixelCount);
                    std::memcpy(pixels.data(), decoded.data(), expectedBytes);
                    nodeValue = std::make_unique<CubeMap>(size, std::move(pixels));
                    break;
                }
                case NodeClass::Volume: {
                    const auto size = node.at("size").get<uint32_t>();
                    const auto channels = node.at("channels").get<uint32_t>();
                    if(size == 0)
                        throw Error("Volume size must be positive");
                    if(channels != 1 && channels != 4)
                        throw Error("Volume channel count must be 1 or 4");
                    const auto decoded = base64_decode(node.at("data").get<std::string>());
                    const auto byteCount = checkedSizeProduct({ size, size, size, channels }, "Volume");
                    if(decoded.size() != byteCount)
                        throw Error("Volume payload has an invalid size");
                    nodeValue = std::make_unique<Volume>(
                        size, channels,
                        std::vector<uint8_t>{ reinterpret_cast<const uint8_t*>(decoded.data()),
                                              reinterpret_cast<const uint8_t*>(decoded.data()) + byteCount });
                    break;
                }
                case NodeClass::LastFrame:
                    nodeValue = std::make_unique<LastFrame>(node.at("ref").get<std::string>(),
                                                            parseEnum<NodeType>(node.at("type"), "node type"),
                                                            node.value("refOutput", 0U));
                    break;
                case NodeClass::Keyboard:
                    nodeValue = std::make_unique<Keyboard>();
                    break;
                case NodeClass::Music:
                    nodeValue = std::make_unique<Music>();
                    break;
                case NodeClass::SoundOutput:
                case NodeClass::Unknown:
                    throw Error("Unsupported node class in STTF file");
            }

            nodeValue->name = node.at("name").get<std::string>();
            if(!nodeMap.emplace(nodeValue->name, nodeValue.get()).second)
                throw Error("Duplicate node name in STTF file: " + nodeValue->name);
            parsed.nodes.push_back(std::move(nodeValue));
        }

        for(auto& node : parsed.nodes) {
            if(node->getNodeClass() != NodeClass::LastFrame)
                continue;
            auto& lastFrame = dynamic_cast<LastFrame&>(*node);
            const auto referenced = nodeMap.find(lastFrame.refNodeName);
            if(referenced == nodeMap.end())
                throw Error("Unknown LastFrame reference: " + lastFrame.refNodeName);
            lastFrame.refNode = referenced->second;
        }

        for(const auto& link : json.at("links")) {
            const auto startName = link.at("start").get<std::string>();
            const auto endName = link.at("end").get<std::string>();
            const auto start = nodeMap.find(startName);
            const auto end = nodeMap.find(endName);
            if(start == nodeMap.end() || end == nodeMap.end())
                throw Error("Link references an unknown node");
            parsed.links.push_back(Link{
                start->second,
                end->second,
                parseEnum<Filter>(link.at("filter"), "filter"),
                parseEnum<Wrap>(link.at("wrapMode"), "wrap mode"),
                link.at("slot").get<uint32_t>(),
                link.value("sourceOutput", 0U),
            });
        }

        *this = std::move(parsed);
    } catch(const Error&) {
        throw;
    } catch(const std::exception& ex) {
        throw Error("Failed to parse STTF file '" + filePath + "': " + ex.what());
    }
}

void ShaderToyTransmissionFormat::save(const std::string& filePath) const {
    std::ofstream file{ filePath };
    if(!file)
        throw Error("Cannot open STTF file for writing: " + filePath);

    try {
        nlohmann::json json;
        nlohmann::to_json(json["metadata"], metadata);
        if(!uniforms.empty()) {
            auto jsonUniforms = nlohmann::json::object();
            for(const auto& [name, uniform] : uniforms) {
                nlohmann::json encoded;
                encoded["type"] = magic_enum::enum_name(uniform.type);
                switch(uniform.type) {
                    case CustomUniformType::Int:
                        encoded["value"] = uniform.intValue;
                        break;
                    case CustomUniformType::Float:
                        encoded["value"] = uniform.value.x;
                        break;
                    case CustomUniformType::Vec2:
                        encoded["value"] = { uniform.value.x, uniform.value.y };
                        break;
                    case CustomUniformType::Vec3:
                        encoded["value"] = { uniform.value.x, uniform.value.y, uniform.value.z };
                        break;
                    case CustomUniformType::Vec4:
                        encoded["value"] = { uniform.value.x, uniform.value.y, uniform.value.z, uniform.value.w };
                        break;
                }
                jsonUniforms.emplace(name, std::move(encoded));
            }
            json["uniforms"] = std::move(jsonUniforms);
        }
        auto& jsonNodes = json["nodes"];

        for(const auto& node : nodes) {
            nlohmann::json jsonNode;
            jsonNode["class"] = magic_enum::enum_name(node->getNodeClass());
            jsonNode["name"] = node->name;

            switch(node->getNodeClass()) {
                case NodeClass::RenderOutput:
                case NodeClass::Keyboard:
                case NodeClass::Music:
                case NodeClass::SoundOutput:
                    break;
                case NodeClass::GLSLShader: {
                    const auto& shader = dynamic_cast<const GLSLShader&>(*node);
                    jsonNode["source"] = shader.source;
                    jsonNode["type"] = magic_enum::enum_name(shader.nodeType);
                    if(shader.fixedWidth != 0 || shader.fixedHeight != 0) {
                        if(shader.fixedWidth == 0 || shader.fixedHeight == 0)
                            throw Error("Cannot save shader with partial fixed resolution");
                        jsonNode["width"] = shader.fixedWidth;
                        jsonNode["height"] = shader.fixedHeight;
                    }
                    if(shader.renderFormat != RenderFormat::RGBA32F)
                        jsonNode["format"] = magic_enum::enum_name(shader.renderFormat);
                    if(shader.iterations != 1)
                        jsonNode["iterations"] = shader.iterations;
                    if(shader.nodeType == NodeType::Compute)
                        jsonNode["localSize"] = { shader.localSizeX, shader.localSizeY, shader.localSizeZ };
                    if(shader.nodeType == NodeType::Mesh) {
                        if(!shader.mesh)
                            throw Error("Cannot save mesh shader without mesh configuration");
                        const auto& mesh = *shader.mesh;
                        jsonNode["mesh"] = {
                            { "vertexSource", mesh.vertexSource },
                            { "size", { mesh.width, mesh.height } },
                            { "instances", mesh.instances },
                            { "depth", mesh.depth },
                            { "cull", magic_enum::enum_name(mesh.cull) },
                            { "clear", { mesh.clear[0], mesh.clear[1], mesh.clear[2], mesh.clear[3] } },
                        };
                    }
                    if(!shader.storageBuffers.empty()) {
                        auto& buffers = jsonNode["storageBuffers"];
                        for(const auto& storage : shader.storageBuffers) {
                            buffers.push_back({
                                { "name", storage.name },
                                { "binding", storage.binding },
                                { "size", storage.size },
                            });
                        }
                    }
                    if(!shader.extraRenderFormats.empty()) {
                        auto& formats = jsonNode["extraRenderFormats"];
                        for(const auto format : shader.extraRenderFormats)
                            formats.push_back(magic_enum::enum_name(format));
                    }
                    break;
                }
                case NodeClass::Texture: {
                    const auto& texture = dynamic_cast<const Texture&>(*node);
                    const auto bytes = gsl::as_bytes(gsl::span<const uint32_t>{ texture.pixel.data(), texture.pixel.size() });
                    jsonNode["data"] = base64_encode(reinterpret_cast<const uint8_t*>(bytes.data()), bytes.size());
                    jsonNode["width"] = texture.width;
                    jsonNode["height"] = texture.height;
                    break;
                }
                case NodeClass::CubeMap: {
                    const auto& texture = dynamic_cast<const CubeMap&>(*node);
                    const auto bytes = gsl::as_bytes(gsl::span<const uint32_t>{ texture.pixel.data(), texture.pixel.size() });
                    jsonNode["data"] = base64_encode(reinterpret_cast<const uint8_t*>(bytes.data()), bytes.size());
                    jsonNode["size"] = texture.size;
                    break;
                }
                case NodeClass::Volume: {
                    const auto& volume = dynamic_cast<const Volume&>(*node);
                    const auto bytes = gsl::as_bytes(gsl::span<const uint8_t>{ volume.pixel.data(), volume.pixel.size() });
                    jsonNode["data"] = base64_encode(reinterpret_cast<const uint8_t*>(bytes.data()), bytes.size());
                    jsonNode["size"] = volume.size;
                    jsonNode["channels"] = volume.channels;
                    break;
                }
                case NodeClass::LastFrame: {
                    const auto& lastFrame = dynamic_cast<const LastFrame&>(*node);
                    jsonNode["ref"] = lastFrame.refNodeName;
                    jsonNode["type"] = magic_enum::enum_name(lastFrame.nodeType);
                    if(lastFrame.refOutput != 0)
                        jsonNode["refOutput"] = lastFrame.refOutput;
                    break;
                }
                case NodeClass::Unknown:
                    throw Error("Cannot save an unknown node class");
            }
            jsonNodes.push_back(std::move(jsonNode));
        }

        auto& jsonLinks = json["links"];
        for(const auto& [start, end, filter, wrapMode, slot, sourceOutput] : links) {
            if(!start || !end)
                throw Error("Cannot save a link with a null endpoint");
            nlohmann::json jsonLink;
            jsonLink["start"] = start->name;
            jsonLink["end"] = end->name;
            jsonLink["filter"] = magic_enum::enum_name(filter);
            jsonLink["wrapMode"] = magic_enum::enum_name(wrapMode);
            jsonLink["slot"] = slot;
            if(sourceOutput != 0)
                jsonLink["sourceOutput"] = sourceOutput;
            jsonLinks.push_back(std::move(jsonLink));
        }

        file << json;
    } catch(const Error&) {
        throw;
    } catch(const std::exception& ex) {
        throw Error("Failed to write STTF file '" + filePath + "': " + ex.what());
    }
}

SHADERTOY_NAMESPACE_END
