use semver::Version;
use serde::Deserialize;
use ureq::Agent;

const RELEASE_NOTES_API: &str = "https://broccol.ai/api/lotus/releases";
const RELEASE_NOTES_LIMIT: u64 = 256 * 1024;
const RELEASE_NOTES_MAX_BYTES: usize = 16 * 1024;

pub(super) fn fetch(version: &str, github_body: &str) -> String {
    let selected = Version::parse(version).ok();
    let site_body = selected.and_then(|selected| {
        request().and_then(|response| matching_body(response, &selected))
    });
    site_body
        .as_deref()
        .and_then(|body| meaningful(body, version))
        .or_else(|| meaningful(github_body, version))
        .map(limit)
        .unwrap_or_default()
}

fn request() -> Option<ReleaseNotesResponse> {
    let mut response = agent()
        .get(RELEASE_NOTES_API)
        .header("Accept", "application/json")
        .header("User-Agent", concat!("Lotus/", env!("CARGO_PKG_VERSION")))
        .call()
        .ok()?;
    response
        .body_mut()
        .with_config()
        .limit(RELEASE_NOTES_LIMIT)
        .read_json()
        .ok()
}

fn matching_body(response: ReleaseNotesResponse, selected: &Version) -> Option<String> {
    let latest = response
        .latest
        .into_iter()
        .chain(response.releases)
        .find(|release| {
            release
                .version
                .as_deref()
                .and_then(normalized_version)
                .as_ref()
                == Some(selected)
        })?;
    latest.body
}

fn normalized_version(version: &str) -> Option<Version> {
    Version::parse(version.trim().strip_prefix('v').unwrap_or(version.trim())).ok()
}

fn meaningful(body: &str, version: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty()
        || [
            format!("release: lotus {version}"),
            format!("lotus v{version}"),
        ]
        .iter()
        .any(|placeholder| body.eq_ignore_ascii_case(placeholder))
    {
        None
    } else {
        Some(body.to_owned())
    }
}

fn limit(body: String) -> String {
    if body.len() <= RELEASE_NOTES_MAX_BYTES {
        return body;
    }
    let end = body
        .char_indices()
        .take_while(|(index, character)| {
            index.saturating_add(character.len_utf8()) <= RELEASE_NOTES_MAX_BYTES
        })
        .last()
        .map_or(0, |(index, character)| index + character.len_utf8());
    body[..end].to_owned()
}

fn agent() -> Agent {
    Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(3)))
        .build()
        .into()
}

#[derive(Deserialize)]
struct ReleaseNotesResponse {
    #[serde(default)]
    latest: Option<SiteRelease>,
    #[serde(default)]
    releases: Vec<SiteRelease>,
}

#[derive(Deserialize)]
struct SiteRelease {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    body: Option<String>,
}
