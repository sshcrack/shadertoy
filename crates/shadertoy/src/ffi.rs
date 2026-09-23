use crate::{Error, Filter, InputKind, PassKind, RenderFormat, Result, Wrap};
use shadertoy_sys as sys;
use std::ffi::CStr;

pub(crate) fn last_error() -> Error {
    // SAFETY: st_last_error returns a thread-local NUL-terminated string or null.
    let pointer = unsafe { sys::st_last_error() };
    if pointer.is_null() {
        return Error::Native("native ShaderToy operation failed".into());
    }
    // SAFETY: contract of st_last_error.
    let message = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    Error::Native(message)
}

pub(crate) fn check(code: i32) -> Result<()> {
    if code == 0 { Ok(()) } else { Err(last_error()) }
}

pub(crate) fn pass_kind(kind: PassKind) -> sys::st_pass_kind {
    match kind {
        PassKind::Image => sys::ST_PASS_IMAGE,
        PassKind::Buffer => sys::ST_PASS_BUFFER,
        PassKind::Mesh => sys::ST_PASS_MESH,
        PassKind::Cubemap => sys::ST_PASS_CUBEMAP,
        PassKind::Compute => sys::ST_PASS_COMPUTE,
    }
}

pub(crate) fn render_format(format: RenderFormat) -> sys::st_render_format {
    match format {
        RenderFormat::R32f => sys::ST_RENDER_R32F,
        RenderFormat::Rg32f => sys::ST_RENDER_RG32F,
        RenderFormat::Rgba16f => sys::ST_RENDER_RGBA16F,
        RenderFormat::Rgba32f => sys::ST_RENDER_RGBA32F,
    }
}

pub(crate) fn input_kind(kind: InputKind) -> sys::st_input_kind {
    match kind {
        InputKind::Pass => sys::ST_INPUT_PASS,
        InputKind::Texture => sys::ST_INPUT_TEXTURE,
        InputKind::Cubemap => sys::ST_INPUT_CUBEMAP,
        InputKind::Volume => sys::ST_INPUT_VOLUME,
        InputKind::Keyboard => sys::ST_INPUT_KEYBOARD,
        InputKind::Music => sys::ST_INPUT_MUSIC,
    }
}

pub(crate) fn filter_kind(filter: Filter) -> sys::st_filter {
    match filter {
        Filter::Mipmap => sys::ST_FILTER_MIPMAP,
        Filter::Linear => sys::ST_FILTER_LINEAR,
        Filter::Nearest => sys::ST_FILTER_NEAREST,
    }
}

pub(crate) fn wrap_kind(wrap: Wrap) -> sys::st_wrap {
    match wrap {
        Wrap::Clamp => sys::ST_WRAP_CLAMP,
        Wrap::Repeat => sys::ST_WRAP_REPEAT,
    }
}
