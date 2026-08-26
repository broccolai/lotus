use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use std::{io, thread};

use lotus_core::application::{
    ApplicationKey, LaunchSpec, RegisteredApplication, application_provider_keys,
    is_reliable_registered_id, is_shared_host_executable, normalized_executable_name,
    normalized_path,
};
use lotus_core::dock::DockItem;
use lotus_core::search::{ApplicationEntry, SearchCatalog};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use super::identity::compose_catalog;
use super::resolver::ApplicationCatalogSnapshot;
use super::sources::discover_start_menu_entries;
use crate::launch::{command_line_arguments, resolve_executable, shortcut_arguments};
use crate::messages::SEARCH_CATALOG_WAKE as SEARCH_CATALOG_WAKE_MESSAGE;
use crate::responsiveness::METRICS;

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

#[derive(Debug)]
struct CacheState {
    snapshot: Arc<ApplicationCatalogSnapshot>,
    refreshed_at: Option<Instant>,
    refreshing: bool,
    generation: u64,
}

impl Default for CacheState {
    fn default() -> Self {
        Self {
            snapshot: Arc::new(ApplicationCatalogSnapshot::new(0, Vec::new())),
            refreshed_at: None,
            refreshing: false,
            generation: 0,
        }
    }
}

impl Default for SearchCatalogCache {
    fn default() -> Self {
        Self::new()
    }
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
        let snapshot = self.snapshot();
        compose_catalog(
            dock_items,
            &snapshot.applications,
            &snapshot.search_entries,
            hidden_executables,
        )
    }

    pub fn snapshot(&self) -> Arc<ApplicationCatalogSnapshot> {
        lock(&self.state).snapshot.clone()
    }

    pub fn ready_catalog(
        &self,
        dock_items: &[DockItem],
        hidden_executables: &[String],
    ) -> Option<ReadySearchCatalog> {
        let (generation, snapshot) = {
            let state = lock(&self.state);
            (state.generation, state.snapshot.clone())
        };
        (generation != 0).then(|| ReadySearchCatalog {
            generation,
            catalog: compose_catalog(
                dock_items,
                &snapshot.applications,
                &snapshot.search_entries,
                hidden_executables,
            ),
        })
    }

    pub fn ready_generation(&self) -> Option<u64> {
        let generation = lock(&self.state).generation;
        (generation != 0).then_some(generation)
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
                let build_started = Instant::now();
                let catalog = build_registered_catalog(discovery());
                let entry_count = catalog.applications.len();
                let generation = {
                    let mut state = lock(&state);
                    state.refreshed_at = Some(Instant::now());
                    state.generation = state.generation.saturating_add(1);
                    state.snapshot =
                        Arc::new(ApplicationCatalogSnapshot::with_search_entries(
                            state.generation,
                            catalog.applications,
                            catalog.search_entries,
                        ));
                    state.generation
                };
                METRICS.record_application_catalog(
                    generation,
                    entry_count,
                    catalog.duplicate_merges,
                    catalog.ambiguous_aliases,
                    build_started.elapsed(),
                );
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

struct CatalogBuild {
    applications: Vec<RegisteredApplication>,
    search_entries: Vec<ApplicationEntry>,
    duplicate_merges: usize,
    ambiguous_aliases: usize,
}

struct CatalogCandidate {
    application: RegisteredApplication,
    search_entry: ApplicationEntry,
    preference: u8,
}

fn build_registered_catalog(entries: Vec<ApplicationEntry>) -> CatalogBuild {
    let candidates = entries
        .into_iter()
        .filter_map(materialize_registered_application)
        .collect::<Vec<_>>();
    let mut parents = (0..candidates.len()).collect::<Vec<_>>();
    let mut component_ids = candidates
        .iter()
        .map(|candidate| {
            candidate
                .application
                .app_user_model_id
                .as_deref()
                .map(str::to_lowercase)
        })
        .collect::<Vec<_>>();
    let mut owners = HashMap::<String, Vec<usize>>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        for key in strong_merge_keys(&candidate.application) {
            if let Some(existing) = owners.get(&key) {
                for &existing in existing {
                    union_if_compatible(&mut parents, &mut component_ids, existing, index);
                }
            }
            owners.entry(key).or_default().push(index);
        }
    }
    let mut groups = BTreeMap::<usize, Vec<usize>>::new();
    for index in 0..candidates.len() {
        let root = find(&mut parents, index);
        groups.entry(root).or_default().push(index);
    }
    let duplicate_merges = candidates.len().saturating_sub(groups.len());
    let mut applications = Vec::with_capacity(groups.len());
    let mut search_entries = Vec::with_capacity(groups.len());
    for indices in groups.into_values() {
        let preferred = indices
            .iter()
            .copied()
            .min_by_key(|&index| (candidates[index].preference, index))
            .expect("catalogue group is not empty");
        let mut application = candidates[preferred].application.clone();
        let mut search_entry = candidates[preferred].search_entry.clone();
        for &index in &indices {
            if index == preferred {
                continue;
            }
            merge_registered_application(&mut application, &candidates[index].application);
        }
        if search_entry.app_user_model_id.is_none() {
            search_entry
                .app_user_model_id
                .clone_from(&application.app_user_model_id);
        }
        applications.push(application);
        search_entries.push(search_entry);
    }
    let ambiguous_aliases = ambiguous_alias_count(&applications);
    CatalogBuild {
        applications,
        search_entries,
        duplicate_merges,
        ambiguous_aliases,
    }
}

