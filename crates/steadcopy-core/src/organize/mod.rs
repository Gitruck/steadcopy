//! 组织规则：路径模板、目录模板、类型过滤、sidecar 配对、落地冲突策略。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/organize-rules/spec.md`

pub mod filter;
pub mod sidecar;
pub mod path_template;

pub use filter::{
    file_ext, normalize_ext, CategoryRule, Classification, FilterConfig, MediaKind,
};
pub use sidecar::{SidecarMatcher, StemRule};
pub use path_template::{
    sanitize_segment, sanitize_value, PathTemplate, Placeholder, RenderContext, TemplateError,
};
