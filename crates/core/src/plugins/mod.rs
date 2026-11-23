//! 插件：清单解析（[`manifest`]）、目录发现/搜索（[`registry`]）与导入（[`import`]）。

pub mod import;
pub mod manifest;
pub mod output;
pub mod registry;

pub use import::{import_path, ImportBatch, ImportFailure, ImportReport};
pub use manifest::{
    check_host_requirement, has_manifest, load_descriptor, CmdType, PluginCmd, PluginDescriptor,
    PluginManifest, PlatformSpec, HOST_VERSION, NATIVE_MANIFEST,
};
pub use output::hits_from_rows;
pub use registry::{extract_feature, PluginRegistry};