fn materialize_registered_application(entry: ApplicationEntry) -> Option<CatalogCandidate> {
    let embedded_arguments = shortcut_arguments(std::path::Path::new(&entry.launch_target));
    let arguments = embedded_arguments
        .clone()
        .or_else(|| entry.arguments.clone());
    let resolved_target = resolve_executable(&entry.launch_target);
    let preference = catalog_preference(arguments.as_deref(), resolved_target.as_deref());
    let launch = LaunchSpec::new(&entry.launch_target, None)?;
    let canonical_launch = resolved_target
        .as_ref()
        .and_then(|target| LaunchSpec::new(target.to_string_lossy(), arguments.as_deref()));
    let executable = resolved_target
        .as_deref()
        .and_then(|target| normalized_path(&target.to_string_lossy()));
    let mut aliases = executable
        .as_deref()
        .and_then(normalized_executable_name)
        .into_iter()
        .collect::<Vec<_>>();
    let parsed_arguments = arguments
        .as_deref()
        .map(command_line_arguments)
        .unwrap_or_default();
    let provider_keys = application_provider_keys(
        entry.app_user_model_id.as_deref(),
        executable.as_deref(),
        &parsed_arguments,
    );
    let is_host_app = provider_keys.iter().any(|key| key.starts_with("chromium:"))
        || executable
            .as_deref()
            .and_then(normalized_executable_name)
            .as_deref()
            .is_some_and(is_host_application);
    if !parsed_arguments.is_empty() {
        let mut values = parsed_arguments.iter().map(String::as_str);
        while let Some(argument) = values.next() {
            if argument.eq_ignore_ascii_case("--processStart")
                && let Some(value) = values.next().and_then(normalized_executable_name)
            {
                aliases.push(value);
            }
        }
    }
    aliases.sort();
    aliases.dedup();
    let canonical_identity = canonical_launch.as_ref().unwrap_or(&launch);
    let id = entry
        .app_user_model_id
        .clone()
        .unwrap_or_else(|| canonical_identity.signature());
    let key = entry
        .app_user_model_id
        .as_deref()
        .filter(|id| is_reliable_registered_id(id))
        .map_or_else(
            || ApplicationKey::LaunchSignature(canonical_identity.signature()),
            |id| ApplicationKey::Registered(id.to_lowercase()),
        );
    let launch_aliases = canonical_launch
        .filter(|canonical| canonical != &launch)
        .into_iter()
        .collect();
    let application = RegisteredApplication {
        key,
        id,
        name: entry.name.clone(),
        launch,
        launch_aliases,
        icon_source: entry.icon_source.clone(),
        app_user_model_id: entry
            .app_user_model_id
            .clone()
            .filter(|id| is_reliable_registered_id(id)),
        canonical_executables: executable.clone().into_iter().collect(),
        executable_aliases: aliases,
        provider_keys,
        is_host_app,
    };
    let search_entry = if embedded_arguments.is_some() {
        entry.with_embedded_arguments(arguments.as_deref())
    } else {
        entry.with_arguments(arguments.as_deref())
    };
    Some(CatalogCandidate {
        application,
        search_entry,
        preference,
    })
}

