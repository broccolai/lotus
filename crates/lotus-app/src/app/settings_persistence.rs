use std::path::Path;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

use lotus_core::settings::{DockSettings, SettingsReset, SettingsStore};
use lotus_windows::interaction::UiThreadWake;

use crate::app::AppError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SettingsSaveReason {
    Apply { onboarding: bool },
    Reorder,
    Pin,
}

#[derive(Clone, Debug)]
pub(super) enum PersistenceOperation {
    Save {
        reason: SettingsSaveReason,
        settings: Box<DockSettings>,
    },
    Reset,
}

#[derive(Debug)]
pub(super) enum PersistenceOutcome {
    Saved,
    Reset(Box<SettingsReset>),
    Failed(String),
}

#[derive(Debug)]
pub(super) struct PersistenceCompletion {
    pub(super) operation_id: u64,
    pub(super) operation: PersistenceOperation,
    pub(super) outcome: PersistenceOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PersistenceRequestError {
    Busy,
    Unavailable,
}

impl PersistenceRequestError {
    pub(super) const fn message(self) -> &'static str {
        match self {
            Self::Busy => {
                "Lotus is still saving another settings change. Try again in a moment."
            }
            Self::Unavailable => {
                "Lotus's settings saver is unavailable. Restart Lotus before trying again."
            }
        }
    }
}

enum WorkerCommand {
    Save {
        id: u64,
        settings: Box<DockSettings>,
    },
    Reset {
        id: u64,
    },
    Shutdown {
        acknowledgement: Sender<()>,
    },
}

struct WorkerCompletion {
    id: u64,
    outcome: PersistenceOutcome,
}

struct PendingOperation {
    id: u64,
    operation: PersistenceOperation,
}

#[derive(Default)]
struct Coordinator {
    next_id: u64,
    pending: Option<PendingOperation>,
}

