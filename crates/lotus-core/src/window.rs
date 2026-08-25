use std::path::{Path, PathBuf};

pub use crate::application::is_reliable_application_identity;
use crate::application::{ApplicationIdentity, WindowApplicationFacts};

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

/// An ephemeral identity for a window published by the tracker.
///
/// `WindowId` is an HWND value and can be recycled after a window closes. Delayed
/// operations must carry this key to ensure the HWND still has the tracker
/// incarnation that was presented to the user.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TrackedWindowKey {
    pub id: WindowId,
    pub process_id: u32,
    pub incarnation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowInfo {
    pub id: WindowId,
    pub process_id: u32,
    /// Minted by the platform tracker whenever this HWND enters its registry.
    pub incarnation: u64,
    pub title: String,
    pub executable_path: PathBuf,
    pub application_facts: WindowApplicationFacts,
}

impl WindowInfo {
    pub const fn key(&self) -> TrackedWindowKey {
        TrackedWindowKey {
            id: self.id,
            process_id: self.process_id,
            incarnation: self.incarnation,
        }
    }

    pub fn executable_name(&self) -> Option<&Path> {
        self.executable_path.file_name().map(Path::new)
    }

    #[must_use]
    pub fn application_identity(&self) -> ApplicationIdentity {
        ApplicationIdentity::from_path(
            self.application_facts.reliable_id(),
            None,
            Some(&self.executable_path),
            std::iter::empty(),
        )
    }
}
