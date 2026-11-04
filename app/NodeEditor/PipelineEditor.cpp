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

#define IMGUI_DEFINE_MATH_OPERATORS
#include "NodeEditor/PipelineEditor.hpp"
#include "FileDialog.hpp"
#include "shadertoy/Support.hpp"

#include <chrono>
#include <queue>

#include "shadertoy/SuppressWarningPush.hpp"

#include <GL/glew.h>
#include <GLFW/glfw3.h>
#include <fmt/format.h>
#include <hello_imgui/dpi_aware.h>
#include <hello_imgui/hello_imgui.h>
#include <hello_imgui/image_from_asset.h>
#include <imgui_stdlib.h>
#include <magic_enum/magic_enum.hpp>
#include <stb_image.h>

using HelloImGui::EmToVec2;

#include "shadertoy/SuppressWarningPop.hpp"

SHADERTOY_NAMESPACE_BEGIN

ShaderToyEditor::ShaderToyEditor() {
    const auto lang = TextEditor::LanguageDefinition::GLSL();
    // TODO: more keywords/built-ins
    mEditor.SetLanguageDefinition(lang);
    mEditor.SetTabSize(4);
    mEditor.SetShowWhitespaces(false);
    mEditor.SetText(R"(void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    fragColor = vec4(0.0,0.0,1.0,1.0);
})");
}

[[nodiscard]] std::string ShaderToyEditor::getText() const {
    return mEditor.GetText();
}

void ShaderToyEditor::setText(const std::string& str) {
    mEditor.SetText(str);
}

void ShaderToyEditor::render(const ImVec2 size) {
    const auto cpos = mEditor.GetCursorPosition();
    ImGui::Text("%6d/%-6d %6d lines  %s", cpos.mLine + 1, cpos.mColumn + 1, mEditor.GetTotalLines(),
                mEditor.IsOverwrite() ? "Ovr" : "Ins");
    mEditor.Render("TextEditor", size, false);
}

static constexpr auto initialShader = R"(void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    // Normalized pixel coordinates (from 0 to 1)
    vec2 uv = fragCoord/iResolution.xy;

    // Time varying pixel color
    vec3 col = 0.5 + 0.5*cos(iTime+uv.xyx+vec3(0,2,4));

    // Output to screen
    fragColor = vec4(col,1.0);
}
)";

uint32_t PipelineEditor::nextId() {
    return mNextId++;
}

void PipelineEditor::setupInitialPipeline() {
    auto& shader = spawnShader(NodeType::Image);
    shader.editor.setText(initialShader);
    auto& sink = spawnRenderOutput();

    mLinks.emplace_back(nextId(), shader.outputs.front().id, sink.inputs.front().id);
}

PipelineEditor::PipelineEditor() {
    const ed::Config config;
    mCtx = ed::CreateEditor(&config);
    mHeaderBackground = 0;

    setupInitialPipeline();
    mShouldBuildPipeline = true;
    mShouldResetLayout = true;
}
void PipelineEditor::resetPipeline() {
    mNodes.clear();
    mLinks.clear();
    mMetadata.clear();
    setupInitialPipeline();
    mShouldBuildPipeline = true;
    mShouldResetLayout = true;
}

PipelineEditor::~PipelineEditor() {
    ed::DestroyEditor(mCtx);
}

void PipelineEditor::resetLayout() {
    std::unordered_map<EditorNode*, std::vector<std::pair<EditorNode*, uint32_t>>> graph;
    std::unordered_map<EditorNode*, uint32_t> degree;
    for(auto& link : mLinks) {
        auto u = findPin(link.startPinId);
        auto v = findPin(link.endPinId);
        auto idx = static_cast<uint32_t>(v - v->node->inputs.data());
        graph[v->node].emplace_back(u->node, idx);
        ++degree[u->node];
    }

    std::queue<EditorNode*> q;
    std::unordered_map<EditorNode*, uint32_t> depth;
    for(auto& node : mNodes)
        if(!degree.count(node.get()))
            q.push(node.get());
    while(!q.empty()) {
        auto u = q.front();
        q.pop();
        for(auto [v, idx] : graph[u]) {
            depth[v] = std::max(depth[v], depth[u] + 1);
            if(--degree[v] == 0) {
                q.push(v);
            }
        }
    }

    std::map<uint32_t, std::vector<EditorNode*>> layers;
    std::unordered_map<const EditorNode*, std::pair<double, uint32_t>> barycenter;
    for(auto [u, d] : depth)
        layers[d].push_back(u);

    float selfX = 0;
    for(auto& [d, layer] : layers) {
        constexpr auto width = 500.0f;
        auto getBarycenter = [&](const EditorNode* u) {
            if(const auto iter = barycenter.find(u); iter != barycenter.cend()) {
                return iter->second.first / iter->second.second;
            }
            return 0.0;
        };
        std::sort(layer.begin(), layer.end(),
                  [&](const EditorNode* u, const EditorNode* v) { return getBarycenter(u) < getBarycenter(v); });
        double pos = 0;
        float selfY = 0;
        for(auto u : layer) {
            constexpr auto height = 300.0f;
            ++pos;
            for(auto [v, idx] : graph[u]) {
                auto& [sum, count] = barycenter[v];
                sum += pos + idx;
                ++count;
            }
            pos += static_cast<double>(u->inputs.size());

            ed::SetNodePosition(u->id, ImVec2{ selfX, selfY });
            selfY += height;
        }
        selfX -= width;
    }

    mShouldZoomToContent = true;
}

bool PipelineEditor::isUniqueName(const std::string_view& name, const EditorNode* exclude) const {
    return std::all_of(mNodes.cbegin(), mNodes.cend(), [&](auto& node) { return node.get() == exclude || node->name != name; });
}
std::string PipelineEditor::generateUniqueName(const std::string_view& base) const {
    if(isUniqueName(base, nullptr))
        return { base.data(), base.size() };
    for(uint32_t idx = 1;; ++idx) {
        if(auto str = fmt::format("{}{}", base, idx); isUniqueName(str, nullptr)) {
            return str;
        }
    }
}

