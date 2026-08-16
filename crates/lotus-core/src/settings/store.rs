use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;
use thiserror::Error;

use super::codec::{SettingsDecodeError, apply_legacy_migrations, decode_settings};
use super::model::DockSettings;

#[derive(Debug, Error)]
pub enum SettingsStoreError {
    #[error("could not {operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not encode Lotus settings: {0}")]
    Encode(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsLoadSource {
    CreatedDefaults,
    Existing,
    Migrated,
    RecoveredInvalid {
        backup_path: PathBuf,
        error: SettingsDecodeError,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsLoad {
    pub settings: DockSettings,
    pub source: SettingsLoadSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsStore {
    directory: PathBuf,
}

impl SettingsStore {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn settings_path(&self) -> PathBuf {
        self.directory.join("settings.json")
    }

    pub fn load(&self) -> Result<SettingsLoad, SettingsStoreError> {
        self.ensure_directory()?;
        let path = self.settings_path();

        if !path.exists() {
            return self.create_defaults();
        }

        let source = fs::read_to_string(&path)
            .map_err(|error| store_io("read settings from", &path, error))?;

        match decode_settings(&source) {
            Ok(settings) => self.finish_valid_load(&source, settings),
            Err(error) => self.recover_invalid(&path, error),
        }
    }

    pub fn save(&self, settings: &DockSettings) -> Result<(), SettingsStoreError> {
        self.ensure_directory()?;
        let path = self.settings_path();
        let settings = settings.clone().normalized();
        let mut json = serde_json::to_string_pretty(&settings)?;
        json.push('\n');

        let mut file = AtomicWriteFile::open(&path)
            .map_err(|error| store_io("open settings for atomic write at", &path, error))?;
        file.write_all(json.as_bytes())
            .map_err(|error| store_io("write settings to", &path, error))?;
        file.commit()
            .map_err(|error| store_io("commit settings at", &path, error))
    }

    fn create_defaults(&self) -> Result<SettingsLoad, SettingsStoreError> {
        let settings = DockSettings::default().normalized();
        self.save(&settings)?;

        Ok(SettingsLoad {
            settings,
            source: SettingsLoadSource::CreatedDefaults,
        })
    }

    fn finish_valid_load(
        &self,
        source: &str,
        mut settings: DockSettings,
    ) -> Result<SettingsLoad, SettingsStoreError> {
        let migrated = apply_legacy_migrations(source, &mut settings);
        if migrated {
            self.save(&settings)?;
        }

        Ok(SettingsLoad {
            settings,
            source: if migrated {
                SettingsLoadSource::Migrated
            } else {
                SettingsLoadSource::Existing
            },
        })
    }

    fn recover_invalid(
        &self,
        settings_path: &Path,
        error: SettingsDecodeError,
    ) -> Result<SettingsLoad, SettingsStoreError> {
        let backup_path = self.invalid_backup_path();
        fs::copy(settings_path, &backup_path).map_err(|error| {
            store_io("back up invalid settings to", &backup_path, error)
        })?;

        Ok(SettingsLoad {
            settings: DockSettings::default().normalized(),
            source: SettingsLoadSource::RecoveredInvalid { backup_path, error },
        })
    }

    fn ensure_directory(&self) -> Result<(), SettingsStoreError> {
        fs::create_dir_all(&self.directory).map_err(|error| {
            store_io("create settings directory at", &self.directory, error)
        })
    }

    fn invalid_backup_path(&self) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
        let base = self
            .directory
            .join(format!("settings.json.invalid-{timestamp}"));

        unique_path(base)
    }
}

fn unique_path(base: PathBuf) -> PathBuf {
    if !base.exists() {
        return base;
    }

    for suffix in 1_u32.. {
        let candidate = base.with_extension(format!(
            "{}-{suffix}",
            base.extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("invalid")
        ));
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("u32 path suffixes cannot be exhausted in practice")
}

fn store_io(operation: &'static str, path: &Path, source: io::Error) -> SettingsStoreError {
    SettingsStoreError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}
