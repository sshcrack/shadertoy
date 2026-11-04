use crate::{Error, Result};

pub const PASS_OUTPUT_DEPTH: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    Image,
    Buffer,
    Mesh,
    Cubemap,
    Compute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MeshCull {
    #[default]
    None,
    Back,
    Front,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderFormat {
    R32f,
    Rg32f,
    Rgba16f,
    #[default]
    Rgba32f,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Pass,
    Texture,
    Cubemap,
    Volume,
    Keyboard,
    Music,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    Mipmap,
    #[default]
    Linear,
    Nearest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wrap {
    Clamp,
    #[default]
    Repeat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassTiming {
    pub name: String,
    pub gpu_nanoseconds: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassProfileSample {
    pub name: String,
    pub gpu_execution_nanoseconds: u64,
    pub attributed_nanoseconds: u64,
    pub completion_wait_nanoseconds: u64,
    pub width: u32,
    pub height: u32,
    pub sample_valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        debug_assert_eq!(Some(pixels.len()), checked_image_len(width, height, 3).ok());
        Self {
            width,
            height,
            pixels,
        }
    }
}

pub(crate) fn checked_image_len(width: u32, height: u32, channels: usize) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidImageDimensions { width, height });
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or(Error::ImageSizeOverflow {
            width,
            height,
            channels,
        })
}

pub(crate) fn zeroed_image_vec<T: Default>(
    width: u32,
    height: u32,
    channels: usize,
) -> Result<Vec<T>> {
    let len = checked_image_len(width, height, channels)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(len)
        .map_err(|_| Error::ImageAllocationFailed {
            width,
            height,
            channels,
        })?;
    values.resize_with(len, T::default);
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_image_len_rejects_invalid_dimensions_and_overflow() {
        assert!(matches!(
            checked_image_len(0, 1, 4),
            Err(Error::InvalidImageDimensions { .. })
        ));
        assert!(checked_image_len(2, 1, usize::MAX).is_err());
    }
}
