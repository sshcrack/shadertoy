/*
    SPDX-License-Identifier: Apache-2.0
    Copyright 2023-2026 Yingwei Zheng and contributors
*/

#include "shadertoy/Compiler.hpp"
#include "shadertoy/Support.hpp"

#include <algorithm>
#include <cassert>
#include <functional>
#include <limits>
#include <optional>
#include <unordered_map>
#include <unordered_set>
#include <vector>

SHADERTOY_NAMESPACE_BEGIN

std::unique_ptr<Pipeline> compilePipeline(const ShaderDocument& document) {
    using Inputs = std::vector<const Link*>;
    std::unordered_map<const Node*, Inputs> inputs;
    std::unordered_set<const Node*> allNodes;
    for(const auto& node : document.nodes)
        allNodes.insert(node.get());

    const Node* sinkNode = nullptr;
    const Node* directRenderNode = nullptr;
    std::size_t outputLinks = 0;
    std::unordered_map<const Node*, std::unordered_set<uint32_t>> shaderInputSlots;

    for(const auto& link : document.links) {
        if(!link.start || !link.end || !allNodes.contains(link.start) || !allNodes.contains(link.end))
            throw Error("Pipeline contains a link with an invalid endpoint");
        if(link.end->getNodeClass() == NodeClass::GLSLShader) {
            if(link.slot >= MaxInputChannels)
                throw Error("Shader input channel index must be between 0 and 15");
            if(!shaderInputSlots[link.end].insert(link.slot).second)
                throw Error("Shader input channel is assigned more than once");
        }
        inputs[link.end].push_back(&link);
        if(link.end->getNodeClass() == NodeClass::RenderOutput) {
            ++outputLinks;
            sinkNode = link.end;
            directRenderNode = link.start;
        }
    }

    if(outputLinks != 1 || !sinkNode || !directRenderNode || directRenderNode->getNodeClass() != NodeClass::GLSLShader) {
        throw Error("Exactly one shader must be connected to the final render output");
    }

    enum class VisitState : uint8_t { Visiting, Done };
    std::unordered_map<const Node*, VisitState> state;
    std::vector<const Node*> order;
    std::unordered_set<const Node*> requiredDoubleBuffer;
    std::vector<const LastFrame*> reachableLastFrames;

    std::function<void(const Node*)> visit = [&](const Node* node) {
        if(const auto found = state.find(node); found != state.end()) {
            if(found->second == VisitState::Visiting)
                throw Error("Loop detected in current-frame shader dependencies");
            return;
        }

        state.emplace(node, VisitState::Visiting);
        if(const auto found = inputs.find(node); found != inputs.end()) {
            for(const auto* link : found->second)
                visit(link->start);
        }

        if(node->getNodeClass() == NodeClass::LastFrame) {
            const auto& lastFrame = dynamic_cast<const LastFrame&>(*node);
            if(!lastFrame.refNode || !allNodes.contains(lastFrame.refNode))
                throw Error("LastFrame references an invalid node");
            if(lastFrame.refNode == directRenderNode)
                throw Error("The final image pass cannot be used as a LastFrame source");
            requiredDoubleBuffer.insert(lastFrame.refNode);
            reachableLastFrames.push_back(&lastFrame);
        }

        state[node] = VisitState::Done;
        order.push_back(node);
    };

    visit(sinkNode);

    // A LastFrame edge is deliberately not a current-frame dependency. Its
    // referenced pass still needs to execute, however, even if it is otherwise
    // only reachable through the feedback edge.
    for(std::size_t i = 0; i < reachableLastFrames.size(); ++i)
        visit(reachableLastFrames[i]->refNode);

    auto pipeline = createPipeline();
    if(!pipeline)
        throw Error("Failed to create the OpenGL pipeline");

    std::unordered_map<const Node*, std::vector<DoubleBufferedTex>> textureMap;
    std::unordered_map<const Node*, Vec2> textureSizeMap;
    std::unordered_map<const Node*, std::vector<DoubleBufferedFB>> frameBufferMap;
    std::unordered_map<std::string, std::pair<uint64_t, BufferId>> storageBufferMap;

    // Allocate render targets first so LastFrame nodes can refer to a pass that
    // appears later in execution order.
    for(const auto* node : order) {
        if(node->getNodeClass() != NodeClass::GLSLShader)
            continue;

        const auto& shader = dynamic_cast<const GLSLShader&>(*node);
        if(shader.extraRenderFormats.size() > 7)
            throw Error("A shader pass may expose at most 8 color render targets: " + node->name);
        if(shader.nodeType == NodeType::Mesh && shader.mesh && shader.mesh->depth &&
           shader.extraRenderFormats.size() > 6)
            throw Error("A depth-enabled mesh may expose at most 7 color render targets: " + node->name);
        if(shader.iterations == 0 || shader.iterations > 4096)
            throw Error("Shader pass iterations must be in the range 1..4096: " + node->name);
        if(node->getNodeType() != NodeType::Compute && shader.iterations != 1)
            throw Error("Pass iterations are only supported for compute shaders: " + node->name);
        if(node->getNodeType() != NodeType::Compute &&
           (shader.localSizeX != 8 || shader.localSizeY != 8 || shader.localSizeZ != 1))
            throw Error("Compute local size is only supported for compute shaders: " + node->name);
        if(node->getNodeType() == NodeType::Compute) {
            if(shader.localSizeX == 0 || shader.localSizeY == 0 || shader.localSizeZ == 0)
                throw Error("Compute local sizes must be positive: " + node->name);
            const uint64_t invocations =
                static_cast<uint64_t>(shader.localSizeX) * shader.localSizeY * shader.localSizeZ;
            if(invocations > 1024)
                throw Error("Compute local workgroup size exceeds 1024 invocations: " + node->name);
            if(shader.localSizeZ != 1)
                throw Error("Compute local workgroup Z size must be 1 for the 2D mainCompute entrypoint: " + node->name);
        }
        if(node->getNodeType() == NodeType::CubeMap) {
            if(!shader.extraRenderFormats.empty())
                throw Error("Cubemap shaders cannot expose extra render targets: " + node->name);
            if(shader.renderFormat != RenderFormat::RGBA32F)
                throw Error("Cubemap shaders do not support explicit render formats: " + node->name);
        }
        if(node == directRenderNode && shader.renderFormat != RenderFormat::RGBA32F)
            throw Error("The final image pass cannot use an explicit offscreen render format");

        if(node->getNodeType() == NodeType::Image || node->getNodeType() == NodeType::Compute ||
           node->getNodeType() == NodeType::Mesh) {
            if((shader.fixedWidth == 0) != (shader.fixedHeight == 0))
                throw Error("Shader pass has a partial fixed resolution: " + node->name);
            if(node->getNodeType() == NodeType::Compute && shader.fixedWidth == 0)
                throw Error("Compute pass requires a fixed resolution: " + node->name);
            if(shader.fixedWidth != 0) {
                if(node == directRenderNode)
                    throw Error("The final image pass cannot have a fixed offscreen resolution");
                if(shader.fixedWidth > static_cast<uint32_t>(std::numeric_limits<int32_t>::max()) ||
                   shader.fixedHeight > static_cast<uint32_t>(std::numeric_limits<int32_t>::max()))
                    throw Error("Shader pass fixed resolution exceeds the OpenGL dimension range: " + node->name);
                textureSizeMap.emplace(
                    node, Vec2{ static_cast<float>(shader.fixedWidth), static_cast<float>(shader.fixedHeight) });
            }
            if(node == directRenderNode && !shader.extraRenderFormats.empty())
                throw Error("The final image pass cannot expose extra render targets");
            std::vector<DoubleBufferedFB> targets;
            std::vector<RenderFormat> formats;
            formats.reserve(1 + shader.extraRenderFormats.size() +
                            (shader.nodeType == NodeType::Mesh && shader.mesh && shader.mesh->depth ? 1U : 0U));
            formats.push_back(shader.renderFormat);
            formats.insert(formats.end(), shader.extraRenderFormats.begin(), shader.extraRenderFormats.end());
            if(shader.nodeType == NodeType::Mesh && shader.mesh && shader.mesh->depth)
                formats.push_back(RenderFormat::R32F);
            targets.reserve(formats.size());
            for(const auto format : formats) {
                if(requiredDoubleBuffer.contains(node)) {
                    targets.emplace_back(pipeline->createFrameBuffer(format), pipeline->createFrameBuffer(format));
                } else if(node != directRenderNode) {
                    targets.emplace_back(pipeline->createFrameBuffer(format));
                } else {
                    targets.emplace_back(nullptr);
                }
            }
            frameBufferMap.emplace(node, std::move(targets));
        } else if(node->getNodeType() == NodeType::CubeMap) {
            if(node == directRenderNode)
                throw Error("A cubemap pass cannot be the final image output");

            std::vector<DoubleBufferedFB> targets;
            targets.reserve(6);
            if(requiredDoubleBuffer.contains(node)) {
                const auto first = pipeline->createCubeMapFrameBuffer();
                const auto second = pipeline->createCubeMapFrameBuffer();
                if(first.size() != 6 || second.size() != 6)
                    throw Error("Invalid cubemap framebuffer allocation");
                for(uint32_t face = 0; face < 6; ++face)
                    targets.emplace_back(first[face], second[face]);
            } else {
                const auto first = pipeline->createCubeMapFrameBuffer();
                if(first.size() != 6)
                    throw Error("Invalid cubemap framebuffer allocation");
                for(auto* face : first)
                    targets.emplace_back(face);
            }
            frameBufferMap.emplace(node, std::move(targets));
        } else {
            throw Error("Unsupported shader pass type");
        }
    }

    for(const auto* node : order) {
        switch(node->getNodeClass()) {
            case NodeClass::GLSLShader: {
                auto& targets = frameBufferMap.at(node);
                std::vector<Channel> channels;
                if(const auto found = inputs.find(node); found != inputs.end()) {
                    channels.reserve(found->second.size());
                    for(const auto* link : found->second) {
                        const auto texture = textureMap.find(link->start);
                        if(texture == textureMap.end())
                            throw Error("Shader input was not prepared before its consumer");
                        uint32_t sourceOutput = link->sourceOutput;
                        if(sourceOutput == std::numeric_limits<uint32_t>::max()) {
                            const auto* sourceShader = dynamic_cast<const GLSLShader*>(link->start);
                            if(!sourceShader || sourceShader->nodeType != NodeType::Mesh || !sourceShader->mesh ||
                               !sourceShader->mesh->depth)
                                throw Error("Shader input source has no readable mesh depth");
                            sourceOutput = static_cast<uint32_t>(1 + sourceShader->extraRenderFormats.size());
                        }
                        if(sourceOutput >= texture->second.size())
                            throw Error("Shader input render-target index is out of range");
                        std::optional<Vec2> size;
                        if(const auto knownSize = textureSizeMap.find(link->start); knownSize != textureSizeMap.end())
                            size = knownSize->second;
                        channels.push_back(
                            Channel{ link->slot, texture->second[sourceOutput], link->filter, link->wrapMode, size });
                    }
                }

                const auto& shader = dynamic_cast<const GLSLShader&>(*node);
                std::vector<std::pair<uint32_t, BufferId>> storageBuffers;
                storageBuffers.reserve(shader.storageBuffers.size());
                std::unordered_set<uint32_t> usedStorageBindings;
                for(const auto& storage : shader.storageBuffers) {
                    if(storage.name.empty() || storage.size == 0)
                        throw Error("Storage buffers require a name and positive size: " + node->name);
                    if(!usedStorageBindings.emplace(storage.binding).second)
                        throw Error("Duplicate storage buffer binding " + std::to_string(storage.binding) +
                                    " in pass " + node->name);
                    auto found = storageBufferMap.find(storage.name);
                    if(found == storageBufferMap.end()) {
                        const auto id = pipeline->createStorageBuffer(storage.name, storage.size);
                        found = storageBufferMap.emplace(storage.name, std::pair<uint64_t, BufferId>{ storage.size, id }).first;
                    } else if(found->second.first != storage.size) {
                        throw Error("Storage buffer '" + storage.name + "' uses inconsistent sizes");
                    }
                    storageBuffers.emplace_back(storage.binding, found->second.second);
                }
                try {
                    std::optional<Vec2> fixedResolution;
                    if(shader.fixedWidth != 0)
                        fixedResolution = Vec2{ static_cast<float>(shader.fixedWidth), static_cast<float>(shader.fixedHeight) };
                    pipeline->addPass(node->name, shader.source, shader.nodeType, targets, std::move(channels), fixedResolution,
                                      node == directRenderNode, shader.renderFormat, shader.extraRenderFormats,
                                      shader.iterations, shader.localSizeX, shader.localSizeY, shader.localSizeZ,
                                      std::move(storageBuffers), shader.mesh);
                } catch(const std::exception& error) {
                    throw Error("Pass '" + node->name + "': " + error.what());
                }

                if(targets.front().t1) {
                    const auto texType = shader.nodeType == NodeType::CubeMap ? TexType::CubeMap : TexType::Tex2D;
                    std::vector<DoubleBufferedTex> textures;
                    if(shader.nodeType == NodeType::CubeMap) {
                        textures.emplace_back(targets.front().t1->getTexture(), targets.front().t2->getTexture(), texType);
                    } else {
                        textures.reserve(targets.size());
                        for(const auto& target : targets)
                            textures.emplace_back(target.t1->getTexture(), target.t2->getTexture(), texType);
                    }
                    textureMap.emplace(node, std::move(textures));
                }
                break;
            }
            case NodeClass::LastFrame: {
                const auto& lastFrame = dynamic_cast<const LastFrame&>(*node);
                const auto& refTargets = frameBufferMap.at(lastFrame.refNode);
                if(lastFrame.refOutput >= refTargets.size())
                    throw Error("LastFrame render-target index is out of range");
                const auto target = refTargets[lastFrame.refOutput];
                if(!target.t1 || !target.t2)
                    throw Error("LastFrame source is not double buffered");
                textureMap.emplace(
                    node, std::vector<DoubleBufferedTex>{ DoubleBufferedTex{
                              target.t2->getTexture(), target.t1->getTexture(),
                              lastFrame.refNode->getNodeType() == NodeType::CubeMap ? TexType::CubeMap : TexType::Tex2D } });
                if(const auto knownSize = textureSizeMap.find(lastFrame.refNode); knownSize != textureSizeMap.end())
                    textureSizeMap.emplace(node, knownSize->second);
                break;
            }
            case NodeClass::RenderOutput:
                break;
            case NodeClass::Texture: {
                const auto& texture = dynamic_cast<const Texture&>(*node);
                if(texture.width == 0 || texture.height == 0 ||
                   texture.width > static_cast<uint32_t>(std::numeric_limits<int>::max()) ||
                   texture.height > static_cast<uint32_t>(std::numeric_limits<int>::max()))
                    throw Error("Texture node has invalid dimensions");
                if(texture.pixel.size() != checkedSizeProduct({ texture.width, texture.height }, "Texture"))
                    throw Error("Texture node has an invalid pixel payload");
                const auto id = pipeline->createTexture(node->name, texture.width, texture.height, texture.pixel.data());
                textureSizeMap.emplace(node, Vec2{ static_cast<float>(texture.width), static_cast<float>(texture.height) });
                textureMap.emplace(node, std::vector<DoubleBufferedTex>{ DoubleBufferedTex{ id, TexType::Tex2D } });
                break;
            }
            case NodeClass::CubeMap: {
                const auto& texture = dynamic_cast<const CubeMap&>(*node);
                if(texture.size == 0 || texture.size > static_cast<uint32_t>(std::numeric_limits<int>::max()))
                    throw Error("Cubemap node has an invalid size");
                if(texture.pixel.size() != checkedSizeProduct({ texture.size, texture.size, 6U }, "Cubemap"))
                    throw Error("Cubemap node has an invalid pixel payload");
                const auto id = pipeline->createCubeMap(texture.size, texture.pixel.data());
                textureSizeMap.emplace(node, Vec2{ static_cast<float>(texture.size), static_cast<float>(texture.size) });
                textureMap.emplace(node, std::vector<DoubleBufferedTex>{ DoubleBufferedTex{ id, TexType::CubeMap } });
                break;
            }
            case NodeClass::Volume: {
                const auto& volume = dynamic_cast<const Volume&>(*node);
                if(volume.size == 0 || volume.size > static_cast<uint32_t>(std::numeric_limits<int>::max()))
                    throw Error("Volume node has an invalid size");
                if(volume.channels != 1 && volume.channels != 4)
                    throw Error("Volume node channel count must be 1 or 4");
                const auto expected = checkedSizeProduct({ volume.size, volume.size, volume.size, volume.channels }, "Volume");
                if(volume.pixel.size() != expected)
                    throw Error("Volume node has an invalid voxel payload");
                const auto id = pipeline->createVolume(volume.size, volume.channels, volume.pixel.data());
                textureSizeMap.emplace(node, Vec2{ static_cast<float>(volume.size), static_cast<float>(volume.size) });
                textureMap.emplace(node, std::vector<DoubleBufferedTex>{ DoubleBufferedTex{ id, TexType::Tex3D } });
                break;
            }
            case NodeClass::Keyboard: {
                const auto id = pipeline->createKeyboardTexture();
                textureSizeMap.emplace(
                    node, Vec2{ static_cast<float>(KeyboardInput::KeyCount), static_cast<float>(KeyboardInput::Rows) });
                textureMap.emplace(node, std::vector<DoubleBufferedTex>{ DoubleBufferedTex{ id, TexType::Tex2D } });
                break;
            }
            case NodeClass::Music: {
                const auto id = pipeline->createAudioTexture();
                textureSizeMap.emplace(
                    node, Vec2{ static_cast<float>(AudioInput::TextureWidth), static_cast<float>(AudioInput::TextureHeight) });
                textureMap.emplace(node, std::vector<DoubleBufferedTex>{ DoubleBufferedTex{ id, TexType::Tex2D } });
                break;
            }
            case NodeClass::SoundOutput:
            case NodeClass::Unknown:
                throw Error("Unsupported node class in renderer");
        }
    }

    return pipeline;
}

SHADERTOY_NAMESPACE_END