template <typename T>
static auto& buildNode(std::vector<std::unique_ptr<EditorNode>>& nodes, std::unique_ptr<T> node) {
    for(auto& input : node->inputs) {
        input.node = node.get();
        input.kind = PinKind::Input;
    }

    for(auto& output : node->outputs) {
        output.node = node.get();
        output.kind = PinKind::Output;
    }
    auto& ref = *node;
    nodes.push_back(std::move(node));
    return ref;
}
EditorTexture& PipelineEditor::spawnTexture() {
    auto ret = std::make_unique<EditorTexture>(nextId(), generateUniqueName("Texture"));
    ret->outputs.emplace_back(nextId(), "Output", NodeType::Image);
    return buildNode(mNodes, std::move(ret));
}
EditorCubeMap& PipelineEditor::spawnCubeMap() {
    auto ret = std::make_unique<EditorCubeMap>(nextId(), generateUniqueName("CubeMap"));
    ret->type = NodeType::CubeMap;
    ret->outputs.emplace_back(nextId(), "Output", NodeType::CubeMap);
    return buildNode(mNodes, std::move(ret));
}
EditorVolume& PipelineEditor::spawnVolume() {
    auto ret = std::make_unique<EditorVolume>(nextId(), generateUniqueName("Volume"));
    ret->type = NodeType::Volume;
    ret->outputs.emplace_back(nextId(), "Output", NodeType::Volume);
    return buildNode(mNodes, std::move(ret));
}
EditorKeyboard& PipelineEditor::spawnKeyboard() {
    auto ret = std::make_unique<EditorKeyboard>(nextId(), generateUniqueName("Keyboard"));
    ret->outputs.emplace_back(nextId(), "Output", NodeType::Image);
    return buildNode(mNodes, std::move(ret));
}
EditorMusic& PipelineEditor::spawnMusic() {
    auto ret = std::make_unique<EditorMusic>(nextId(), generateUniqueName("Music"));
    ret->outputs.emplace_back(nextId(), "Output", NodeType::Image);
    return buildNode(mNodes, std::move(ret));
}
EditorRenderOutput& PipelineEditor::spawnRenderOutput() {
    auto ret = std::make_unique<EditorRenderOutput>(nextId(), generateUniqueName("RenderOutput"));
    ret->inputs.emplace_back(nextId(), "Input", NodeType::Image);
    return buildNode(mNodes, std::move(ret));
}
EditorLastFrame& PipelineEditor::spawnLastFrame() {
    auto ret = std::make_unique<EditorLastFrame>(nextId(), generateUniqueName("LastFrame"));
    ret->outputs.emplace_back(nextId(), "Output", NodeType::Image);
    return buildNode(mNodes, std::move(ret));
}
EditorShader& PipelineEditor::spawnShader(NodeType type) {
    auto ret = std::make_unique<EditorShader>(nextId(), generateUniqueName("Shader"));
    ret->type = type;
    for(uint32_t idx = 0; idx < 4; ++idx) {
        ret->inputs.emplace_back(nextId(), fmt::format("Channel{}", idx).c_str(), NodeType::Image);
    }
    ret->outputs.emplace_back(nextId(), "Output", type);
    return buildNode(mNodes, std::move(ret));
}

static ImColor getIconColor(const NodeType type) {
    switch(type) {
        case NodeType::Image:
        case NodeType::Mesh:
            return { 255, 0, 0 };
        case NodeType::CubeMap:
            return { 0, 255, 0 };
        case NodeType::Volume:
            return { 0, 255, 255 };
        case NodeType::Sound:
            return { 0, 0, 255 };
        case NodeType::Compute:
            return { 255, 128, 0 };
    }
    return {};
}

static void drawPinIcon(const EditorPin& pin, const bool connected, const int alpha) {
    IconType iconType = IconType::Square;
    ImColor color = getIconColor(pin.type);
    color.Value.w = static_cast<float>(alpha) / 255.0f;
    switch(pin.type) {
        case NodeType::Image:
        case NodeType::Mesh:
            iconType = IconType::Square;
            break;
        case NodeType::CubeMap:
            iconType = IconType::Diamond;
            break;
        case NodeType::Sound:
            iconType = IconType::Circle;
            break;
        case NodeType::Volume:
            iconType = IconType::RoundSquare;
            break;
        case NodeType::Compute:
            iconType = IconType::Square;
            break;
    }

    ax::Widgets::icon(EmToVec2(1, 1), iconType, connected, color, ImColor(32, 32, 32, alpha));
}

bool PipelineEditor::canCreateLink(const EditorPin* startPin, const EditorPin* endPin) const {
    if(endPin == startPin) {
        return false;
    }
    if(endPin->kind == startPin->kind) {
        return false;
    }
    /*
    if(endPin->type != startPin->type) {
        return false;
    }
    */
    if(endPin->node == startPin->node) {
        return false;
    }
    if(isPinLinked(endPin->id)) {
        return false;
    }
    return true;
}

bool PipelineEditor::isPinLinked(ed::PinId id) const {
    if(!id)
        return false;

    return std::any_of(mLinks.cbegin(), mLinks.cend(), [id](auto& link) { return link.startPinId == id || link.endPinId == id; });
}

EditorNode* PipelineEditor::findNode(const ed::NodeId id) const {
    if(!id)
        return nullptr;

    for(auto& node : mNodes)
        if(node->id == id)
            return node.get();

    return nullptr;
}

EditorPin* PipelineEditor::findPin(const ed::PinId id) const {
    if(!id)
        return nullptr;

    for(auto& node : mNodes) {
        for(auto& pin : node->inputs)
            if(pin.id == id)
                return &pin;

        for(auto& pin : node->outputs)
            if(pin.id == id)
                return &pin;
    }

    return nullptr;
}

