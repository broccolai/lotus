use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;
use thiserror::Error;

use super::codec::{
    SettingsDecodeError, apply_settings_migrations, decode_settings_document,
};
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
    #[error("the export destination `{path}` is the active Lotus settings file")]
    ExportAliasesLiveSettings { path: PathBuf },
    #[error("could not resolve settings path `{path}`")]
    InvalidPath { path: PathBuf },
    #[error("the system clock could not create a reset backup timestamp: {0}")]
    Clock(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsLoadSource {
    CreatedDefaults,
    Existing,
    Migrated {
        backup_path: PathBuf,
        from_version: u32,
        to_version: u32,
    },
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

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsReset {
    pub settings: DockSettings,
    pub backup_path: PathBuf,
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

        let source_bytes = fs::read(&path)
            .map_err(|error| store_io("read settings from", &path, error))?;
        let source =
            String::from_utf8(source_bytes.clone()).map_err(SettingsDecodeError::from);

        match source.and_then(|source| decode_settings_document(&source)) {
            Ok(decoded) => self.finish_valid_load(&source_bytes, decoded),
            Err(error) => self.recover_invalid(&source_bytes, error),
        }
    }

    pub fn save(&self, settings: &DockSettings) -> Result<(), SettingsStoreError> {
        self.ensure_directory()?;
        let path = self.settings_path();
        Self::write_settings(&path, settings, "settings")
    }

    pub fn export(
        &self,
        settings: &DockSettings,
        destination: &Path,
    ) -> Result<(), SettingsStoreError> {
        self.validate_export_destination(destination)?;

        Self::write_settings(destination, settings, "exported settings")
    }

    pub fn validate_export_destination(
        &self,
        destination: &Path,
    ) -> Result<(), SettingsStoreError> {
        if paths_alias(destination, &self.settings_path())? {
            return Err(SettingsStoreError::ExportAliasesLiveSettings {
                path: destination.to_owned(),
            });
        }
        Ok(())
    }

    pub fn reset(&self) -> Result<SettingsReset, SettingsStoreError> {
        self.ensure_directory()?;
        let settings_path = self.settings_path();
        let source = fs::read(&settings_path)
            .map_err(|error| store_io("read settings from", &settings_path, error))?;
        let backup_path = self.reset_backup_path()?;
        write_atomic_bytes(&backup_path, &source, "reset backup")?;

        let settings = DockSettings::default().normalized();
        Self::write_settings(&settings_path, &settings, "reset settings")?;

        Ok(SettingsReset {
            settings,
            backup_path,
        })
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
        source: &[u8],
        mut decoded: super::codec::DecodedSettings,
    ) -> Result<SettingsLoad, SettingsStoreError> {
        let from_version = decoded.source_schema_version;
        let migrated = apply_settings_migrations(&mut decoded);
        if migrated {
            let to_version = decoded.settings.schema_version;
            let backup_path = self.pre_migration_backup_path(from_version, to_version);
            write_atomic_bytes(&backup_path, source, "pre-migration backup")?;
            self.save(&decoded.settings)?;

            return Ok(SettingsLoad {
                settings: decoded.settings,
                source: SettingsLoadSource::Migrated {
                    backup_path,
                    from_version,
                    to_version,
                },
            });
        }

        Ok(SettingsLoad {
            settings: decoded.settings,
            source: SettingsLoadSource::Existing,
        })
    }

    fn recover_invalid(
        &self,
        source: &[u8],
        error: SettingsDecodeError,
    ) -> Result<SettingsLoad, SettingsStoreError> {
        let backup_path = self.unique_backup_path("invalid")?;
        write_atomic_bytes(&backup_path, source, "back up invalid settings to")?;

        let settings = DockSettings::default().normalized();
        self.save(&settings)?;

        Ok(SettingsLoad {
            settings,
            source: SettingsLoadSource::RecoveredInvalid { backup_path, error },
        })
    }

    fn ensure_directory(&self) -> Result<(), SettingsStoreError> {
        fs::create_dir_all(&self.directory).map_err(|error| {
            store_io("create settings directory at", &self.directory, error)
        })
    }

    fn pre_migration_backup_path(&self, from_version: u32, to_version: u32) -> PathBuf {
        self.directory.join(format!(
            "settings.json.pre-migration-v{from_version}-to-v{to_version}.bak"
        ))
    }

    fn write_settings(
        path: &Path,
        settings: &DockSettings,
        description: &'static str,
    ) -> Result<(), SettingsStoreError> {
        let settings = settings.clone().normalized();
        let json = encode_settings(&settings)?;
        write_atomic_bytes(path, json.as_bytes(), description)
    }

    fn reset_backup_path(&self) -> Result<PathBuf, SettingsStoreError> {
        self.unique_backup_path("reset")
    }

    fn unique_backup_path(&self, kind: &str) -> Result<PathBuf, SettingsStoreError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| SettingsStoreError::Clock(error.to_string()))?;
        let stem = format!(
            "settings.json.{kind}-{}-{:09}.bak",
            timestamp.as_secs(),
            timestamp.subsec_nanos()
        );
        let mut candidate = self.directory.join(&stem);
        let mut suffix = 1_u32;
        while candidate.exists() {
            candidate = self.directory.join(format!("{stem}.{suffix}"));
            suffix = suffix.saturating_add(1);
        }
        Ok(candidate)
    }
}

fn encode_settings(settings: &DockSettings) -> Result<String, SettingsStoreError> {
    let mut json = serde_json::to_string_pretty(settings)?;
    json.push('\n');
    Ok(json)
}

fn paths_alias(left: &Path, right: &Path) -> Result<bool, SettingsStoreError> {
    let left = canonical_path(left)?;
    let right = canonical_path(right)?;
    Ok(left
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy()))
}

fn canonical_path(path: &Path) -> Result<PathBuf, SettingsStoreError> {
    let absolute = std::path::absolute(path)
        .map_err(|error| store_io("resolve settings path", path, error))?;
    if absolute.exists() {
        return absolute
            .canonicalize()
            .map_err(|error| store_io("resolve settings path", &absolute, error));
    }

    let parent = absolute
        .parent()
        .ok_or_else(|| SettingsStoreError::InvalidPath {
            path: absolute.clone(),
        })?;
    let parent = parent
        .canonicalize()
        .map_err(|error| store_io("resolve settings directory", parent, error))?;
    let file_name =
        absolute
            .file_name()
            .ok_or_else(|| SettingsStoreError::InvalidPath {
                path: absolute.clone(),
            })?;
    Ok(parent.join(file_name))
}

fn write_atomic_bytes(
    path: &Path,
    bytes: &[u8],
    description: &'static str,
) -> Result<(), SettingsStoreError> {
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| store_io("open atomic file at", path, error))?;
    file.write_all(bytes)
        .map_err(|error| store_io(description, path, error))?;
    file.commit()
        .map_err(|error| store_io("commit atomic file at", path, error))
}

fn store_io(operation: &'static str, path: &Path, source: io::Error) -> SettingsStoreError {
    SettingsStoreError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}
