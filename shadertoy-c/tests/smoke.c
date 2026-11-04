#include <shadertoy/shadertoy.h>

#include <stdint.h>
#include <stdio.h>

int main(void) {
    if(st_context_create_hidden(UINT32_MAX, 1) != NULL) {
        fputs("oversized context dimensions were unexpectedly accepted\n", stderr);
        return 3;
    }

#if defined(__linux__)
    st_context* context = st_context_create_hidden(16, 16);
    if(context == NULL) {
        const char* error = st_last_error();
        fprintf(stderr, "failed to create display-less EGL context: %s\n", error != NULL ? error : "unknown error");
        return 6;
    }
    if(st_context_make_current(context) != 0) {
        const char* error = st_last_error();
        fprintf(stderr, "failed to make display-less EGL context current: %s\n", error != NULL ? error : "unknown error");
        st_context_destroy(context);
        return 7;
    }

    st_project* odd_volume_project = st_project_create("odd-volume");
    if(odd_volume_project == NULL) {
        fputs("failed to create odd-volume project\n", stderr);
        st_context_destroy(context);
        return 10;
    }
    uint8_t odd_volume[27];
    for(size_t index = 0; index < sizeof(odd_volume); ++index)
        odd_volume[index] = (uint8_t)index;
    if(st_project_add_volume_u8(odd_volume_project, "volume", 3, 1, odd_volume, sizeof(odd_volume)) != 0) {
        const char* error = st_last_error();
        fprintf(stderr, "failed to add tightly-packed 3x3x3 R8 volume: %s\n", error != NULL ? error : "unknown error");
        st_project_destroy(odd_volume_project);
        st_context_destroy(context);
        return 11;
    }
    if(st_project_add_pass(odd_volume_project, "image", ST_PASS_IMAGE,
                           "void mainImage(out vec4 color, in vec2 coord) { color = vec4(coord / iResolution.xy, 0.0, 1.0); }") !=
       0) {
        const char* error = st_last_error();
        fprintf(stderr, "failed to add odd-volume smoke pass: %s\n", error != NULL ? error : "unknown error");
        st_project_destroy(odd_volume_project);
        st_context_destroy(context);
        return 12;
    }
    st_runtime* odd_volume_runtime = st_runtime_create();
    if(odd_volume_runtime == NULL || st_runtime_load_project(odd_volume_runtime, odd_volume_project) != 0) {
        const char* error = st_last_error();
        fprintf(stderr, "failed to load tightly-packed 3x3x3 R8 volume: %s\n", error != NULL ? error : "unknown error");
        st_runtime_destroy(odd_volume_runtime);
        st_project_destroy(odd_volume_project);
        st_context_destroy(context);
        return 13;
    }
    st_runtime_destroy(odd_volume_runtime);
    st_project_destroy(odd_volume_project);
    st_context_destroy(context);
#endif

    st_runtime* runtime = st_runtime_create();
    if(runtime == NULL) {
        fputs("failed to create runtime for validation smoke\n", stderr);
        return 4;
    }
    uint8_t rgb[3] = { 0, 0, 0 };
    if(st_runtime_render_rgb(runtime, 0, 1, rgb, 0) == 0) {
        fputs("zero render dimensions were unexpectedly accepted\n", stderr);
        st_runtime_destroy(runtime);
        return 5;
    }
    if(st_runtime_set_audio_input(runtime, NULL) != 0) {
        fputs("resetting audio input to silence was unexpectedly rejected\n", stderr);
        st_runtime_destroy(runtime);
        return 14;
    }
    st_audio_input bad_audio = { 0 };
    bad_audio.available = 1;
    bad_audio.spectrum_len = 4;
    if(st_runtime_set_audio_input(runtime, &bad_audio) == 0) {
        fputs("null audio spectrum with non-zero length was unexpectedly accepted\n", stderr);
        st_runtime_destroy(runtime);
        return 15;
    }
    st_runtime_destroy(runtime);

    st_project* project = st_project_create("c-api-smoke");
    if(project == NULL) {
        const char* error = st_last_error();
        fprintf(stderr, "%s\n", error != NULL ? error : "unknown C API error");
        return 1;
    }

    const uint8_t byte = 0;
    if(st_project_add_texture_rgba8(project, "overflow", UINT32_MAX, UINT32_MAX, &byte, 1) == 0) {
        fputs("overflowing texture dimensions were unexpectedly accepted\n", stderr);
        st_project_destroy(project);
        return 2;
    }

    const uint8_t cubemap[24] = { 0 };
    if(st_project_add_cubemap_rgba8(project, "cube", 1, cubemap, sizeof(cubemap)) != 0) {
        fputs("valid cubemap was unexpectedly rejected\n", stderr);
        st_project_destroy(project);
        return 6;
    }
    if(st_project_add_cubemap_rgba8(project, "bad-cube", 1, cubemap, sizeof(cubemap) - 1) == 0) {
        fputs("invalid cubemap payload was unexpectedly accepted\n", stderr);
        st_project_destroy(project);
        return 7;
    }

    const uint8_t volume[1] = { 42 };
    if(st_project_add_volume_u8(project, "volume", 1, 1, volume, sizeof(volume)) != 0) {
        fputs("valid volume was unexpectedly rejected\n", stderr);
        st_project_destroy(project);
        return 8;
    }
    if(st_project_add_volume_u8(project, "bad-volume", 1, 2, volume, sizeof(volume)) == 0) {
        fputs("invalid volume channel count was unexpectedly accepted\n", stderr);
        st_project_destroy(project);
        return 9;
    }

    st_project_destroy(project);
    puts("shadertoy-c-smoke=OK");
    return 0;
}
