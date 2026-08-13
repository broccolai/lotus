use std::cmp::Reverse;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

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
        serde_json::from_str(source)
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
        self.entries.push(SearchUsageEntry { target, launches: 1, last_used: self.sequence });
        true
    }

    fn rank(&self, launch_target: &str) -> UsageRank {
        let target = normalize_target(launch_target);
        self.entries.iter().find(|entry| entry.target == target).map_or(
            UsageRank::default(),
            |entry| UsageRank { launches: entry.launches, last_used: entry.last_used },
        )
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
    pub icon_source: String,
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
            source: ApplicationSource::default(),
            hidden_until_search: false,
        }
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
}

#[derive(Clone, Debug, Default)]
pub struct SearchCatalog {
    entries: Vec<CatalogEntry>,
}

impl SearchCatalog {
    pub fn new(entries: impl IntoIterator<Item = ApplicationEntry>) -> Self {
        let mut seen_names = HashSet::new();
        let mut catalog = Vec::new();

        for mut entry in entries {
            entry.name = entry.name.trim().into();
            entry.launch_target = entry.launch_target.trim().into();
            if entry.name.is_empty() || entry.launch_target.is_empty() {
                continue;
            }

            let normalized_name = normalize(&entry.name);
            if normalized_name.is_empty() || !seen_names.insert(identity_key(&normalized_name)) {
                continue;
            }

            let ordinal = catalog.len();
            catalog.push(CatalogEntry { entry, normalized_name, ordinal });
        }

        Self { entries: catalog }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<&ApplicationEntry> {
        self.search_with_usage(query, limit, &SearchUsage::default())
    }

    pub fn search_with_usage(
        &self,
        query: &str,
        limit: usize,
        usage: &SearchUsage,
    ) -> Vec<&ApplicationEntry> {
        let normalized_query = normalize(query);
        let mut ranked = self
            .entries
            .iter()
            .filter_map(|candidate| {
                if normalized_query.is_empty() && candidate.entry.hidden_until_search {
                    return None;
                }
                score(&candidate.normalized_name, &normalized_query).map(|score| (candidate, score))
            })
            .collect::<Vec<_>>();

        ranked.sort_by(|(left, left_score), (right, right_score)| {
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
                .then_with(|| {
                    Reverse(usage.rank(&left.entry.launch_target))
                        .cmp(&Reverse(usage.rank(&right.entry.launch_target)))
                })
                .then_with(|| {
                    if normalized_query.is_empty() {
                        left.ordinal.cmp(&right.ordinal)
                    } else {
                        compare_names(&left.entry.name, &right.entry.name)
                    }
                })
                .then_with(|| left.entry.name.cmp(&right.entry.name))
                .then_with(|| left.entry.launch_target.cmp(&right.entry.launch_target))
        });

        ranked.into_iter().take(limit).map(|(candidate, _)| &candidate.entry).collect()
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
    normalized_name: String,
    ordinal: usize,
}

fn compare_names(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MatchScore {
    quality: i32,
    penalty: i32,
}

fn score(candidate: &str, query: &str) -> Option<MatchScore> {
    if query.is_empty() {
        return Some(MatchScore { quality: 0, penalty: 0 });
    }

    let candidate_chars = candidate.chars().collect::<Vec<_>>();
    let mut quality = 0;
    let mut penalty = 0;
    for token in query.split_whitespace() {
        let token = score_token(&candidate_chars, &token.chars().collect::<Vec<_>>())?;
        quality += token.quality;
        penalty += token.penalty;
    }

    penalty += i32::try_from(candidate_chars.len()).unwrap_or(i32::MAX);
    Some(MatchScore { quality, penalty })
}

fn score_token(candidate: &[char], token: &[char]) -> Option<MatchScore> {
    if candidate == token {
        return Some(MatchScore { quality: 1_400, penalty: 0 });
    }

    if candidate.starts_with(token) {
        return Some(MatchScore { quality: 1_200, penalty: 0 });
    }

    if let Some(index) = find_slice(candidate, token, true) {
        return Some(MatchScore { quality: 1_000, penalty: as_score_index(index) });
    }

    if let Some(index) = find_slice(candidate, token, false) {
        return Some(MatchScore { quality: 800, penalty: as_score_index(index) });
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

    Some(MatchScore { quality: 500, penalty: as_score_index(gap_penalty) })
}

fn find_slice(candidate: &[char], token: &[char], require_word_start: bool) -> Option<usize> {
    if token.is_empty() || token.len() > candidate.len() {
        return None;
    }

    candidate.windows(token.len()).enumerate().find_map(|(index, window)| {
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

fn identity_key(normalized_name: &str) -> String {
    normalized_name.chars().filter(|character| *character != ' ').collect()
}

fn normalize_target(value: &str) -> String {
    value.trim().replace('/', "\\").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{ApplicationEntry, SearchCatalog};

    #[test]
    fn hidden_entries_stay_out_of_default_results_but_can_be_intentionally_searched() {
        let catalog = SearchCatalog::new([
            ApplicationEntry::new("Calculator", "calc.exe", None),
            ApplicationEntry::new("ChatGPT", "chatgpt.exe", None).hidden_until_search(),
        ]);

        assert_eq!(catalog.search("", 8)[0].name, "Calculator");
        assert!(catalog.search("", 8).iter().all(|entry| entry.name != "ChatGPT"));
        assert_eq!(catalog.search("chat", 8)[0].name, "ChatGPT");
        assert_eq!(catalog.search("gpt", 8)[0].name, "ChatGPT");
    }
}
