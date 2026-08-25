use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use lotus_core::application::{
    ApplicationKey, ApplicationPresentation, ApplicationPresentationIcon,
    ApplicationResolution, LaunchSpec, RegisteredApplication, ResolutionEvidence,
    WindowApplicationAssignments, WindowApplicationFacts, application_provider_keys,
    is_reliable_registered_id, is_shared_host_executable, normalized_executable_name,
    normalized_path, normalized_value,
};
use lotus_core::search::ApplicationEntry;
use lotus_core::settings::PinnedApp;
use lotus_core::window::{TrackedWindowKey, WindowInfo};

use crate::responsiveness::METRICS;

#[derive(Clone, Debug, Default)]
pub struct ApplicationCatalogIndex {
    application_keys: HashMap<ApplicationKey, usize>,
    application_ids: HashMap<String, CandidateSet>,
    registered_ids: HashMap<String, CandidateSet>,
    pin_launch_signatures: HashMap<String, CandidateSet>,
    launch_signatures: HashMap<String, CandidateSet>,
    provider_keys: HashMap<String, CandidateSet>,
    executable_paths: HashMap<String, CandidateSet>,
    executable_aliases: HashMap<String, CandidateSet>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CandidateSet {
    Unique(usize),
    Ambiguous(usize),
}

#[derive(Clone, Debug)]
pub struct ApplicationCatalogSnapshot {
    pub generation: u64,
    pub applications: Box<[RegisteredApplication]>,
    pub search_entries: Box<[ApplicationEntry]>,
    index: ApplicationCatalogIndex,
}

impl ApplicationCatalogSnapshot {
    #[must_use]
    pub fn new(generation: u64, applications: Vec<RegisteredApplication>) -> Self {
        Self::with_search_entries(generation, applications, Vec::new())
    }

    #[must_use]
    pub fn with_search_entries(
        generation: u64,
        applications: Vec<RegisteredApplication>,
        search_entries: Vec<ApplicationEntry>,
    ) -> Self {
        let mut index = ApplicationCatalogIndex::default();
        for (position, application) in applications.iter().enumerate() {
            index
                .application_keys
                .insert(application.key.clone(), position);
            insert(
                &mut index.application_ids,
                Some(application.id.clone()),
                position,
            );
            if let Some(id) = application.app_user_model_id.as_deref() {
                insert(&mut index.registered_ids, normalized_value(id), position);
            }
            for launch in
                std::iter::once(&application.launch).chain(&application.launch_aliases)
            {
                insert(
                    &mut index.pin_launch_signatures,
                    Some(launch.signature()),
                    position,
                );
                if runtime_launch_signature_is_safe(application, launch) {
                    insert(
                        &mut index.launch_signatures,
                        Some(launch.signature()),
                        position,
                    );
                }
            }
            for key in &application.provider_keys {
                insert(&mut index.provider_keys, Some(key.clone()), position);
            }
            for path in &application.canonical_executables {
                insert(&mut index.executable_paths, normalized_path(path), position);
            }
            for alias in &application.executable_aliases {
                insert(
                    &mut index.executable_aliases,
                    normalized_executable_name(alias),
                    position,
                );
            }
        }
        Self {
            generation,
            applications: applications.into_boxed_slice(),
            search_entries: search_entries.into_boxed_slice(),
            index,
        }
    }

    #[must_use]
    pub fn application(&self, index: usize) -> Option<&RegisteredApplication> {
        self.applications.get(index)
    }

    #[must_use]
    pub fn application_index_for_key(&self, key: &ApplicationKey) -> Option<usize> {
        self.index.application_keys.get(key).copied()
    }

