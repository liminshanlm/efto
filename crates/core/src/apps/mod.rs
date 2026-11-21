//! 应用发现与启动。

pub mod indexer;
pub mod launch;
pub mod lnk;
pub mod model;
pub mod registry;
pub mod scanner;
pub mod startmenu;

pub use indexer::{AppIndex, Indexer, ScanReport};
pub use launch::{launch, shell_open, split_args};
pub use model::{AppEntry, AppSource, LaunchTarget, TargetKind};
