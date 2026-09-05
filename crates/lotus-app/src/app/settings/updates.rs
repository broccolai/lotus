use lotus_core::settings::UpdateChannel;
use lotus_windows::update::{Release, UpdateChecker, UpdateResult, UpdateStartError};

pub(super) struct SettingsUpdates {
    checker: UpdateChecker,
    pending: Option<Release>,
}

impl SettingsUpdates {
    pub(super) fn new(updates_allowed: bool) -> Self {
        Self {
            checker: UpdateChecker::new(updates_allowed),
            pending: None,
        }
    }

    pub(super) fn offer(&mut self, release: Release) {
        self.pending = Some(release);
    }

    pub(super) fn take_offer(&mut self) -> Option<Release> {
        self.pending.take()
    }

    pub(super) fn start_check(
        &mut self,
        channel: UpdateChannel,
    ) -> Result<bool, UpdateStartError> {
        self.checker.start_check(env!("CARGO_PKG_VERSION"), channel)
    }

    pub(super) fn start_download(
        &mut self,
        release: Release,
    ) -> Result<bool, UpdateStartError> {
        self.checker.start_download(release)
    }

    pub(super) fn drain_results(&self) -> Vec<UpdateResult> {
        self.checker.drain().collect()
    }
}