/// Serializes Lotus's durable settings writes without making the UI wait for disk.
pub(super) struct SettingsPersistence {
    directory: std::path::PathBuf,
    commands: Sender<WorkerCommand>,
    completions: Mutex<Receiver<WorkerCompletion>>,
    coordinator: Mutex<Coordinator>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl SettingsPersistence {
    pub(super) fn new(store: SettingsStore) -> Result<Self, AppError> {
        let directory = store.directory().to_owned();
        let wake = UiThreadWake::settings_persistence();
        let (commands, command_receiver) = mpsc::channel();
        let (completion_sender, completions) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("lotus-settings-persistence".into())
            .spawn(move || {
                run_worker(&store, &command_receiver, &completion_sender, wake);
            })
            .map_err(AppError::SettingsPersistenceWorker)?;

        Ok(Self {
            directory,
            commands,
            completions: Mutex::new(completions),
            coordinator: Mutex::new(Coordinator::default()),
            worker: Mutex::new(Some(worker)),
        })
    }

    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(super) fn request_save(
        &self,
        reason: SettingsSaveReason,
        settings: DockSettings,
    ) -> Result<(), PersistenceRequestError> {
        self.request(PersistenceOperation::Save {
            reason,
            settings: Box::new(settings),
        })
    }

    pub(super) fn request_reset(&self) -> Result<(), PersistenceRequestError> {
        self.request(PersistenceOperation::Reset)
    }

    pub(super) fn drain_completion(&self) -> Option<PersistenceCompletion> {
        let completion = match self
            .completions
            .lock()
            .expect("settings persistence completions poisoned")
            .try_recv()
        {
            Ok(completion) => completion,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
        };
        let mut coordinator = self
            .coordinator
            .lock()
            .expect("settings persistence coordinator poisoned");
        let pending = coordinator.pending.take()?;
        if pending.id != completion.id {
            lotus_windows::diagnostics::record_message(
                "settings.persistence_completion_mismatch",
                "discarded an out-of-order settings persistence completion",
            );
            return None;
        }
        Some(PersistenceCompletion {
            operation_id: completion.id,
            operation: pending.operation,
            outcome: completion.outcome,
        })
    }

    pub(super) fn export(
        &self,
        settings: &DockSettings,
        destination: &Path,
    ) -> Result<(), AppError> {
        SettingsStore::new(&self.directory).export(settings, destination)?;
        Ok(())
    }

    pub(super) fn validate_export_destination(
        &self,
        destination: &Path,
    ) -> Result<(), AppError> {
        SettingsStore::new(&self.directory).validate_export_destination(destination)?;
        Ok(())
    }

    fn request(
        &self,
        operation: PersistenceOperation,
    ) -> Result<(), PersistenceRequestError> {
        let mut coordinator = self
            .coordinator
            .lock()
            .expect("settings persistence coordinator poisoned");
        if coordinator.pending.is_some() {
            lotus_windows::diagnostics::record_message(
                "settings.persistence_busy",
                "rejected an overlapping durable settings mutation while another write was in flight",
            );
            return Err(PersistenceRequestError::Busy);
        }
        coordinator.next_id = coordinator.next_id.wrapping_add(1);
        let id = coordinator.next_id;
        let command = match &operation {
            PersistenceOperation::Save { settings, .. } => WorkerCommand::Save {
                id,
                settings: settings.clone(),
            },
            PersistenceOperation::Reset => WorkerCommand::Reset { id },
        };
        if self.commands.send(command).is_err() {
            lotus_windows::diagnostics::record_message(
                "settings.persistence_worker_unavailable",
                "settings persistence worker was unavailable before a write could start",
            );
            return Err(PersistenceRequestError::Unavailable);
        }
        coordinator.pending = Some(PendingOperation { id, operation });
        Ok(())
    }
}

impl Drop for SettingsPersistence {
    fn drop(&mut self) {
        const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

        let pending = self
            .coordinator
            .get_mut()
            .expect("settings persistence coordinator poisoned")
            .pending
            .is_some();
        if pending {
            lotus_windows::diagnostics::record_message(
                "settings.persistence_shutdown_pending",
                "waiting briefly for an accepted settings write before Lotus exits",
            );
        }

        let (acknowledgement, acknowledged) = mpsc::channel();
        if self
            .commands
            .send(WorkerCommand::Shutdown { acknowledgement })
            .is_err()
        {
            lotus_windows::diagnostics::record_message(
                "settings.persistence_shutdown_worker_unavailable",
                "settings persistence worker was unavailable during shutdown",
            );
            join_worker(
                self.worker
                    .get_mut()
                    .expect("settings persistence worker handle poisoned"),
            );
            return;
        }
        if acknowledged.recv_timeout(SHUTDOWN_WAIT).is_err() {
            lotus_windows::diagnostics::record_message(
                "settings.persistence_shutdown_timeout",
                "Lotus exited before the settings persistence worker confirmed shutdown",
            );
            return;
        }
        join_worker(
            self.worker
                .get_mut()
                .expect("settings persistence worker handle poisoned"),
        );
    }
}

fn join_worker(worker: &mut Option<JoinHandle<()>>) {
    if let Some(worker) = worker.take()
        && worker.join().is_err()
    {
        lotus_windows::diagnostics::record_message(
            "settings.persistence_shutdown_panicked",
            "settings persistence worker panicked during shutdown",
        );
    }
}

fn run_worker(
    store: &SettingsStore,
    commands: &Receiver<WorkerCommand>,
    completions: &Sender<WorkerCompletion>,
    wake: UiThreadWake,
) {
    while let Ok(command) = commands.recv() {
        if let WorkerCommand::Shutdown { acknowledgement } = command {
            let _sent = acknowledgement.send(());
            return;
        }
        let (id, outcome) = match command {
            WorkerCommand::Save { id, settings } => (
                id,
                store.save(&settings).map_or_else(
                    |error| PersistenceOutcome::Failed(error.to_string()),
                    |()| PersistenceOutcome::Saved,
                ),
            ),
            WorkerCommand::Reset { id } => (
                id,
                store.reset().map_or_else(
                    |error| PersistenceOutcome::Failed(error.to_string()),
                    |reset| PersistenceOutcome::Reset(Box::new(reset)),
                ),
            ),
            WorkerCommand::Shutdown { .. } => {
                unreachable!("shutdown handled before persistence")
            }
        };
        if completions.send(WorkerCompletion { id, outcome }).is_err() {
            return;
        }
        if !wake.wake() {
            lotus_windows::diagnostics::record_message(
                "settings.persistence_wake_failed",
                "settings persistence completed without a UI wake",
            );
        }
    }
}
