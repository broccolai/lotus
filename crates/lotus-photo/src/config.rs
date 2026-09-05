use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

const MIN_DPI: u32 = 96;
const MAX_DPI: u32 = 384;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhotoScene {
    pub kind: SceneKind,
    #[serde(default = "default_dpi")]
    pub dpi: u32,
    pub apps: Vec<AppConfig>,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub selected: Option<usize>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneKind {
    Dock,
    Search,
    Switcher,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub name: String,
    pub path: PathBuf,
}

impl PhotoScene {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let source = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        let mut scene: Self =
            serde_json::from_str(&source).map_err(|source| ConfigError::Parse {
                path: path.to_owned(),
                source,
            })?;
        if !(MIN_DPI..=MAX_DPI).contains(&scene.dpi) {
            return Err(ConfigError::DpiOutOfRange { dpi: scene.dpi });
        }
        if scene.apps.is_empty() {
            return Err(ConfigError::EmptyApps);
        }
        if let Some(selected) = scene.selected
            && selected >= scene.apps.len()
        {
            return Err(ConfigError::SelectedOutOfRange {
                selected,
                len: scene.apps.len(),
            });
        }
        if matches!(scene.kind, SceneKind::Switcher) && scene.selected.is_none() {
            scene.selected = Some(0);
        }
        let directory = path.parent().unwrap_or_else(|| Path::new("."));
        for app in &mut scene.apps {
            if app.path.is_relative() {
                app.path = directory.join(&app.path);
            }
        }
        Ok(scene)
    }
}

const fn default_dpi() -> u32 {
    192
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read scene file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse scene file {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("DPI {dpi} is outside the supported {MIN_DPI}..={MAX_DPI} range")]
    DpiOutOfRange { dpi: u32 },
    #[error("scene must contain at least one app")]
    EmptyApps,
    #[error("selected index {selected} is outside the {len} configured apps")]
    SelectedOutOfRange { selected: usize, len: usize },
}
