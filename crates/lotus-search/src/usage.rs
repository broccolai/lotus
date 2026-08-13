use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use lotus_core::search::SearchUsage;

pub struct SearchUsageStore {
    path: PathBuf,
}

impl SearchUsageStore {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("search-usage.json"),
        }
    }

    pub fn load(&self) -> io::Result<SearchUsage> {
        match fs::read_to_string(&self.path) {
            Ok(source) => SearchUsage::from_json(&source).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(SearchUsage::default())
            }
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, usage: &SearchUsage) -> io::Result<()> {
        let Some(directory) = self.path.parent() else {
            return Err(io::Error::other(
                "search usage path has no parent directory",
            ));
        };
        fs::create_dir_all(directory)?;
        let mut source = usage.to_json().map_err(io::Error::other)?;
        source.push('\n');

        let mut file = AtomicWriteFile::open(&self.path)?;
        file.write_all(source.as_bytes())?;
        file.commit()
    }
}