void PipelineEditor::renderEditor() {
    ed::Begin("##PipelineEditor", ImVec2(0.0, 0.0));
    ax::NodeEditor::Utilities::BlueprintNodeBuilder builder(mHeaderBackground, 64, 64);

    const auto cursorTopLeft = ImGui::GetCursorScreenPos();

    mShaderNodeNames.clear();
    mShaderNodes.clear();
    const EditorNode* directRenderNode = nullptr;
    for(const auto& link : mLinks) {
        const auto u = findPin(link.startPinId);
        const auto v = findPin(link.endPinId);
        if(v->node->getClass() == NodeClass::RenderOutput && u->node->getClass() == NodeClass::GLSLShader) {
            directRenderNode = u->node;
        }
    }
    for(auto& node : mNodes) {
        if(node->getClass() == NodeClass::GLSLShader) {
            if(node.get() == directRenderNode)
                continue;
            mShaderNodeNames.push_back(node->name.c_str());
            mShaderNodes.push_back(node.get());
        }
    }

    for(auto& node : mNodes) {
        builder.begin(node->id);
        builder.header(node->color);
        if(node->rename) {
            if(ImGui::InputText("##Name", &node->name, ImGuiInputTextFlags_EnterReturnsTrue | ImGuiInputTextFlags_CharsNoBlank)) {
                if(isUniqueName(node->name, node.get())) {
                    node->rename = false;
                } else {
                    HelloImGui::Log(HelloImGui::LogLevel::Error, "Please specify a unique name for this node");
                }
            }
        } else
            ImGui::TextUnformatted(node->name.c_str());
        ImGui::SameLine();
        ImGui::Dummy(EmToVec2(0, 1.5));
        builder.endHeader();

        constexpr auto disabledAlphaScale = 48.0f / 255.0f;

        for(auto& input : node->inputs) {
            auto alpha = ImGui::GetStyle().Alpha;
            if(mNewLinkPin && !canCreateLink(mNewLinkPin, &input) && &input != mNewLinkPin)
                alpha *= disabledAlphaScale;

            builder.input(input.id);
            ImGui::PushStyleVar(ImGuiStyleVar_Alpha, alpha);
            const bool linked = isPinLinked(input.id);
            drawPinIcon(input, linked, static_cast<int>(alpha * 255));
            ImGui::SameLine();
            if(!input.name.empty()) {
                ImGui::TextUnformatted(input.name.c_str());
                if(linked && node->getClass() == NodeClass::GLSLShader)
                    ImGui::SameLine();
            }
            if(linked && node->getClass() == NodeClass::GLSLShader) {
                for(auto& link : mLinks) {
                    if(link.endPinId == input.id) {
                        if(ImGui::Button(magic_enum::enum_name(link.filter).data())) {
                            link.filter = static_cast<Filter>((static_cast<uint32_t>(link.filter) + 1) %
                                                              static_cast<uint32_t>(magic_enum::enum_count<Filter>()));
                        }
                        if(ImGui::Button(magic_enum::enum_name(link.wrapMode).data())) {
                            link.wrapMode = static_cast<Wrap>((static_cast<uint32_t>(link.wrapMode) + 1) %
                                                              static_cast<uint32_t>(magic_enum::enum_count<Wrap>()));
                        }
                        break;
                    }
                }
            }
            ImGui::PopStyleVar();
            builder.endInput();
        }

        for(auto& output : node->outputs) {
            auto alpha = ImGui::GetStyle().Alpha;
            if(mNewLinkPin && !canCreateLink(mNewLinkPin, &output) && &output != mNewLinkPin)
                alpha *= disabledAlphaScale;

            ImGui::PushStyleVar(ImGuiStyleVar_Alpha, alpha);
            builder.output(output.id);
            if(!output.name.empty()) {
                ImGui::TextUnformatted(output.name.c_str());
                ImGui::SameLine();
            }
            mShouldBuildPipeline |= node->renderContent();
            ImGui::SameLine();
            drawPinIcon(output, isPinLinked(output.id), static_cast<int>(alpha * 255));
            ImGui::PopStyleVar();
            builder.endOutput();
        }

        builder.end();

        if(node->getClass() == NodeClass::LastFrame) {
            dynamic_cast<EditorLastFrame*>(node.get())->renderPopup();
        }
    }

    for(const auto& link : mLinks)
        ed::Link(link.id, link.startPinId, link.endPinId, ImColor(255, 255, 255), 2.0f);

    if(!mOnNodeCreate) {
        if(ed::BeginCreate(ImColor(255, 255, 255), 2.0f)) {
            auto showLabel = [](const char* label, ImColor color) {
                ImGui::SetCursorPosY(ImGui::GetCursorPosY() - ImGui::GetTextLineHeight());
                const auto size = ImGui::CalcTextSize(label);

                const auto padding = ImGui::GetStyle().FramePadding;
                const auto spacing = ImGui::GetStyle().ItemSpacing;

                ImGui::SetCursorPos(ImGui::GetCursorPos() + ImVec2(spacing.x, -spacing.y));

                const auto rectMin = ImGui::GetCursorScreenPos() - padding;
                const auto rectMax = ImGui::GetCursorScreenPos() + size + padding;

                const auto drawList = ImGui::GetWindowDrawList();
                drawList->AddRectFilled(rectMin, rectMax, color, size.y * 0.15f);
                ImGui::TextUnformatted(label);
            };

            ed::PinId startPinId = 0, endPinId = 0;
            if(ed::QueryNewLink(&startPinId, &endPinId)) {
                auto startPin = findPin(startPinId);
                auto endPin = findPin(endPinId);

                mNewLinkPin = startPin ? startPin : endPin;

                if(startPin->kind == PinKind::Input) {
                    std::swap(startPin, endPin);
                    std::swap(startPinId, endPinId);
                }

                if(startPin && endPin) {
                    if(endPin == startPin) {
                        ed::RejectNewItem(ImColor(255, 0, 0), 2.0f);
                    } else if(endPin->kind == startPin->kind) {
                        showLabel("x Incompatible Pin Kind", ImColor(45, 32, 32, 180));
                        ed::RejectNewItem(ImColor(255, 0, 0), 2.0f);
                    }
                    /* else if(endPin->type != startPin->type) {
                        showLabel("x Incompatible Pin Type", ImColor(45, 32, 32, 180));
                        ed::RejectNewItem(ImColor(255, 128, 128), 1.0f);
                    }*/
                    else if(endPin->node == startPin->node) {
                        showLabel("x Self Loop", ImColor(45, 32, 32, 180));
                        ed::RejectNewItem(ImColor(255, 128, 128), 1.0f);
                    } else if(isPinLinked(endPin->id)) {
                        showLabel("x Multiple Inputs", ImColor(45, 32, 32, 180));
                        ed::RejectNewItem(ImColor(255, 128, 128), 1.0f);
                    } else {
                        showLabel("+ Create Link", ImColor(32, 45, 32, 180));
                        if(ed::AcceptNewItem(ImColor(128, 255, 128), 4.0f)) {
                            mLinks.emplace_back(nextId(), startPinId, endPinId);
                        }
                    }
                }
            }

            ed::PinId pinId = 0;
            if(ed::QueryNewNode(&pinId)) {
                mNewLinkPin = findPin(pinId);
                if(mNewLinkPin)
                    showLabel("+ Create Node", ImColor(32, 45, 32, 180));

                if(ed::AcceptNewItem()) {
                    mOnNodeCreate = true;
                    mNewNodeLinkPin = findPin(pinId);
                    mNewLinkPin = nullptr;
                    ed::Suspend();
                    ImGui::OpenPopup("Create New Node");
                    ed::Resume();
                }
            }
        } else
            mNewLinkPin = nullptr;

        ed::EndCreate();

        if(ed::BeginDelete()) {
            ed::LinkId linkId = 0;
            while(ed::QueryDeletedLink(&linkId)) {
                if(ed::AcceptDeletedItem()) {
                    const auto id = std::find_if(mLinks.cbegin(), mLinks.cend(),
                                                 [linkId](const EditorLink& link) { return link.id == linkId; });
                    if(id != mLinks.end())
                        mLinks.erase(id);
                }
            }

            ed::NodeId nodeId = 0;
            while(ed::QueryDeletedNode(&nodeId)) {
                if(ed::AcceptDeletedItem()) {
                    auto id = std::find_if(mNodes.cbegin(), mNodes.cend(),
                                           [nodeId](const std::unique_ptr<EditorNode>& node) { return node->id == nodeId; });

                    if(id != mNodes.end()) {
                        mLinks.erase(std::remove_if(mLinks.begin(), mLinks.end(),
                                                    [&](auto& link) {
                                                        auto u = findPin(link.startPinId);
                                                        auto v = findPin(link.endPinId);
                                                        return (u->node == id->get() || v->node == id->get());
                                                    }),
                                     mLinks.end());
                        mNodes.erase(id);
                    }
                }
            }
        }
        ed::EndDelete();
    }
    ImGui::SetCursorScreenPos(cursorTopLeft);

    const auto openPopupPosition = ImGui::GetMousePos();
    ed::Suspend();

    if(ed::ShowNodeContextMenu(&mContextNodeId))
        ImGui::OpenPopup("Node Context Menu");
    else if(ed::ShowLinkContextMenu(&mContextLinkId))
        ImGui::OpenPopup("Link Context Menu");
    else if(ed::ShowBackgroundContextMenu()) {
        ImGui::OpenPopup("Create New Node");
        mNewNodeLinkPin = nullptr;
    }

    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, EmToVec2(0.25, 0.25));
    if(ImGui::BeginPopup("Node Context Menu")) {
        const auto node = findNode(mContextNodeId);
        if(!node->rename && ImGui::MenuItem("Rename")) {
            node->rename = true;
        }
        if(node->getClass() != NodeClass::RenderOutput && ImGui::MenuItem("Delete"))
            ed::DeleteNode(mContextNodeId);
        ImGui::EndPopup();
    }

    if(ImGui::BeginPopup("Link Context Menu")) {
        if(ImGui::MenuItem("Delete"))
            ed::DeleteLink(mContextLinkId);
        ImGui::EndPopup();
    }
    if(ImGui::BeginPopup("Create New Node")) {
        const auto newNodePosition = openPopupPosition;

        EditorNode* node = nullptr;

        ImGui::TextUnformatted("New Node");

        ImGui::Separator();
        if(ImGui::MenuItem("Texture"))
            node = &spawnTexture();
        if(ImGui::MenuItem("CubeMap"))
            node = &spawnCubeMap();
        if(ImGui::MenuItem("LastFrame"))
            node = &spawnLastFrame();
        auto hasClass = [&](NodeClass nodeClass) {
            return std::any_of(mNodes.begin(), mNodes.end(), [&](const auto& it) { return it->getClass() == nodeClass; });
        };
        if(!hasClass(NodeClass::Keyboard) && ImGui::MenuItem("Keyboard"))
            node = &spawnKeyboard();
        if(!hasClass(NodeClass::Music) && ImGui::MenuItem("Music"))
            node = &spawnMusic();

        ImGui::Separator();
        if(ImGui::MenuItem("Shader"))
            node = &spawnShader(NodeType::Image);

        ImGui::Separator();
        if(!hasClass(NodeClass::RenderOutput) && ImGui::MenuItem("Render Output"))
            node = &spawnRenderOutput();

        if(node) {
            mOnNodeCreate = false;

            ed::SetNodePosition(node->id, newNodePosition);

            if(auto startPin = mNewNodeLinkPin) {
                auto& pins = startPin->kind == PinKind::Input ? node->outputs : node->inputs;

                for(auto& pin : pins) {
                    auto endPin = &pin;
                    if(startPin->kind == PinKind::Input)
                        std::swap(startPin, endPin);
                    if(canCreateLink(startPin, endPin)) {
                        mLinks.emplace_back(nextId(), startPin->id, endPin->id);
                        break;
                    }
                }
            }
        }

        ImGui::EndPopup();
    } else
        mOnNodeCreate = false;
    ImGui::PopStyleVar();
    ed::Resume();

    ed::End();
}

