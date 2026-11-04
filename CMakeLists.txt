cmake_minimum_required(VERSION 3.20)

option(SHADERTOY_BUILD_GUI "Build the interactive ShaderToy editor" ON)
option(SHADERTOY_BUILD_PREVIEW_TOOL "Build the deterministic hidden-window shader preview CLI" OFF)
option(SHADERTOY_BUILD_C_API "Build the stable shared C ABI used by external language bindings" OFF)
option(SHADERTOY_BUILD_C_API_STATIC "Build the static C ABI used by the Rust CLI" OFF)

if(SHADERTOY_BUILD_GUI)
    list(APPEND VCPKG_MANIFEST_FEATURES gui)
endif()
if(SHADERTOY_BUILD_PREVIEW_TOOL)
    list(APPEND VCPKG_MANIFEST_FEATURES preview)
endif()
if(SHADERTOY_BUILD_C_API OR SHADERTOY_BUILD_C_API_STATIC)
    list(APPEND VCPKG_MANIFEST_FEATURES c-api)
endif()

project(shadertoy VERSION 2.6.1 HOMEPAGE_URL "https://github.com/sshcrack/shadertoy" LANGUAGES CXX)

include(GNUInstallDirs)
include(CTest)

set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_subdirectory(shadertoy)

if(BUILD_TESTING)
    add_executable(shadertoy-library-smoke tests/library_smoke.cpp)
    target_compile_features(shadertoy-library-smoke PRIVATE cxx_std_23)
    target_link_libraries(shadertoy-library-smoke PRIVATE shadertoy::shadertoy)
    add_test(NAME shadertoy-library-smoke COMMAND shadertoy-library-smoke)
endif()

if(SHADERTOY_BUILD_GUI)
    add_subdirectory(app)
endif()

if(SHADERTOY_BUILD_C_API OR SHADERTOY_BUILD_C_API_STATIC)
    add_subdirectory(shadertoy-c)
endif()

install(FILES README.md LICENSE DESTINATION .)
install(DIRECTORY licenses examples DESTINATION .)

if(SHADERTOY_BUILD_PREVIEW_TOOL)
    find_package(glfw3 CONFIG REQUIRED)
    find_package(Stb REQUIRED)

    add_executable(shadertoy-preview tools/shadertoy-preview.cpp)
    target_compile_features(shadertoy-preview PRIVATE cxx_std_23)
    target_include_directories(shadertoy-preview PRIVATE ${Stb_INCLUDE_DIR})
    target_link_libraries(shadertoy-preview PRIVATE shadertoy::shadertoy glfw)
endif()
