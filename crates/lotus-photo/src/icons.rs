use std::path::Path;

use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::icon::Icon;
use lotus_windows::native_icon::NativeIconCache;
use thiserror::Error;

pub fn load(
    cache: &mut NativeIconCache,
    name: &str,
    path: &Path,
    size: u32,
) -> Result<Icon<EmbeddedIcon>, IconError> {
    cache
        .icon(path, size)
        .map_err(|source| IconError::Extract {
            name: name.to_owned(),
            path: path.to_owned(),
            source,
        })?
        .map(Icon::Raster)
        .ok_or_else(|| IconError::Missing {
            name: name.to_owned(),
            path: path.to_owned(),
        })
}

#[derive(Debug, Error)]
pub enum IconError {
    #[error("could not load an icon for app {name:?} from {path}: {source}")]
    Extract {
        name: String,
        path: std::path::PathBuf,
        source: lotus_windows::native_icon::NativeIconError,
    },
    #[error("no icon was found for app {name:?} at {path}")]
    Missing {
        name: String,
        path: std::path::PathBuf,
    },
}