ShaderDocument PipelineEditor::makeDocument() const {
    ShaderDocument document;
    for(const auto& [key, value] : mMetadata)
        document.metadata.emplace(key, value);

    std::unordered_map<const EditorNode*, Node*> nodeMap;
    for(const auto& editorNode : mNodes) {
        if(editorNode->getClass() == NodeClass::LastFrame && !dynamic_cast<const EditorLastFrame&>(*editorNode).lastFrame) {
            throw Error("LastFrame node '" + editorNode->name + "' has no source");
        }
        if(editorNode->getClass() == NodeClass::Texture) {
            const auto& texture = dynamic_cast<const EditorTexture&>(*editorNode);
            if(texture.width == 0 || texture.height == 0 ||
               texture.pixel.size() != static_cast<std::size_t>(texture.width) * texture.height) {
                throw Error("Texture node '" + editorNode->name + "' has no valid texture");
            }
        }
        if(editorNode->getClass() == NodeClass::CubeMap) {
            const auto& cubemap = dynamic_cast<const EditorCubeMap&>(*editorNode);
            if(cubemap.size == 0 || cubemap.pixel.size() != static_cast<std::size_t>(cubemap.size) * cubemap.size * 6U) {
                throw Error("CubeMap node '" + editorNode->name + "' has no valid cubemap");
            }
        }
        if(editorNode->getClass() == NodeClass::Volume) {
            const auto& volume = dynamic_cast<const EditorVolume&>(*editorNode);
            const auto expected = static_cast<std::size_t>(volume.size) * volume.size * volume.size * volume.channels;
            if(volume.size == 0 || (volume.channels != 1 && volume.channels != 4) || volume.pixel.size() != expected) {
                throw Error("Volume node '" + editorNode->name + "' has no valid volume");
            }
        }

        auto node = editorNode->toSTTF();
        node->name = editorNode->name;
        nodeMap.emplace(editorNode.get(), node.get());
        document.nodes.push_back(std::move(node));
    }

    for(std::size_t index = 0; index < mNodes.size(); ++index) {
        if(mNodes[index]->getClass() != NodeClass::LastFrame)
            continue;
        const auto& editorLastFrame = dynamic_cast<const EditorLastFrame&>(*mNodes[index]);
        auto& lastFrame = dynamic_cast<LastFrame&>(*document.nodes[index]);
        lastFrame.refNode = nodeMap.at(editorLastFrame.lastFrame);
        lastFrame.refNodeName = editorLastFrame.lastFrame->name;
    }

    for(const auto& link : mLinks) {
        const auto* startPin = findPin(link.startPinId);
        const auto* endPin = findPin(link.endPinId);
        if(!startPin || !endPin)
            throw Error("Pipeline editor contains a dangling link");
        const auto slot = static_cast<uint32_t>(endPin - endPin->node->inputs.data());
        document.links.push_back(
            Link{ nodeMap.at(startPin->node), nodeMap.at(endPin->node), link.filter, link.wrapMode, slot, link.sourceOutput });
    }

    return document;
}

