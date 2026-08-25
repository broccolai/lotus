use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::application::{ApplicationIdentity, LaunchSpec};

const MAX_USAGE_ENTRIES: usize = 64;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchUsage {
    sequence: u64,
    entries: Vec<SearchUsageEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchUsageEntry {
    target: String,
    launches: u32,
    last_used: u64,
}

impl SearchUsage {
    pub fn from_json(source: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str::<Self>(source).map(Self::normalized)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn record_launch(&mut self, launch_target: &str) -> bool {
        let target = normalize_target(launch_target);
        if target.is_empty() {
            return false;
        }
        self.sequence = self.sequence.saturating_add(1);
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.target == target) {
            entry.launches = entry.launches.saturating_add(1);
            entry.last_used = self.sequence;
            return true;
        }
        if self.entries.len() >= MAX_USAGE_ENTRIES
            && let Some((index, _)) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| (entry.launches, entry.last_used))
        {
            self.entries.swap_remove(index);
        }
        self.entries.push(SearchUsageEntry {
            target,
            launches: 1,
            last_used: self.sequence,
        });
        true
    }

    fn rank(&self, launch_target: &str) -> UsageRank {
        let target = normalize_target(launch_target);
        self.entries
            .iter()
            .find(|entry| entry.target == target)
            .map_or(UsageRank::default(), |entry| UsageRank {
                launches: entry.launches,
                last_used: entry.last_used,
            })
    }

    fn normalized(mut self) -> Self {
        let mut entries = BTreeMap::<String, SearchUsageEntry>::new();

        for entry in self.entries {
            let target = normalize_target(&entry.target);
            if target.is_empty() {
                continue;
            }

            entries
                .entry(target.clone())
                .and_modify(|current| {
                    current.launches = current.launches.saturating_add(entry.launches);
                    current.last_used = current.last_used.max(entry.last_used);
                })
                .or_insert(SearchUsageEntry {
                    target,
                    launches: entry.launches,
                    last_used: entry.last_used,
                });
        }

        let mut entries = entries.into_values().collect::<Vec<_>>();
        entries.sort_unstable_by(|left, right| {
            right
                .launches
                .cmp(&left.launches)
                .then_with(|| right.last_used.cmp(&left.last_used))
                .then_with(|| left.target.cmp(&right.target))
        });
        entries.truncate(MAX_USAGE_ENTRIES);

        self.sequence = self.sequence.max(
            entries
                .iter()
                .map(|entry| entry.last_used)
                .max()
                .unwrap_or_default(),
        );
        self.entries = entries;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct UsageRank {
    launches: u32,
    last_used: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationEntry {
    pub name: String,
    pub launch_target: String,
    pub arguments: Option<String>,
    arguments_embedded_in_target: bool,
    pub icon_source: String,
    pub app_user_model_id: Option<String>,
    pub source: ApplicationSource,
    pub hidden_until_search: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum ApplicationSource {
    Running,
    #[default]
    Installed,
    Pinned,
}

impl ApplicationEntry {
    pub fn new(
        name: impl Into<String>,
        launch_target: impl Into<String>,
        icon_source: Option<String>,
    ) -> Self {
        let launch_target = launch_target.into();
        Self {
            name: name.into(),
            icon_source: icon_source
                .filter(|source| !source.trim().is_empty())
                .unwrap_or_else(|| launch_target.clone()),
            launch_target,
            arguments: None,
            arguments_embedded_in_target: false,
            app_user_model_id: None,
            source: ApplicationSource::default(),
            hidden_until_search: false,
        }
    }

    #[must_use]
    pub fn with_arguments(mut self, arguments: Option<&str>) -> Self {
        self.arguments = arguments
            .map(str::trim)
            .filter(|arguments| !arguments.is_empty())
            .map(str::to_owned);
        self
    }

    #[must_use]
    pub fn with_embedded_arguments(mut self, arguments: Option<&str>) -> Self {
        self = self.with_arguments(arguments);
        self.arguments_embedded_in_target = self.arguments.is_some();
        self
    }

    #[must_use]
    pub fn invocation_arguments(&self) -> Option<&str> {
        (!self.arguments_embedded_in_target)
            .then_some(self.arguments.as_deref())
            .flatten()
    }

    #[must_use]
    pub fn with_app_user_model_id(mut self, app_user_model_id: impl Into<String>) -> Self {
        let app_user_model_id = app_user_model_id.into();
        self.app_user_model_id =
            (!app_user_model_id.trim().is_empty()).then_some(app_user_model_id);
        self
    }

    #[must_use]
    pub const fn with_source(mut self, source: ApplicationSource) -> Self {
        self.source = source;
        self
    }

    #[must_use]
    pub const fn hidden_until_search(mut self) -> Self {
        self.hidden_until_search = true;
        self
    }

    #[must_use]
    pub fn application_identity(&self) -> ApplicationIdentity {
        ApplicationIdentity::new(
            self.app_user_model_id.as_deref(),
            Some(&self.launch_target),
            None,
            std::iter::empty(),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchCatalog {
    entries: Vec<CatalogEntry>,
}

impl SearchCatalog {
    pub fn new(entries: impl IntoIterator<Item = ApplicationEntry>) -> Self {
        let mut seen_identities = HashSet::new();
        let mut catalog = Vec::new();

        for mut entry in entries {
            entry.name = entry.name.trim().into();
            entry.launch_target = entry.launch_target.trim().into();
            if entry.name.is_empty() || entry.launch_target.is_empty() {
                continue;
            }

            let normalized_name = normalize(&entry.name);
            if normalized_name.is_empty()
                || !seen_identities.insert(application_identity(&entry))
            {
                continue;
            }

            let ordinal = catalog.len();
            catalog.push(CatalogEntry {
                normalized_name_chars: normalized_name.chars().collect(),
                sort_name: entry.name.to_lowercase(),
                entry,
                ordinal,
            });
        }

        Self { entries: catalog }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<&ApplicationEntry> {
        self.search_with_usage(query, limit, &SearchUsage::default())
    }

    pub fn entries_for_management(&self) -> impl Iterator<Item = &ApplicationEntry> {
        self.entries.iter().map(|candidate| &candidate.entry)
    }

    pub fn search_with_usage(
        &self,
        query: &str,
        limit: usize,
        usage: &SearchUsage,
    ) -> Vec<&ApplicationEntry> {
        let normalized_query = normalize(query);
        let query_tokens = query_tokens(&normalized_query);
        let mut ranked = self
            .entries
            .iter()
            .filter_map(|candidate| {
                if normalized_query.is_empty() && candidate.entry.hidden_until_search {
                    return None;
                }
                score(&candidate.normalized_name_chars, &query_tokens).map(|score| {
                    let usage = usage.rank(&candidate.entry.launch_target);
                    (candidate, score, usage)
                })
            })
            .collect::<Vec<_>>();

        ranked.sort_by(
            |(left, left_score, left_usage), (right, right_score, right_usage)| {
                Reverse(left_score.quality)
                    .cmp(&Reverse(right_score.quality))
                    .then_with(|| {
                        if normalized_query.is_empty() {
                            std::cmp::Ordering::Equal
                        } else {
                            Reverse(left.entry.source).cmp(&Reverse(right.entry.source))
                        }
                    })
                    .then_with(|| left_score.penalty.cmp(&right_score.penalty))
                    .then_with(|| Reverse(left_usage).cmp(&Reverse(right_usage)))
                    .then_with(|| {
                        if normalized_query.is_empty() {
                            left.ordinal.cmp(&right.ordinal)
                        } else {
                            left.sort_name.cmp(&right.sort_name)
                        }
                    })
                    .then_with(|| left.entry.name.cmp(&right.entry.name))
                    .then_with(|| left.entry.launch_target.cmp(&right.entry.launch_target))
            },
        );

        ranked
            .into_iter()
            .take(limit)
            .map(|(candidate, _, _)| &candidate.entry)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Clone, Debug)]
struct CatalogEntry {
    entry: ApplicationEntry,
    normalized_name_chars: Vec<char>,
    sort_name: String,
    ordinal: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MatchScore {
    quality: i32,
    penalty: i32,
}

fn score(candidate: &[char], query_tokens: &[Vec<char>]) -> Option<MatchScore> {
    if query_tokens.is_empty() {
        return Some(MatchScore {
            quality: 0,
            penalty: 0,
        });
    }

    let mut quality = 0;
    let mut penalty = 0;
    for token in query_tokens {
        let token = score_token(candidate, token)?;
        quality += token.quality;
        penalty += token.penalty;
    }

    penalty += i32::try_from(candidate.len()).unwrap_or(i32::MAX);
    Some(MatchScore { quality, penalty })
}

fn query_tokens(query: &str) -> Vec<Vec<char>> {
    query
        .split_whitespace()
        .map(|token| token.chars().collect())
        .collect()
}

fn score_token(candidate: &[char], token: &[char]) -> Option<MatchScore> {
    if candidate == token {
        return Some(MatchScore {
            quality: 1_400,
            penalty: 0,
        });
    }

    if candidate.starts_with(token) {
        return Some(MatchScore {
            quality: 1_200,
            penalty: 0,
        });
    }

    if let Some(index) = find_slice(candidate, token, true) {
        return Some(MatchScore {
            quality: 1_000,
            penalty: as_score_index(index),
        });
    }

    if let Some(index) = find_slice(candidate, token, false) {
        return Some(MatchScore {
            quality: 800,
            penalty: as_score_index(index),
        });
    }

    let mut candidate_index = 0;
    let mut gap_penalty = 0;
    for character in token {
        let relative_match = candidate[candidate_index..]
            .iter()
            .position(|candidate_character| candidate_character == character)?;
        gap_penalty += relative_match;
        candidate_index += relative_match + 1;
    }

    Some(MatchScore {
        quality: 500,
        penalty: as_score_index(gap_penalty),
    })
}

fn find_slice(
    candidate: &[char],
    token: &[char],
    require_word_start: bool,
) -> Option<usize> {
    if token.is_empty() || token.len() > candidate.len() {
        return None;
    }

    candidate
        .windows(token.len())
        .enumerate()
        .find_map(|(index, window)| {
            if window != token {
                return None;
            }

            if require_word_start {
                (index > 0 && candidate[index - 1] == ' ').then_some(index - 1)
            } else {
                Some(index)
            }
        })
}

fn as_score_index(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}

fn normalize(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut pending_space = false;

    for character in value.trim().to_lowercase().chars() {
        if character.is_alphanumeric() {
            if pending_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push(character);
            pending_space = false;
        } else {
            pending_space = true;
        }
    }

    normalized
}

fn application_identity(entry: &ApplicationEntry) -> String {
    let identity = entry.application_identity();
    if entry.arguments.is_none() && identity.reliable_registered_id().is_some() {
        return identity
            .deduplication_key()
            .expect("a reliable registered identity has a deduplication key");
    }

    let launch = LaunchSpec::new(&entry.launch_target, entry.arguments.as_deref())
        .map_or_else(
            || format!("target:{}", normalize_target(&entry.launch_target)),
            |launch| format!("launch:{}", launch.signature()),
        );
    identity
        .reliable_registered_id()
        .map_or(launch.clone(), |id| format!("registered:{id}:{launch}"))
}

fn normalize_target(value: &str) -> String {
    value.trim().replace('/', "\\").to_lowercase()
}