fn is_host_application(executable: &str) -> bool {
    windows_registry::CLASSES_ROOT
        .open(format!("Applications\\{executable}"))
        .and_then(|key| key.get_value("IsHostApp"))
        .is_ok()
}

fn catalog_preference(
    arguments: Option<&str>,
    resolved_target: Option<&std::path::Path>,
) -> u8 {
    if arguments.is_some_and(|arguments| {
        command_line_arguments(arguments)
            .iter()
            .map(String::as_str)
            .any(|argument| argument.eq_ignore_ascii_case("--processStart"))
    }) {
        return 0;
    }
    if resolved_target
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("app-"))
    {
        return 2;
    }
    1
}

fn strong_merge_keys(application: &RegisteredApplication) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(id) = application.app_user_model_id.as_deref() {
        keys.push(format!("registered:{}", id.to_lowercase()));
    }
    let shared_host = application.is_host_app
        || application
            .canonical_executables
            .iter()
            .filter_map(|path| normalized_executable_name(path))
            .any(|name| is_shared_host_executable(&name));
    if !shared_host {
        keys.push(format!("launch:{}", application.launch.signature()));
        keys.extend(
            application
                .launch_aliases
                .iter()
                .map(|launch| format!("launch:{}", launch.signature())),
        );
    }
    keys.extend(
        application
            .provider_keys
            .iter()
            .map(|key| format!("provider:{key}")),
    );
    keys.sort();
    keys.dedup();
    keys
}

fn merge_registered_application(
    retained: &mut RegisteredApplication,
    duplicate: &RegisteredApplication,
) {
    if !matches!(retained.key, ApplicationKey::Registered(_))
        && matches!(duplicate.key, ApplicationKey::Registered(_))
    {
        retained.key.clone_from(&duplicate.key);
        retained.id.clone_from(&duplicate.id);
        retained
            .app_user_model_id
            .clone_from(&duplicate.app_user_model_id);
    }
    retained.launch_aliases.push(duplicate.launch.clone());
    retained
        .launch_aliases
        .extend(duplicate.launch_aliases.iter().cloned());
    retained
        .canonical_executables
        .extend(duplicate.canonical_executables.iter().cloned());
    retained
        .executable_aliases
        .extend(duplicate.executable_aliases.iter().cloned());
    retained
        .provider_keys
        .extend(duplicate.provider_keys.iter().cloned());
    retained.is_host_app |= duplicate.is_host_app;
    retained.launch_aliases.sort();
    retained.launch_aliases.dedup();
    retained.canonical_executables.sort();
    retained.canonical_executables.dedup();
    retained.executable_aliases.sort();
    retained.executable_aliases.dedup();
    retained.provider_keys.sort();
    retained.provider_keys.dedup();
}

fn ambiguous_alias_count(applications: &[RegisteredApplication]) -> usize {
    let mut aliases = HashMap::<&str, usize>::new();
    for application in applications {
        for alias in &application.executable_aliases {
            *aliases.entry(alias).or_default() += 1;
        }
    }
    aliases.values().filter(|&&count| count > 1).count()
}

fn find(parents: &mut [usize], index: usize) -> usize {
    if parents[index] != index {
        parents[index] = find(parents, parents[index]);
    }
    parents[index]
}

fn union_if_compatible(
    parents: &mut [usize],
    component_ids: &mut [Option<String>],
    left: usize,
    right: usize,
) {
    let left = find(parents, left);
    let right = find(parents, right);
    if left == right
        || matches!(
            (&component_ids[left], &component_ids[right]),
            (Some(left), Some(right)) if left != right
        )
    {
        return;
    }
    let root = left.min(right);
    let child = left.max(right);
    parents[left] = root;
    parents[right] = root;
    if component_ids[root].is_none() {
        component_ids[root] = component_ids[child].take();
    }
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