void PipelineEditor::build(Runtime& runtime) {
    try {
        const auto start = Clock::now();
        auto result = runtime.setDocument(makeDocument());
        if(!result)
            throw result.error();

        const auto duration =
            static_cast<double>(std::chrono::duration_cast<std::chrono::nanoseconds>(Clock::now() - start).count()) * 1e-9;
        HelloImGui::Log(HelloImGui::LogLevel::Info, "Compiled in %.1f secs", duration);
    } catch(const std::exception& error) {
        HelloImGui::Log(HelloImGui::LogLevel::Error, "Build failed: %s", error.what());
    }
}

void PipelineEditor::render(Runtime& runtime) {
    updateNodeType();
    if(!ImGui::Begin("Editor", nullptr)) {
        ImGui::End();
        return;
    }

    if(ImGui::BeginTabBar("##EditorTabBar", ImGuiTabBarFlags_Reorderable)) {
        // pipeline editor
        if(ImGui::BeginTabItem("Pipeline", nullptr, ImGuiTabItemFlags_NoReorder)) {

            ed::SetCurrentEditor(mCtx);
            // toolbar
            if(ImGui::Button("Build")) {
                mShouldBuildPipeline = true;
            }
            ImGui::SameLine();
            if(ImGui::Button("Zoom to context")) {
                mShouldZoomToContent = true;
            }
            if(mShouldZoomToContent) {
                ed::NavigateToContent();
                mShouldZoomToContent = false;
            }
            ImGui::SameLine();
            if(ImGui::Button("Reset layout")) {
                mShouldResetLayout = true;
            }
            if(mShouldResetLayout) {
                resetLayout();
                mShouldResetLayout = false;
            }
            ImGui::SameLine();
            if(ImGui::Button("Edit metadata")) {
                mOpenMetadataEditor = true;
                mMetadataEditorRequestFocus = true;
            }

            renderEditor();
            ed::SetCurrentEditor(nullptr);
            ImGui::EndTabItem();
        }
        if(mOpenMetadataEditor &&
           ImGui::BeginTabItem("Metadata", &mOpenMetadataEditor,
                               mMetadataEditorRequestFocus ? ImGuiTabItemFlags_SetSelected : ImGuiTabItemFlags_None)) {
            if(ImGui::Button("Add item")) {
                mMetadata.emplace_back("Key", "Value");
            }
            if(ImGui::BeginChild("##StringMap")) {
                uint32_t removeIdx = std::numeric_limits<uint32_t>::max();
                uint32_t idx = 0;
                const auto width = ImGui::GetContentRegionAvail().x / 7.0f * 3.0f;
                for(auto& [k, v] : mMetadata) {
                    ImGui::SetNextItemWidth(width);
                    ImGui::InputText(fmt::format("##Key{}", idx).c_str(), &k);
                    ImGui::SameLine();
                    ImGui::SetNextItemWidth(width);
                    ImGui::InputText(fmt::format("##Value{}", idx).c_str(), &v);
                    ImGui::SameLine();
                    if(ImGui::Button("Remove")) {
                        removeIdx = idx;
                    }
                    ++idx;
                }

                if(removeIdx != std::numeric_limits<uint32_t>::max()) {
                    mMetadata.erase(mMetadata.cbegin() + removeIdx);
                }
            }
            ImGui::EndChild();

            mMetadataEditorRequestFocus = false;
            ImGui::EndTabItem();
        }
        // source editor
        for(auto& node : mNodes) {
            if(const auto shader = dynamic_cast<EditorShader*>(node.get())) {
                if(shader->isOpen &&
                   ImGui::BeginTabItem(shader->name.c_str(), &shader->isOpen,
                                       shader->requestFocus ? ImGuiTabItemFlags_SetSelected : ImGuiTabItemFlags_None)) {
                    shader->editor.render(ImVec2(0, 0));
                    shader->requestFocus = false;
                    ImGui::EndTabItem();
                }
            }
        }
        ImGui::EndTabBar();
    }
    ImGui::End();

    if(mShouldBuildPipeline) {
        build(runtime);
        mShouldBuildPipeline = false;
    }
}