    #[must_use]
    pub fn key_for_external_identifier(&self, value: &str) -> Option<ApplicationKey> {
        if let Lookup::Unique(index) = lookup(
            &self.index.application_ids,
            (!value.trim().is_empty()).then(|| value.trim().to_owned()),
        ) {
            return self
                .application(index)
                .map(|application| application.key.clone());
        }
        if is_reliable_registered_id(value) {
            return match lookup(&self.index.registered_ids, normalized_value(value)) {
                Lookup::Unique(index) => self
                    .application(index)
                    .map(|application| application.key.clone()),
                Lookup::Ambiguous(_) => None,
                Lookup::Missing => normalized_value(value).map(ApplicationKey::Registered),
            };
        }
        if let Lookup::Unique(index) =
            lookup(&self.index.executable_paths, normalized_path(value))
        {
            return self
                .application(index)
                .map(|application| application.key.clone());
        }
        let alias = normalized_executable_name(value)?;
        if is_shared_host_executable(&alias) {
            return None;
        }
        if let Lookup::Unique(index) = lookup(&self.index.executable_aliases, Some(alias)) {
            return self
                .application(index)
                .map(|application| application.key.clone());
        }
        None
    }

    #[must_use]
    pub fn key_for_pin(
        &self,
        id: &str,
        app_user_model_id: Option<&str>,
        launch: &LaunchSpec,
        executable_aliases: &[String],
    ) -> Option<ApplicationKey> {
        if let Some(value) = app_user_model_id.filter(|id| is_reliable_registered_id(id)) {
            if let Lookup::Unique(index) =
                lookup(&self.index.registered_ids, normalized_value(value))
            {
                return self
                    .application(index)
                    .map(|application| application.key.clone());
            }
            return normalized_value(value).map(ApplicationKey::Registered);
        }
        if let Lookup::Unique(index) =
            lookup(&self.index.registered_ids, normalized_value(id))
        {
            return self
                .application(index)
                .map(|application| application.key.clone());
        }
        if let Lookup::Unique(index) =
            lookup(&self.index.pin_launch_signatures, Some(launch.signature()))
        {
            return self
                .application(index)
                .map(|application| application.key.clone());
        }
        for alias in executable_aliases {
            if is_shared_host_executable(alias) {
                continue;
            }
            if let Lookup::Unique(index) = lookup(
                &self.index.executable_aliases,
                normalized_executable_name(alias),
            ) {
                return self
                    .application(index)
                    .map(|application| application.key.clone());
            }
        }
        Some(ApplicationKey::from_launch_fallback(launch))
    }
}

fn runtime_launch_signature_is_safe(
    application: &RegisteredApplication,
    launch: &LaunchSpec,
) -> bool {
    launch.arguments.is_some()
        || !application
            .canonical_executables
            .iter()
            .filter_map(|path| normalized_executable_name(path))
            .any(|name| is_shared_host_executable(&name))
}

#[derive(Default)]
pub struct ApplicationResolver {
    cache: HashMap<TrackedWindowKey, CachedResolution>,
    associations: ApplicationAssociations,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplicationAssociations {
    executable_aliases: HashMap<String, AssociationCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AssociationCandidate {
    Unique(ApplicationKey),
    Ambiguous(usize),
}

impl ApplicationAssociations {
    #[must_use]
    pub fn from_pins(pins: &[PinnedApp], catalog: &ApplicationCatalogSnapshot) -> Self {
        let mut executable_aliases = HashMap::new();
        for pin in pins {
            let Some(launch) =
                LaunchSpec::new(&pin.launch_target, pin.arguments.as_deref())
            else {
                continue;
            };
            let Some(key) = catalog.key_for_pin(
                &pin.id,
                pin.app_user_model_id.as_deref(),
                &launch,
                &pin.match_executables,
            ) else {
                continue;
            };
            for alias in &pin.match_executables {
                let Some(alias) = normalized_executable_name(alias) else {
                    continue;
                };
                if is_shared_host_executable(&alias) {
                    continue;
                }
                insert_association(&mut executable_aliases, alias, key.clone());
            }
        }
        Self { executable_aliases }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationResolutionExplanation {
    pub tracked_key: TrackedWindowKey,
    pub executable: String,
    pub window_app_user_model_id: Option<String>,
    pub process_app_user_model_id: Option<String>,
    pub relaunch_signature: Option<String>,
    pub provider_keys: Vec<String>,
    pub selected_application: Option<String>,
    pub resolution: ApplicationResolution,
}

#[derive(Clone)]
struct CachedResolution {
    generation: u64,
    fingerprint: WindowIdentityFingerprint,
    resolution: ApplicationResolution,
}

#[derive(Clone, Eq, PartialEq)]
struct WindowIdentityFingerprint {
    window_id: Option<String>,
    process_id: Option<String>,
    relaunch: Option<String>,
    executable: Option<String>,
    prevent_pinning: bool,
}

impl ApplicationResolver {
    #[must_use]
    pub fn resolve_all(
        &mut self,
        windows: &[WindowInfo],
        catalog: &ApplicationCatalogSnapshot,
        associations: &ApplicationAssociations,
        window_revision: u64,
    ) -> WindowApplicationAssignments {
        let started = Instant::now();
        if self.associations != *associations {
            self.cache.clear();
            self.associations.clone_from(associations);
        }
        self.cache
            .retain(|key, _| windows.iter().any(|window| window.key() == *key));
        let mut by_window = HashMap::with_capacity(windows.len());
        let mut presentation_by_window = HashMap::with_capacity(windows.len());
        for window in windows {
            let key = window.key();
            let fingerprint = WindowIdentityFingerprint::from_window(window);
            let cached = self
                .cache
                .get(&key)
                .filter(|cached| {
                    cached.generation == catalog.generation
                        && cached.fingerprint == fingerprint
                })
                .map(|cached| cached.resolution.clone());
            let was_cached = cached.is_some();
            let resolution = cached.unwrap_or_else(|| {
                let resolution = resolve_window(window, catalog, associations);
                self.cache.insert(
                    key,
                    CachedResolution {
                        generation: catalog.generation,
                        fingerprint,
                        resolution: resolution.clone(),
                    },
                );
                resolution
            });
            let presentation = application_presentation(window, &resolution, catalog);
            METRICS.record_application_resolution(was_cached, &resolution);
            by_window.insert(key, resolution);
            presentation_by_window.insert(key, presentation);
        }
        METRICS.record_application_resolution_batch(started.elapsed());
        WindowApplicationAssignments {
            catalog_generation: catalog.generation,
            window_revision,
            by_window,
            presentation_by_window,
        }
    }

    #[must_use]
    pub fn explain_window(
        window: &WindowInfo,
        catalog: &ApplicationCatalogSnapshot,
        associations: &ApplicationAssociations,
    ) -> ApplicationResolutionExplanation {
        let resolution = resolve_window(window, catalog, associations);
        let selected_application = match &resolution {
            ApplicationResolution::Resolved {
                registered_index, ..
            } => catalog
                .application(*registered_index)
                .map(|application| application.name.clone()),
            ApplicationResolution::Associated { key } => catalog
                .application_index_for_key(key)
                .and_then(|index| catalog.application(index))
                .map(|application| application.name.clone()),
            _ => None,
        };
        ApplicationResolutionExplanation {
            tracked_key: window.key(),
            executable: window.executable_path.to_string_lossy().into_owned(),
            window_app_user_model_id: window
                .application_facts
                .window_app_user_model_id
                .clone(),
            process_app_user_model_id: window
                .application_facts
                .process_app_user_model_id
                .clone(),
            relaunch_signature: window
                .application_facts
                .relaunch
                .as_ref()
                .map(LaunchSpec::signature),
            provider_keys: provider_keys(&window.application_facts),
            selected_application,
            resolution,
        }
    }
}

fn application_presentation(
    window: &WindowInfo,
    resolution: &ApplicationResolution,
    catalog: &ApplicationCatalogSnapshot,
) -> ApplicationPresentation {
    if let Some(application) = registered_presentation(resolution, catalog) {
        return ApplicationPresentation {
            display_name: application.name.clone(),
            icon: ApplicationPresentationIcon::Source(application.icon_source.clone()),
        };
    }

    let executable_path = window.executable_path.to_string_lossy().into_owned();
    let display_name = window
        .application_facts
        .display_name
        .as_deref()
        .and_then(nonblank)
        .or_else(|| nonblank(&window.title))
        .map_or_else(|| executable_stem(&window.executable_path), str::to_owned);
    ApplicationPresentation {
        display_name,
        icon: ApplicationPresentationIcon::NativeWindow {
            key: window.key(),
            fallback_path: executable_path,
        },
    }
}

fn registered_presentation<'a>(
    resolution: &ApplicationResolution,
    catalog: &'a ApplicationCatalogSnapshot,
) -> Option<&'a RegisteredApplication> {
    match resolution {
        ApplicationResolution::Resolved {
            registered_index, ..
        } => catalog.application(*registered_index),
        ApplicationResolution::Associated { key } => catalog
            .application_index_for_key(key)
            .and_then(|index| catalog.application(index)),
        ApplicationResolution::Prevented
        | ApplicationResolution::Ambiguous { .. }
        | ApplicationResolution::Unregistered { .. } => None,
    }
}

fn nonblank(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn executable_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .and_then(nonblank)
        .unwrap_or("Application")
        .to_owned()
}

impl WindowIdentityFingerprint {
    fn from_window(window: &WindowInfo) -> Self {
        let facts = &window.application_facts;
        Self {
            window_id: reliable_id(facts.window_app_user_model_id.as_deref()),
            process_id: reliable_id(facts.process_app_user_model_id.as_deref()),
            relaunch: facts.relaunch.as_ref().map(LaunchSpec::signature),
            executable: normalized_path(&window.executable_path.to_string_lossy()),
            prevent_pinning: facts.prevent_pinning,
        }
    }
}

fn resolve_window(
    window: &WindowInfo,
    catalog: &ApplicationCatalogSnapshot,
    associations: &ApplicationAssociations,
) -> ApplicationResolution {
    let facts = &window.application_facts;
    if facts.prevent_pinning {
        return ApplicationResolution::Prevented;
    }
    let window_id = reliable_id(facts.window_app_user_model_id.as_deref());
    let process_id = reliable_id(facts.process_app_user_model_id.as_deref());
    if let (Some(left), Some(right)) = (&window_id, &process_id)
        && left != right
    {
        return ApplicationResolution::Ambiguous {
            evidence: ResolutionEvidence::ExactRegisteredId,
            candidate_count: 2,
        };
    }
    if let Some(id) = window_id.as_deref().or(process_id.as_deref()) {
        return resolve_lookup(
            &catalog.index.registered_ids,
            normalized_value(id),
            catalog,
            ResolutionEvidence::ExactRegisteredId,
        )
        .unwrap_or_else(|| ApplicationResolution::Unregistered {
            key: ApplicationKey::Registered(id.to_owned()),
            launch: facts.relaunch.clone(),
        });
    }
    if let Some(alias) =
        normalized_executable_name(&window.executable_path.to_string_lossy())
        && !is_shared_host_executable(&alias)
        && let Some(association) = associations.executable_aliases.get(&alias)
    {
        return match association {
            AssociationCandidate::Unique(key) => {
                ApplicationResolution::Associated { key: key.clone() }
            }
            AssociationCandidate::Ambiguous(candidate_count) => {
                ApplicationResolution::Ambiguous {
                    evidence: ResolutionEvidence::ExplicitAssociation,
                    candidate_count: *candidate_count,
                }
            }
        };
    }
    if let Some(relaunch) = facts.relaunch.as_ref()
        && let Some(resolution) = resolve_lookup(
            &catalog.index.launch_signatures,
            Some(relaunch.signature()),
            catalog,
            ResolutionEvidence::ExactRelaunch,
        )
    {
        return resolution;
    }
    for provider_key in provider_keys(facts) {
        if let Some(resolution) = resolve_lookup(
            &catalog.index.provider_keys,
            Some(provider_key),
            catalog,
            ResolutionEvidence::ExactProviderKey,
        ) {
            return resolution;
        }
    }
    let executable = normalized_path(&window.executable_path.to_string_lossy());
    let shared = executable
        .as_deref()
        .and_then(normalized_executable_name)
        .is_some_and(|name| is_shared_host_executable(&name));
    if !shared {
        if let Some(resolution) = resolve_lookup(
            &catalog.index.executable_paths,
            executable.clone(),
            catalog,
            ResolutionEvidence::ExactExecutablePath,
        ) {
            return resolution;
        }
        if let Some(resolution) = resolve_lookup(
            &catalog.index.executable_aliases,
            executable.as_deref().and_then(normalized_executable_name),
            catalog,
            ResolutionEvidence::UniqueExecutableAlias,
        ) {
            return resolution;
        }
    }
    ApplicationResolution::Unregistered {
        key: facts.relaunch.as_ref().map_or_else(
            || {
                if shared {
                    ApplicationKey::Ephemeral(window.key())
                } else {
                    executable.map_or_else(
                        || ApplicationKey::Ephemeral(window.key()),
                        ApplicationKey::ExecutablePath,
                    )
                }
            },
            |relaunch| {
                if shared && relaunch.arguments.is_none() {
                    ApplicationKey::Ephemeral(window.key())
                } else {
                    ApplicationKey::from_launch_fallback(relaunch)
                }
            },
        ),
        launch: facts.relaunch.clone(),
    }
}

fn reliable_id(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| is_reliable_registered_id(value))
        .and_then(normalized_value)
}

fn provider_keys(facts: &WindowApplicationFacts) -> Vec<String> {
    application_provider_keys(
        facts.reliable_id(),
        facts
            .relaunch
            .as_ref()
            .and_then(|launch| launch.arguments.as_deref()),
    )
}

enum Lookup {
    Unique(usize),
    Ambiguous(usize),
    Missing,
}

fn lookup(index: &HashMap<String, CandidateSet>, key: Option<String>) -> Lookup {
    key.and_then(|key| index.get(&key))
        .map_or(Lookup::Missing, |set| match set {
            CandidateSet::Unique(index) => Lookup::Unique(*index),
            CandidateSet::Ambiguous(count) => Lookup::Ambiguous(*count),
        })
}

fn resolve_lookup(
    index: &HashMap<String, CandidateSet>,
    key: Option<String>,
    catalog: &ApplicationCatalogSnapshot,
    evidence: ResolutionEvidence,
) -> Option<ApplicationResolution> {
    match lookup(index, key) {
        Lookup::Unique(index) => {
            catalog
                .application(index)
                .map(|application| ApplicationResolution::Resolved {
                    key: application.key.clone(),
                    registered_index: index,
                    evidence,
                })
        }
        Lookup::Ambiguous(candidate_count) => Some(ApplicationResolution::Ambiguous {
            evidence,
            candidate_count,
        }),
        Lookup::Missing => None,
    }
}

fn insert(index: &mut HashMap<String, CandidateSet>, key: Option<String>, position: usize) {
    let Some(key) = key else {
        return;
    };
    match index.get_mut(&key) {
        None => {
            index.insert(key, CandidateSet::Unique(position));
        }
        Some(CandidateSet::Unique(existing)) if *existing != position => {
            *index.get_mut(&key).expect("candidate exists") = CandidateSet::Ambiguous(2);
        }
        Some(CandidateSet::Ambiguous(count)) => *count += 1,
        Some(CandidateSet::Unique(_)) => {}
    }
}

fn insert_association(
    associations: &mut HashMap<String, AssociationCandidate>,
    alias: String,
    key: ApplicationKey,
) {
    match associations.get_mut(&alias) {
        None => {
            associations.insert(alias, AssociationCandidate::Unique(key));
        }
        Some(AssociationCandidate::Unique(existing)) if *existing != key => {
            *associations.get_mut(&alias).expect("association exists") =
                AssociationCandidate::Ambiguous(2);
        }
        Some(AssociationCandidate::Ambiguous(count)) => *count += 1,
        Some(AssociationCandidate::Unique(_)) => {}
    }
}
