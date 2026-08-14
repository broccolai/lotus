use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WindowId(u64);

impl WindowId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowInfo {
    pub id: WindowId,
    pub process_id: u32,
    pub title: String,
    pub executable_path: PathBuf,
    pub app_user_model_id: Option<String>,
}

impl WindowInfo {
    pub fn executable_name(&self) -> Option<&Path> {
        self.executable_path.file_name().map(Path::new)
    }
}