PipelineEditor& PipelineEditor::get() {
    static PipelineEditor instance;
    return instance;
}

std::unique_ptr<Node> EditorRenderOutput::toSTTF() const {
    return std::make_unique<RenderOutput>();
}
void EditorRenderOutput::fromSTTF(Node& node) {
    type = node.getNodeType();
}

bool EditorShader::renderContent() {
    if(ImGui::Button("Edit")) {
        isOpen = true;
        requestFocus = true;
    }
    if(ImGui::Button(magic_enum::enum_name(type).data())) {
        type = static_cast<NodeType>((static_cast<uint32_t>(type) + 1) % 3);
    }
    return false;
}
std::unique_ptr<Node> EditorShader::toSTTF() const {
    auto shader = std::make_unique<GLSLShader>(editor.getText(), type);
    shader->fixedWidth = fixedWidth;
    shader->fixedHeight = fixedHeight;
    shader->renderFormat = renderFormat;
    shader->iterations = iterations;
    shader->localSizeX = localSizeX;
    shader->localSizeY = localSizeY;
    shader->localSizeZ = localSizeZ;
    shader->storageBuffers = storageBuffers;
    shader->extraRenderFormats = extraRenderFormats;
    return shader;
}
void EditorShader::fromSTTF(Node& node) {
    const auto& shader = dynamic_cast<GLSLShader&>(node);
    type = shader.nodeType;
    fixedWidth = shader.fixedWidth;
    fixedHeight = shader.fixedHeight;
    renderFormat = shader.renderFormat;
    iterations = shader.iterations;
    localSizeX = shader.localSizeX;
    localSizeY = shader.localSizeY;
    localSizeZ = shader.localSizeZ;
    storageBuffers = shader.storageBuffers;
    extraRenderFormats = shader.extraRenderFormats;
    editor.setText(shader.source);
}

struct ImageStorage final {
    uint32_t width;
    uint32_t height;
    std::vector<uint32_t> data;
};

static ImageStorage loadImageFromFile(const char* path) {
    HelloImGui::Log(HelloImGui::LogLevel::Info, "Loading image %s", path);
    stbi_set_flip_vertically_on_load(true);
    int width, height, channels;
    const auto ptr = stbi_load(path, &width, &height, &channels, 4);
    if(!ptr) {
        HelloImGui::Log(HelloImGui::LogLevel::Error, "Failed to load image %s: %s", path, stbi_failure_reason());
        return { 0, 0, {} };
    }
    auto guard = scopeExit([ptr] { stbi_image_free(ptr); });
    const auto begin = reinterpret_cast<const uint32_t*>(ptr);
    const auto end = begin + static_cast<ptrdiff_t>(width) * height;
    return { static_cast<uint32_t>(width), static_cast<uint32_t>(height), std::vector<uint32_t>{ begin, end } };
}

static void uploadTexturePreview(EditorTexture& texture) {
    if(texture.previewTexture != 0 && glfwGetCurrentContext() != nullptr)
        glDeleteTextures(1, &texture.previewTexture);
    texture.previewTexture = 0;

    if(texture.width == 0 || texture.height == 0 || texture.pixel.empty())
        return;

    glGenTextures(1, &texture.previewTexture);
    glBindTexture(GL_TEXTURE_2D, texture.previewTexture);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, static_cast<GLsizei>(texture.width), static_cast<GLsizei>(texture.height), 0,
                 GL_RGBA, GL_UNSIGNED_BYTE, texture.pixel.data());
    glBindTexture(GL_TEXTURE_2D, 0);
}

EditorTexture::~EditorTexture() {
    if(previewTexture != 0 && glfwGetCurrentContext() != nullptr)
        glDeleteTextures(1, &previewTexture);
}

bool EditorTexture::renderContent() {
    bool updated = false;
    if(ImGui::Button("Update image")) {
        if(const auto path = FileDialog::openFile("Images", "jpg,jpeg,bmp,png,tga,tiff")) {
            auto loaded = loadImageFromFile(path->c_str());
            if(!loaded.data.empty()) {
                width = loaded.width;
                height = loaded.height;
                pixel = std::move(loaded.data);
                uploadTexturePreview(*this);
                updated = true;
            }
        }
    }

    if(!pixel.empty() && ImGui::Button("Vertical Flip")) {
        for(uint32_t top = 0, bottom = height - 1; top < bottom; ++top, --bottom) {
            for(uint32_t x = 0; x < width; ++x)
                std::swap(pixel[static_cast<std::size_t>(top) * width + x], pixel[static_cast<std::size_t>(bottom) * width + x]);
        }
        uploadTexturePreview(*this);
        updated = true;
    }

    if(previewTexture != 0) {
        ImGui::Text("%u x %u", width, height);
        ImGui::Image(static_cast<ImTextureID>(previewTexture), EmToVec2(3, 3), ImVec2{ 0, 1 }, ImVec2{ 1, 0 });
    } else {
        ImGui::TextUnformatted("Unavailable");
    }
    return updated;
}

