/*
    SPDX-License-Identifier: Apache-2.0
    Copyright 2023-2026 Yingwei Zheng and contributors
*/
#include "shadertoy/Project.hpp"

#include "shadertoy/Error.hpp"
#include "shadertoy/Support.hpp"

#include <cstdint>
#include <limits>
#include <memory>
#include <unordered_map>
#include <unordered_set>

SHADERTOY_NAMESPACE_BEGIN

namespace {
    NodeType passNodeType(const ProjectPassKind kind) {
        switch(kind) {
            case ProjectPassKind::Image:
            case ProjectPassKind::Buffer:
                return NodeType::Image;
            case ProjectPassKind::Mesh:
                return NodeType::Mesh;
            case ProjectPassKind::CubeMap:
                return NodeType::CubeMap;
            case ProjectPassKind::Compute:
                return NodeType::Compute;
        }
        throw Error("Unknown project pass kind");
    }
}  // namespace

Result<ShaderDocument> makeProjectDocument(const ProjectDescription& project) {
    try {
        if(project.name.empty())
            throw Error("Project name must not be empty");
        if(project.passes.empty())
            throw Error("Project must contain at least one shader pass");

        ShaderDocument document;
        document.metadata.emplace("Name", project.name);

        std::unordered_map<std::string, Node*> sources;
        std::unordered_map<std::string, GLSLShader*> passes;
        std::unordered_set<std::string> names;
        std::unordered_map<std::string, uint64_t> storageBufferSizes;
        GLSLShader* finalImage = nullptr;

        for(const auto& texture : project.textures) {
            if(texture.name.empty())
                throw Error("Texture name must not be empty");
            if(!names.emplace(texture.name).second)
                throw Error("Duplicate project source name: " + texture.name);
            if(texture.width == 0 || texture.height == 0)
                throw Error("Texture dimensions must be positive: " + texture.name);
            if(texture.rgba.size() != checkedSizeProduct({ texture.width, texture.height }, "Texture"))
                throw Error("Texture pixel payload has the wrong size: " + texture.name);

            auto node = std::make_unique<Texture>(texture.width, texture.height, texture.rgba);
            node->name = texture.name;
            sources.emplace(texture.name, node.get());
            document.nodes.emplace_back(std::move(node));
        }

        for(const auto& cubeMap : project.cubeMaps) {
            if(cubeMap.name.empty())
                throw Error("Cubemap name must not be empty");
            if(!names.emplace(cubeMap.name).second)
                throw Error("Duplicate project source name: " + cubeMap.name);
            if(cubeMap.size == 0)
                throw Error("Cubemap size must be positive: " + cubeMap.name);
            if(cubeMap.rgba.size() != checkedSizeProduct({ cubeMap.size, cubeMap.size, 6U }, "Cubemap"))
                throw Error("Cubemap pixel payload has the wrong size: " + cubeMap.name);

            auto node = std::make_unique<CubeMap>(cubeMap.size, cubeMap.rgba);
            node->name = cubeMap.name;
            sources.emplace(cubeMap.name, node.get());
            document.nodes.emplace_back(std::move(node));
        }

        for(const auto& volume : project.volumes) {
            if(volume.name.empty())
                throw Error("Volume name must not be empty");
            if(!names.emplace(volume.name).second)
                throw Error("Duplicate project source name: " + volume.name);
            if(volume.size == 0)
                throw Error("Volume size must be positive: " + volume.name);
            if(volume.channels != 1 && volume.channels != 4)
                throw Error("Volume channels must be 1 or 4: " + volume.name);
            if(volume.data.size() != checkedSizeProduct({ volume.size, volume.size, volume.size, volume.channels }, "Volume"))
                throw Error("Volume pixel payload has the wrong size: " + volume.name);

            auto node = std::make_unique<Volume>(volume.size, volume.channels, volume.data);
            node->name = volume.name;
            sources.emplace(volume.name, node.get());
            document.nodes.emplace_back(std::move(node));
        }

        for(const auto& pass : project.passes) {
            if(pass.name.empty())
                throw Error("Pass name must not be empty");
            if(!names.emplace(pass.name).second)
                throw Error("Duplicate project source name: " + pass.name);
            if(pass.source.empty())
                throw Error("Pass source must not be empty: " + pass.name);
            if((pass.width == 0) != (pass.height == 0))
                throw Error("Pass fixed resolution must specify both width and height: " + pass.name);
            if((pass.width != 0 || pass.height != 0) &&
               pass.kind != ProjectPassKind::Buffer && pass.kind != ProjectPassKind::Compute &&
               pass.kind != ProjectPassKind::Mesh)
                throw Error("Fixed pass resolution is only supported for buffer/compute/mesh passes: " + pass.name);
            if(pass.kind == ProjectPassKind::Compute && (pass.width == 0 || pass.height == 0))
                throw Error("Compute passes require an explicit width and height: " + pass.name);
            if(pass.width > static_cast<uint32_t>(std::numeric_limits<int32_t>::max()) ||
               pass.height > static_cast<uint32_t>(std::numeric_limits<int32_t>::max()))
                throw Error("Pass fixed resolution exceeds the OpenGL dimension range: " + pass.name);
            if(pass.kind != ProjectPassKind::Buffer && pass.kind != ProjectPassKind::Compute &&
               pass.kind != ProjectPassKind::Mesh && pass.renderFormat != RenderFormat::RGBA32F)
                throw Error("Explicit render formats are only supported for buffer/compute/mesh passes: " + pass.name);
            if(!pass.extraRenderFormats.empty() && pass.kind != ProjectPassKind::Buffer &&
               pass.kind != ProjectPassKind::Compute && pass.kind != ProjectPassKind::Mesh)
                throw Error("Multiple render targets are only supported for buffer/compute/mesh passes: " + pass.name);
            if(pass.kind == ProjectPassKind::Mesh) {
                if(!pass.mesh)
                    throw Error("Mesh pass is missing mesh configuration: " + pass.name);
                const auto& mesh = *pass.mesh;
                if(mesh.vertexSource.empty())
                    throw Error("Mesh pass vertex source must not be empty: " + pass.name);
                if(mesh.width < 2 || mesh.height < 2 || mesh.instances == 0)
                    throw Error("Mesh grid must be at least 2x2 with a positive instance count: " + pass.name);
            } else if(pass.mesh) {
                throw Error("Mesh configuration is only valid for mesh passes: " + pass.name);
            }
            if(pass.extraRenderFormats.size() > 7)
                throw Error("A pass may expose at most 8 color render targets: " + pass.name);
            if(pass.kind == ProjectPassKind::Mesh && pass.mesh && pass.mesh->depth &&
               pass.extraRenderFormats.size() > 6)
                throw Error("A depth-enabled mesh may expose at most 7 color render targets: " + pass.name);
            if(pass.iterations == 0 || pass.iterations > 4096)
                throw Error("Pass iterations must be in the range 1..4096: " + pass.name);
            if(pass.kind != ProjectPassKind::Compute && pass.iterations != 1)
                throw Error("Pass iterations are only supported for compute passes: " + pass.name);
            if(pass.kind != ProjectPassKind::Compute &&
               (pass.localSizeX != 8 || pass.localSizeY != 8 || pass.localSizeZ != 1))
                throw Error("Compute local size is only supported for compute passes: " + pass.name);
            if(pass.kind == ProjectPassKind::Compute) {
                if(pass.localSizeX == 0 || pass.localSizeY == 0 || pass.localSizeZ == 0)
                    throw Error("Compute local sizes must be positive: " + pass.name);
                const uint64_t invocations = static_cast<uint64_t>(pass.localSizeX) * pass.localSizeY * pass.localSizeZ;
                if(invocations > 1024)
                    throw Error("Compute local workgroup size exceeds 1024 invocations: " + pass.name);
                if(pass.localSizeZ != 1)
                    throw Error("Compute local workgroup Z size must be 1 for the 2D mainCompute entrypoint: " + pass.name);
            }
            std::unordered_set<uint32_t> usedStorageBindings;
            for(const auto& storage : pass.storageBuffers) {
                if(storage.name.empty())
                    throw Error("Storage buffer name must not be empty in pass " + pass.name);
                if(storage.size == 0)
                    throw Error("Storage buffer size must be positive: " + storage.name);
                if(!usedStorageBindings.emplace(storage.binding).second)
                    throw Error("Duplicate storage buffer binding " + std::to_string(storage.binding) + " in pass " + pass.name);
                const auto [it, inserted] = storageBufferSizes.emplace(storage.name, storage.size);
                if(!inserted && it->second != storage.size)
                    throw Error("Storage buffer '" + storage.name + "' uses inconsistent sizes across passes");
            }

            auto node = std::make_unique<GLSLShader>(pass.source, passNodeType(pass.kind));
            node->name = pass.name;
            node->fixedWidth = pass.width;
            node->fixedHeight = pass.height;
            node->renderFormat = pass.renderFormat;
            node->iterations = pass.iterations;
            node->localSizeX = pass.localSizeX;
            node->localSizeY = pass.localSizeY;
            node->localSizeZ = pass.localSizeZ;
            node->storageBuffers = pass.storageBuffers;
            node->extraRenderFormats = pass.extraRenderFormats;
            node->mesh = pass.mesh;
            auto* shader = node.get();
            passes.emplace(pass.name, shader);
            sources.emplace(pass.name, shader);
            if(pass.kind == ProjectPassKind::Image) {
                if(finalImage)
                    throw Error("Project must contain exactly one image pass");
                finalImage = shader;
            }
            document.nodes.emplace_back(std::move(node));
        }

        if(!finalImage)
            throw Error("Project must contain exactly one image pass");

        Keyboard* keyboard = nullptr;
        Music* music = nullptr;
        uint32_t feedbackIndex = 0;

        for(const auto& pass : project.passes) {
            auto* consumer = passes.at(pass.name);
            std::unordered_set<uint32_t> usedChannels;
            for(const auto& input : pass.inputs) {
                if(input.channel >= MaxInputChannels)
                    throw Error("Channel index must be between 0 and 15 in pass " + pass.name);
                if(!usedChannels.emplace(input.channel).second)
                    throw Error("Duplicate channel " + std::to_string(input.channel) + " in pass " + pass.name);

                Node* producer = nullptr;
                switch(input.kind) {
                    case ProjectInputKind::Pass:
                    case ProjectInputKind::Texture:
                    case ProjectInputKind::CubeMap:
                    case ProjectInputKind::Volume: {
                        const auto found = sources.find(input.source);
                        if(found == sources.end())
                            throw Error("Unknown input source '" + input.source + "' in pass " + pass.name);
                        producer = found->second;
                        if(input.kind == ProjectInputKind::Pass && producer->getNodeClass() != NodeClass::GLSLShader)
                            throw Error("Input source is not a shader pass: " + input.source);
                        if(input.kind == ProjectInputKind::Pass) {
                            const auto* sourcePass = dynamic_cast<const GLSLShader*>(producer);
                            if(!sourcePass)
                                throw Error("Input source is not a shader pass: " + input.source);
                            if(input.sourceOutput == ProjectDepthOutput) {
                                if(sourcePass->nodeType != NodeType::Mesh || !sourcePass->mesh || !sourcePass->mesh->depth)
                                    throw Error("Input source has no readable depth output: " + input.source);
                            } else if(input.sourceOutput > sourcePass->extraRenderFormats.size()) {
                                throw Error("Input source output index is out of range: " + input.source);
                            }
                        } else if(input.sourceOutput != 0) {
                            throw Error("Only shader pass inputs can select a nonzero output index");
                        }
                        if(input.kind == ProjectInputKind::Texture && producer->getNodeClass() != NodeClass::Texture)
                            throw Error("Input source is not a texture: " + input.source);
                        if(input.kind == ProjectInputKind::CubeMap && producer->getNodeClass() != NodeClass::CubeMap)
                            throw Error("Input source is not a cubemap: " + input.source);
                        if(input.kind == ProjectInputKind::Volume && producer->getNodeClass() != NodeClass::Volume)
                            throw Error("Input source is not a volume: " + input.source);
                        break;
                    }
                    case ProjectInputKind::Keyboard:
                        if(input.sourceOutput != 0)
                            throw Error("Keyboard inputs cannot select a render output");
                        if(input.previousFrame)
                            throw Error("Keyboard inputs cannot use previous-frame semantics");
                        if(!keyboard) {
                            auto node = std::make_unique<Keyboard>();
                            node->name = "keyboard";
                            keyboard = node.get();
                            document.nodes.emplace_back(std::move(node));
                        }
                        producer = keyboard;
                        break;
                    case ProjectInputKind::Music:
                        if(input.sourceOutput != 0)
                            throw Error("Music inputs cannot select a render output");
                        if(input.previousFrame)
                            throw Error("Music inputs cannot use previous-frame semantics");
                        if(!music) {
                            auto node = std::make_unique<Music>();
                            node->name = "music";
                            music = node.get();
                            document.nodes.emplace_back(std::move(node));
                        }
                        producer = music;
                        break;
                }

                if(input.previousFrame && input.sourceOutput == ProjectDepthOutput)
                    throw Error("Previous-frame mesh depth inputs are not supported");
                if(input.previousFrame) {
                    if(input.kind != ProjectInputKind::Pass)
                        throw Error("Only shader pass inputs can use previous-frame semantics");
                    if(input.sourceOutput != 0)
                        throw Error("Previous-frame feedback is only supported for render output 0");
                    auto* sourcePass = dynamic_cast<GLSLShader*>(producer);
                    if(!sourcePass)
                        throw Error("Previous-frame source is not a shader pass: " + input.source);
                    if(sourcePass == finalImage)
                        throw Error("The final image pass cannot be used as a previous-frame source");

                    auto last = std::make_unique<LastFrame>(sourcePass->name, sourcePass->nodeType, input.sourceOutput);
                    last->name = "__feedback_" + std::to_string(feedbackIndex++);
                    last->refNode = sourcePass;
                    producer = last.get();
                    document.nodes.emplace_back(std::move(last));
                }

                document.links.push_back(Link{ producer, consumer, input.filter, input.wrap, input.channel,
                                               input.previousFrame ? 0U : input.sourceOutput });
            }
        }

        auto output = std::make_unique<RenderOutput>();
        output->name = "Output";
        auto* outputPtr = output.get();
        document.nodes.emplace_back(std::move(output));
        document.links.push_back(Link{ finalImage, outputPtr, Filter::Linear, Wrap::Clamp, 0 });

        return document;
    } catch(const Error& error) {
        return std::unexpected(error);
    } catch(const std::exception& error) {
        return std::unexpected(Error(error.what()));
    }
}

SHADERTOY_NAMESPACE_END
