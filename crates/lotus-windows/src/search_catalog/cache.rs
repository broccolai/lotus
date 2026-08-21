use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use std::{io, thread};

use lotus_core::application::ApplicationIdentity;
use lotus_core::dock::DockItem;
use lotus_core::search::{ApplicationEntry, SearchCatalog};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use super::identity::compose_catalog;
use super::sources::discover_start_menu_entries;
use crate::launch::resolve_executable;
use crate::messages::SEARCH_CATALOG_WAKE as SEARCH_CATALOG_WAKE_MESSAGE;

type Discovery = dyn Fn() -> Vec<ApplicationEntry> + Send + Sync + 'static;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshStatus {
    Started,
    Fresh,
    InProgress,
}

pub struct SearchCatalogCache {
    state: Arc<Mutex<CacheState>>,
    discovery: Arc<Discovery>,
    owner_thread: u32,
}

pub struct ReadySearchCatalog {
    pub generation: u64,
    pub catalog: SearchCatalog,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredApplication {
    pub id: String,
    pub name: String,
    pub launch_target: String,
    pub arguments: Option<String>,
    pub icon_source: String,
    pub app_user_model_id: Option<String>,
}

impl RegisteredApplication {
    #[must_use]
    pub fn application_identity(&self) -> ApplicationIdentity {
        let executable = resolve_executable(&self.launch_target);
        ApplicationIdentity::from_path(
            self.app_user_model_id.as_deref(),
            Some(&self.id),
            executable.as_deref(),
            std::iter::empty(),
        )
    }
}

impl Default for SearchCatalogCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
struct CacheState {
    entries: Vec<ApplicationEntry>,
    refreshed_at: Option<Instant>,
    refreshing: bool,
    generation: u64,
}

impl SearchCatalogCache {
    pub fn new() -> Self {
        Self::with_discovery(discover_start_menu_entries)
    }

    pub fn catalog(
        &self,
        dock_items: &[DockItem],
        hidden_executables: &[String],
    ) -> SearchCatalog {
        let entries = lock(&self.state).entries.clone();
        compose_catalog(dock_items, entries, hidden_executables)
    }

    pub fn registered_application(
        &self,
        window: &lotus_core::window::WindowInfo,
        fallback_name: &str,
    ) -> Option<RegisteredApplication> {
        let native_identity =
            crate::application_identity::window_application_identity(window.id);
        if native_identity
            .as_ref()
            .is_some_and(|identity| identity.prevent_pinning)
        {
            return None;
        }
        let app_user_model_id = native_identity
            .as_ref()
            .and_then(|identity| identity.app_user_model_id.as_deref())
            .or(window.app_user_model_id.as_deref());
        let entries = lock(&self.state).entries.clone();
        let window_identity = ApplicationIdentity::from_path(
            app_user_model_id,
            None,
            Some(&window.executable_path),
            std::iter::empty(),
        );
        let entry = entries.iter().find(|entry| {
            application_entry_identity(entry)
                .match_strength(&window_identity)
                .is_match()
        });

        if let Some(entry) = entry {
            return Some(RegisteredApplication {
                id: entry
                    .app_user_model_id
                    .clone()
                    .unwrap_or_else(|| entry.launch_target.clone()),
                name: entry.name.clone(),
                launch_target: entry.launch_target.clone(),
                arguments: None,
                icon_source: entry.icon_source.clone(),
                app_user_model_id: entry.app_user_model_id.clone(),
            });
        }

        let identity = native_identity?;
        let launch = crate::application_identity::relaunch_application(
            identity.relaunch_command.as_deref()?,
        )?;
        let app_user_model_id = identity.app_user_model_id;
        Some(RegisteredApplication {
            id: app_user_model_id
                .clone()
                .unwrap_or_else(|| launch.target.clone()),
            name: identity
                .display_name
                .filter(|name| !name.starts_with('@'))
                .unwrap_or_else(|| fallback_name.to_owned()),
            icon_source: launch.target.clone(),
            launch_target: launch.target,
            arguments: launch.arguments,
            app_user_model_id,
        })
    }

    pub fn ready_catalog(
        &self,
        dock_items: &[DockItem],
        hidden_executables: &[String],
    ) -> Option<ReadySearchCatalog> {
        let state = lock(&self.state);
        (state.generation != 0).then(|| ReadySearchCatalog {
            generation: state.generation,
            catalog: compose_catalog(dock_items, state.entries.clone(), hidden_executables),
        })
    }

    pub fn refresh_if_stale(&self, maximum_age: Duration) -> io::Result<RefreshStatus> {
        {
            let mut state = lock(&self.state);
            if state.refreshing {
                return Ok(RefreshStatus::InProgress);
            }
            if state
                .refreshed_at
                .is_some_and(|updated| updated.elapsed() < maximum_age)
            {
                return Ok(RefreshStatus::Fresh);
            }
            state.refreshing = true;
        }

        let state = Arc::clone(&self.state);
        let discovery = Arc::clone(&self.discovery);
        let owner_thread = self.owner_thread;
        let spawn = thread::Builder::new()
            .name("lotus-start-menu-catalog".into())
            .spawn(move || {
                let completion = RefreshCompletion {
                    state: Arc::clone(&state),
                    owner_thread,
                };
                let entries = discovery();
                {
                    let mut state = lock(&state);
                    state.entries = entries;
                    state.refreshed_at = Some(Instant::now());
                    state.generation = state.generation.saturating_add(1);
                }
                drop(completion);
            });

        if let Err(error) = spawn {
            lock(&self.state).refreshing = false;
            return Err(error);
        }
        Ok(RefreshStatus::Started)
    }

    fn with_discovery(
        discovery: impl Fn() -> Vec<ApplicationEntry> + Send + Sync + 'static,
    ) -> Self {
        let cache = Self {
            state: Arc::new(Mutex::new(CacheState::default())),
            discovery: Arc::new(discovery),
            owner_thread: unsafe { GetCurrentThreadId() },
        };
        let _ = cache.refresh_if_stale(Duration::ZERO);
        cache
    }
}

fn application_entry_identity(entry: &ApplicationEntry) -> ApplicationIdentity {
    let executable = resolve_executable(&entry.launch_target);
    ApplicationIdentity::from_path(
        entry.app_user_model_id.as_deref(),
        Some(&entry.launch_target),
        executable.as_deref(),
        std::iter::empty(),
    )
}

struct RefreshCompletion {
    state: Arc<Mutex<CacheState>>,
    owner_thread: u32,
}

impl Drop for RefreshCompletion {
    fn drop(&mut self) {
        lock(&self.state).refreshing = false;
        let _ = unsafe {
            PostThreadMessageW(
                self.owner_thread,
                SEARCH_CATALOG_WAKE_MESSAGE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
}

pub const fn is_search_catalog_wake(message: u32) -> bool {
    message == SEARCH_CATALOG_WAKE_MESSAGE
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