std::unique_ptr<Node> EditorTexture::toSTTF() const {
    return std::make_unique<Texture>(width, height, pixel);
}

void EditorTexture::fromSTTF(Node& node) {
    const auto& texture = dynamic_cast<Texture&>(node);
    width = texture.width;
    height = texture.height;
    pixel = texture.pixel;
    uploadTexturePreview(*this);
}

bool EditorCubeMap::renderContent() {
    bool updated = false;
    if(ImGui::Button("Update image")) {
        const auto paths = FileDialog::openFiles("Images", "jpg,jpeg,bmp,png,tga,tiff");
        if(!paths.empty()) {
            if(paths.size() != 6) {
                HelloImGui::Log(HelloImGui::LogLevel::Error, "Please choose exactly 6 images for cube map");
                return false;
            }

            std::vector<uint32_t> loadedPixels;
            uint32_t loadedSize = 0;
            for(const auto& path : paths) {
                auto image = loadImageFromFile(path.c_str());
                if(image.data.empty() || image.width != image.height) {
                    HelloImGui::Log(HelloImGui::LogLevel::Error, "Cubemap faces must be square images");
                    return false;
                }
                if(loadedSize == 0)
                    loadedSize = image.width;
                else if(loadedSize != image.width) {
                    HelloImGui::Log(HelloImGui::LogLevel::Error, "Cubemap faces must use the same dimensions");
                    return false;
                }
                loadedPixels.insert(loadedPixels.end(), image.data.begin(), image.data.end());
            }
            size = loadedSize;
            pixel = std::move(loadedPixels);
            updated = true;
        }
    }

    if(pixel.empty())
        ImGui::TextUnformatted("Unavailable");
    else
        ImGui::Text("Loaded (%u x %u, 6 faces)", size, size);
    return updated;
}

std::unique_ptr<Node> EditorCubeMap::toSTTF() const {
    return std::make_unique<CubeMap>(size, pixel);
}

void EditorCubeMap::fromSTTF(Node& node) {
    const auto& texture = dynamic_cast<CubeMap&>(node);
    size = texture.size;
    pixel = texture.pixel;
}

bool EditorVolume::renderContent() {
    if(pixel.empty())
        ImGui::TextUnformatted("Unavailable");
    else
        ImGui::Text("Loaded (%u^3, %u channel%s)", size, channels, channels == 1 ? "" : "s");
    return false;
}

std::unique_ptr<Node> EditorVolume::toSTTF() const {
    return std::make_unique<Volume>(size, channels, pixel);
}

void EditorVolume::fromSTTF(Node& node) {
    const auto& texture = dynamic_cast<Volume&>(node);
    size = texture.size;
    channels = texture.channels;
    pixel = texture.pixel;
}

// See also https://github.com/thedmd/imgui-node-editor/issues/48
bool EditorLastFrame::renderContent() {
    const auto& editor = PipelineEditor::get();
    auto& selectables = editor.mShaderNodes;
    if(std::find(selectables.cbegin(), selectables.cend(), lastFrame) == selectables.cend())
        lastFrame = nullptr;
    if(ImGui::Button(lastFrame ? lastFrame->name.c_str() : "<Select One>")) {
        openPopup = true;
    }
    return false;
}
void EditorLastFrame::renderPopup() {
    const auto& editor = PipelineEditor::get();
    const auto& names = editor.mShaderNodeNames;
    const auto& nodes = editor.mShaderNodes;

    ed::Suspend();
    if(openPopup) {
        ImGui::OpenPopup("##popup_button");
        openPopup = false;
        editing = true;
    }

    if(editing && ImGui::BeginPopup("##popup_button")) {
        lastFrame = nullptr;
        ImGui::BeginChild("##popup_scroller", EmToVec2(4, 4), true, ImGuiWindowFlags_AlwaysVerticalScrollbar);
        for(uint32_t idx = 0; idx < names.size(); ++idx) {
            if(ImGui::Button(names[idx])) {
                lastFrame = nodes[idx];
                editing = false;
                ImGui::CloseCurrentPopup();
            }
        }

        ImGui::EndChild();
        ImGui::EndPopup();
    } else
        editing = false;
    ed::Resume();
}
std::unique_ptr<Node> EditorLastFrame::toSTTF() const {
    return std::make_unique<LastFrame>(lastFrame->name, type, refOutput);
}
void EditorLastFrame::fromSTTF(Node& node) {
    refOutput = dynamic_cast<LastFrame&>(node).refOutput;
    // source pointer is fixed by post processing
}
std::unique_ptr<Node> EditorKeyboard::toSTTF() const {
    return std::make_unique<Keyboard>();
}
void EditorKeyboard::fromSTTF(Node&) {}
std::unique_ptr<Node> EditorMusic::toSTTF() const {
    return std::make_unique<Music>();
}
void EditorMusic::fromSTTF(Node&) {}
void PipelineEditor::loadDocument(ShaderDocument document) {
    std::vector<std::unique_ptr<EditorNode>> oldNodes;
    oldNodes.swap(mNodes);
    std::vector<EditorLink> oldLinks;
    oldLinks.swap(mLinks);
    std::vector<std::pair<std::string, std::string>> oldMetadata;
    oldMetadata.swap(mMetadata);
    const auto oldNextId = mNextId;

    auto rollback = scopeFail([&] {
        oldNodes.swap(mNodes);
        oldLinks.swap(mLinks);
        oldMetadata.swap(mMetadata);
        mNextId = oldNextId;
    });

    for(const auto& [key, value] : document.metadata)
        mMetadata.emplace_back(key, value);

    std::unordered_map<const Node*, EditorNode*> nodeMap;
    for(auto& node : document.nodes) {
        EditorNode* editorNode = nullptr;
        switch(node->getNodeClass()) {
            case NodeClass::RenderOutput:
                editorNode = &spawnRenderOutput();
                break;
            case NodeClass::GLSLShader:
                editorNode = &spawnShader(node->getNodeType());
                break;
            case NodeClass::Texture:
                editorNode = &spawnTexture();
                break;
            case NodeClass::CubeMap:
                editorNode = &spawnCubeMap();
                break;
            case NodeClass::Volume:
                editorNode = &spawnVolume();
                break;
            case NodeClass::LastFrame:
                editorNode = &spawnLastFrame();
                break;
            case NodeClass::Keyboard:
                editorNode = &spawnKeyboard();
                break;
            case NodeClass::Music:
                editorNode = &spawnMusic();
                break;
            case NodeClass::SoundOutput:
            case NodeClass::Unknown:
                throw Error("The editor does not support this document node class");
        }

        editorNode->name = node->name;
        editorNode->fromSTTF(*node);
        nodeMap.emplace(node.get(), editorNode);
    }

    for(auto& node : document.nodes) {
        if(node->getNodeClass() != NodeClass::LastFrame)
            continue;

        auto* editorNode = dynamic_cast<EditorLastFrame*>(nodeMap.at(node.get()));
        auto& lastFrame = dynamic_cast<LastFrame&>(*node);
        if(!lastFrame.refNode || !nodeMap.contains(lastFrame.refNode))
            throw Error("LastFrame '" + node->name + "' references an unknown source");
        editorNode->lastFrame = nodeMap.at(lastFrame.refNode);
        editorNode->type = lastFrame.nodeType;
    }

    for(const auto& [start, end, filter, wrapMode, slot, sourceOutput] : document.links) {
        if(!nodeMap.contains(start) || !nodeMap.contains(end))
            throw Error("Document link references an unknown node");
        auto* startNode = nodeMap.at(start);
        auto* endNode = nodeMap.at(end);
        if(startNode->outputs.empty() || slot >= endNode->inputs.size())
            throw Error("Document link uses an invalid pin");
        mLinks.emplace_back(nextId(), startNode->outputs.front().id, endNode->inputs[slot].id, filter, wrapMode, sourceOutput);
    }

    mShouldResetLayout = true;
    mShouldBuildPipeline = true;
}

