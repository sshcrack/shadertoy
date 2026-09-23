use super::*;
use crate::manifest::{Filter, InputKind, PassKind, Wrap};
use crate::scaffold::Template;

impl From<TemplateArg> for Template {
    fn from(value: TemplateArg) -> Self {
        match value {
            TemplateArg::Minimal => Template::Minimal,
            TemplateArg::Multipass => Template::Multipass,
        }
    }
}

impl From<PassKindArg> for PassKind {
    fn from(value: PassKindArg) -> Self {
        match value {
            PassKindArg::Buffer => PassKind::Buffer,
            PassKindArg::Mesh => PassKind::Mesh,
            PassKindArg::Cubemap => PassKind::Cubemap,
            PassKindArg::Compute => PassKind::Compute,
            PassKindArg::Sound => PassKind::Sound,
        }
    }
}

impl From<InputKindArg> for InputKind {
    fn from(value: InputKindArg) -> Self {
        match value {
            InputKindArg::Pass => InputKind::Pass,
            InputKindArg::Texture => InputKind::Texture,
            InputKindArg::Keyboard => InputKind::Keyboard,
            InputKindArg::Music => InputKind::Music,
            InputKindArg::Video => InputKind::Video,
            InputKindArg::Webcam => InputKind::Webcam,
        }
    }
}

impl From<FilterArg> for Filter {
    fn from(value: FilterArg) -> Self {
        match value {
            FilterArg::Mipmap => Filter::Mipmap,
            FilterArg::Linear => Filter::Linear,
            FilterArg::Nearest => Filter::Nearest,
        }
    }
}

impl From<WrapArg> for Wrap {
    fn from(value: WrapArg) -> Self {
        match value {
            WrapArg::Clamp => Wrap::Clamp,
            WrapArg::Repeat => Wrap::Repeat,
        }
    }
}