void PipelineEditor::loadSTTF(const std::string& path) {
    try {
        HelloImGui::Log(HelloImGui::LogLevel::Info, "Loading STTF from %s", path.c_str());
        ShaderDocument document;
        document.load(path);
        loadDocument(std::move(document));
        HelloImGui::Log(HelloImGui::LogLevel::Info, "Loaded STTF successfully");
    } catch(const std::exception& error) {
        HelloImGui::Log(HelloImGui::LogLevel::Error, "Failed to load STTF %s: %s", path.c_str(), error.what());
    }
}

void PipelineEditor::saveSTTF(const std::string& path) {
    try {
        HelloImGui::Log(HelloImGui::LogLevel::Info, "Writing shader to STTF file %s", path.c_str());
        auto document = makeDocument();
        document.save(path);
        HelloImGui::Log(HelloImGui::LogLevel::Info, "Saved STTF successfully");
    } catch(const std::exception& error) {
        HelloImGui::Log(HelloImGui::LogLevel::Error, "Failed to save STTF %s: %s", path.c_str(), error.what());
    }
}

void PipelineEditor::loadFromShaderToy(const std::string& path) {
    HelloImGui::Log(HelloImGui::LogLevel::Info, "Importing %s", path.c_str());
    auto imported = importFromShaderToy(path);
    if(!imported) {
        HelloImGui::Log(HelloImGui::LogLevel::Error, "Failed to import %s: %s", path.c_str(), imported.error().what());
        return;
    }

    try {
        loadDocument(std::move(*imported));
        HelloImGui::Log(HelloImGui::LogLevel::Info, "Imported ShaderToy shader successfully");
    } catch(const std::exception& error) {
        HelloImGui::Log(HelloImGui::LogLevel::Error, "Failed to load imported shader: %s", error.what());
    }
}

std::string PipelineEditor::getShaderName() const {
    using namespace std::string_view_literals;
    for(auto [k, v] : mMetadata)
        if(k == "Name"sv || k == "name"sv)
            return v;
    return "untitled";
}

void PipelineEditor::updateNodeType() {
    while(true) {
        bool modified = false;
        auto sync = [&](NodeType& x, NodeType y) {
            if(x == y)
                return;
            x = y;
            modified = true;
        };

        // last frame
        for(auto& node : mNodes) {
            if(node->getClass() == NodeClass::LastFrame) {
                auto& lastFrame = dynamic_cast<EditorLastFrame&>(*node);
                if(std::find(mShaderNodes.cbegin(), mShaderNodes.cend(), lastFrame.lastFrame) != mShaderNodes.cend() &&
                   lastFrame.lastFrame->type != lastFrame.type) {
                    sync(lastFrame.type, lastFrame.lastFrame->type);
                }
            }
        }

        std::unordered_map<uintptr_t, ed::PinId> graph;
        for(auto link : mLinks) {
            assert(!graph.count(link.endPinId.Get()));
            graph.emplace(link.endPinId.Get(), link.startPinId);
        }
        for(const auto& node : mNodes) {
            // input pin
            for(auto& input : node->inputs) {
                if(auto iter = graph.find(input.id.Get()); iter != graph.cend()) {
                    sync(input.type, findPin(iter->second)->node->type);
                } else {
                    sync(input.type, NodeType::Image);
                }
            }

            // output pin
            for(auto& output : node->outputs)
                sync(output.type, node->type);
        }

        if(!modified)
            return;
    }
}

SHADERTOY_NAMESPACE_END
