use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use std::{env, fs, io, thread};

use lotus_core::dock::DockItem;
use lotus_core::search::{ApplicationEntry, ApplicationSource, SearchCatalog};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_CommonPrograms, FOLDERID_Desktop,
    FOLDERID_Programs, FOLDERID_PublicDesktop, IEnumShellItems, IShellItem,
    KF_FLAG_DEFAULT, SHGetKnownFolderItem, SHGetKnownFolderPath, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING,
};
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};
use windows::core::{GUID, PWSTR};

use super::launch::{ComApartment, resolve_executable, shortcut_arguments};

const WINDOWS_SETTINGS_NAME: &str = "Windows Settings";
const WINDOWS_SETTINGS_TARGET: &str = "ms-settings:";
const SEARCH_CATALOG_WAKE_MESSAGE: u32 = WM_APP + 0x4C6;

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
            // SAFETY: Captures the UI thread constructing the app-owned cache.
            owner_thread: unsafe { GetCurrentThreadId() },
        };
        let _ = cache.refresh_if_stale(Duration::ZERO);
        cache
    }
}

struct RefreshCompletion {
    state: Arc<Mutex<CacheState>>,
    owner_thread: u32,
}

impl Drop for RefreshCompletion {
    fn drop(&mut self) {
        lock(&self.state).refreshing = false;
        // SAFETY: The captured thread id belongs to the UI thread that created
        // the cache. Posting a value-only thread message transfers no pointers.
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

fn compose_catalog(
    dock_items: &[DockItem],
    discovered_entries: impl IntoIterator<Item = ApplicationEntry>,
    hidden_executables: &[String],
) -> SearchCatalog {
    let dock_entry = |item: &DockItem| {
        ApplicationEntry::new(
            item.display_name.clone(),
            item.launch_target.clone(),
            Some(item.executable_path.clone()),
        )
        .with_source(if item.is_pinned {
            ApplicationSource::Pinned
        } else {
            ApplicationSource::Running
        })
    };
    let mut entries = dock_items
        .iter()
        .filter(|item| item.is_pinned)
        .map(dock_entry)
        .collect::<Vec<_>>();

    entries.extend(discovered_entries.into_iter().map(|mut entry| {
        if entry.icon_source.starts_with(r"shell:AppsFolder\")
            && let Some(item) = dock_items
                .iter()
                .find(|item| item.display_name.eq_ignore_ascii_case(&entry.name))
        {
            entry.icon_source.clone_from(&item.executable_path);
        }
        if matches_hidden_executable(&entry, hidden_executables) {
            entry.hidden_until_search()
        } else {
            entry
        }
    }));
    entries.extend(
        dock_items
            .iter()
            .filter(|item| !item.is_pinned)
            .map(dock_entry),
    );

    entries.push(ApplicationEntry::new(
        WINDOWS_SETTINGS_NAME,
        WINDOWS_SETTINGS_TARGET,
        Some(WINDOWS_SETTINGS_TARGET.into()),
    ));
    SearchCatalog::new(entries)
}

fn matches_hidden_executable(
    entry: &ApplicationEntry,
    hidden_executables: &[String],
) -> bool {
    hidden_executables.iter().any(|hidden| {
        let hidden = Path::new(hidden);
        let matches_path = [entry.launch_target.as_str(), entry.icon_source.as_str()]
            .into_iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&hidden.to_string_lossy()));
        let matches_name = hidden
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| entry.name.eq_ignore_ascii_case(name));
        matches_path || matches_name
    })
}

fn discover_start_menu_entries() -> Vec<ApplicationEntry> {
    let roots = [
        known_folder(&FOLDERID_Programs),
        known_folder(&FOLDERID_CommonPrograms),
    ];
    let mut entries = discover_entries(roots.into_iter().flatten());
    entries.extend(discover_apps_folder_entries());
    entries.extend(discover_desktop_web_apps());
    entries
}

fn discover_desktop_web_apps() -> Vec<ApplicationEntry> {
    let mut candidates = desktop_roots()
        .into_iter()
        .enumerate()
        .flat_map(|(root_index, root)| {
            supported_files(&root)
                .into_iter()
                .filter(|path| is_chromium_web_app_shortcut(path))
                .filter_map(move |path| {
                    let name = display_name(&path)?;
                    (!should_exclude(&name)).then_some((
                        name.to_lowercase(),
                        root_index,
                        name,
                        path,
                    ))
                })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        (&left.0, left.1, path_sort_key(&left.3)).cmp(&(
            &right.0,
            right.1,
            path_sort_key(&right.3),
        ))
    });
    candidates
        .into_iter()
        .map(|(_, _, name, path)| {
            let target = path.to_string_lossy().into_owned();
            ApplicationEntry::new(name, target.clone(), Some(target))
        })
        .collect()
}

fn desktop_roots() -> Vec<PathBuf> {
    let mut candidates = [
        known_folder(&FOLDERID_Desktop),
        known_folder(&FOLDERID_PublicDesktop),
        env::var_os("USERPROFILE").map(|profile| PathBuf::from(profile).join("Desktop")),
        env::var_os("PUBLIC").map(|public| PathBuf::from(public).join("Desktop")),
        env::var_os("OneDrive").map(|onedrive| PathBuf::from(onedrive).join("Desktop")),
        env::var_os("OneDriveConsumer")
            .map(|onedrive| PathBuf::from(onedrive).join("Desktop")),
        env::var_os("OneDriveCommercial")
            .map(|onedrive| PathBuf::from(onedrive).join("Desktop")),
    ]
    .into_iter()
    .flatten()
    .filter(|path| path.is_dir())
    .collect::<Vec<_>>();
    candidates.sort_by_key(|path| path_sort_key(path));
    candidates.dedup_by(|left, right| path_sort_key(left) == path_sort_key(right));
    candidates
}

fn is_chromium_web_app_shortcut(path: &Path) -> bool {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
    {
        return false;
    }

    let arguments = shortcut_arguments(path);
    let target = resolve_executable(&path.to_string_lossy());
    chromium_web_app_identity(arguments.as_deref(), target.as_deref())
}

fn chromium_web_app_identity(arguments: Option<&str>, target: Option<&Path>) -> bool {
    arguments.is_some_and(chromium_web_app_arguments)
        || target
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with("_proxy.exe"))
}

fn chromium_web_app_arguments(arguments: &str) -> bool {
    arguments.split_ascii_whitespace().any(|argument| {
        let argument = argument.trim_matches('"').to_ascii_lowercase();
        argument.starts_with("--app-id=") || argument.starts_with("--app=")
    })
}

fn discover_apps_folder_entries() -> Vec<ApplicationEntry> {
    let Some(_apartment) = ComApartment::enter() else {
        return Vec::new();
    };
    // SAFETY: COM is initialized for this thread and the returned shell item is
    // owned by windows-rs. No access token or non-default folder flags are used.
    let Ok(folder) = (unsafe {
        SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
    }) else {
        return Vec::new();
    };
    // SAFETY: `folder` is a live AppsFolder shell item and windows-rs owns the
    // returned enumerator interface.
    let Ok(enumerator) =
        (unsafe { folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) })
    else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    loop {
        let mut item = [None];
        let mut fetched = 0;
        // SAFETY: `item` and `fetched` are writable for this synchronous COM
        // call, and the enumerator owns any returned interface reference.
        if unsafe { enumerator.Next(&mut item, Some(&raw mut fetched)) }.is_err()
            || fetched == 0
        {
            break;
        }
        let Some(item) = item[0].take() else {
            continue;
        };
        let Some(name) = shell_item_text(&item, SIGDN_NORMALDISPLAY) else {
            continue;
        };
        let Some(identity) = shell_item_text(&item, SIGDN_PARENTRELATIVEPARSING) else {
            continue;
        };
        if let Some(entry) = apps_folder_entry(name, &identity) {
            entries.push(entry);
        }
    }
    entries
}

fn apps_folder_entry(name: String, identity: &str) -> Option<ApplicationEntry> {
    if should_exclude(&name)
        || identity.starts_with("http://")
        || identity.starts_with("https://")
    {
        return None;
    }
    let target = format!(r"shell:AppsFolder\{identity}");
    Some(ApplicationEntry::new(name, target.clone(), Some(target)))
}

fn shell_item_text(
    item: &IShellItem,
    format: windows::Win32::UI::Shell::SIGDN,
) -> Option<String> {
    // SAFETY: `item` is live and the returned task-allocated UTF-16 string is
    // transferred to the guard before conversion.
    let text = unsafe { item.GetDisplayName(format) }.ok()?;
    let text = CoTaskMemPath(text);
    // SAFETY: GetDisplayName returned a valid null-terminated UTF-16 allocation
    // which remains live through this conversion.
    let text = unsafe { text.0.to_string() }.ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn discover_entries(roots: impl IntoIterator<Item = PathBuf>) -> Vec<ApplicationEntry> {
    let mut candidates = Vec::new();
    for (root_index, root) in roots.into_iter().enumerate() {
        for path in supported_files(&root) {
            let Some(name) = display_name(&path) else {
                continue;
            };
            if should_exclude(&name) || !is_launchable_entry(&path) {
                continue;
            }
            candidates.push((
                catalog_priority(&path, &name),
                name.to_lowercase(),
                root_index,
                name,
                path,
            ));
        }
    }
    candidates.sort_by(|left, right| {
        (left.0, &left.1, left.2, path_sort_key(&left.4)).cmp(&(
            right.0,
            &right.1,
            right.2,
            path_sort_key(&right.4),
        ))
    });
    candidates
        .into_iter()
        .map(|(_, _, _, name, path)| {
            let target = path.to_string_lossy().into_owned();
            ApplicationEntry::new(name, target.clone(), Some(target))
        })
        .collect()
}

fn known_folder(folder: &GUID) -> Option<PathBuf> {
    // SAFETY: `folder` points to a live known-folder GUID, no access token is
    // supplied, and the returned task-allocated string is owned by the guard.
    let path = unsafe { SHGetKnownFolderPath(folder, KF_FLAG_DEFAULT, None) }.ok()?;
    let path = CoTaskMemPath(path);
    // SAFETY: SHGetKnownFolderPath returned a valid null-terminated UTF-16
    // allocation that remains live through this conversion.
    unsafe { path.0.to_string() }.ok().map(PathBuf::from)
}

struct CoTaskMemPath(PWSTR);

impl Drop for CoTaskMemPath {
    fn drop(&mut self) {
        // SAFETY: This pointer came from SHGetKnownFolderPath and is released
        // exactly once with its documented allocator.
        unsafe { CoTaskMemFree(Some(self.0.0.cast::<c_void>())) };
    }
}

fn supported_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();

    while let Some(directory) = pending.pop() {
        let Ok(children) = fs::read_dir(directory) else {
            continue;
        };
        for child in children.flatten() {
            let path = child.path();
            let Ok(kind) = child.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && is_supported(&path) {
                files.push(path);
            }
        }
    }

    files.sort_by(|left, right| {
        path_sort_key(left)
            .cmp(&path_sort_key(right))
            .then_with(|| left.as_os_str().cmp(right.as_os_str()))
    });
    files
}

fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["lnk", "url", "appref-ms", "exe"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn display_name(path: &Path) -> Option<String> {
    let name = path.file_stem()?.to_string_lossy();
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(
        match name.to_ascii_lowercase().as_str() {
            "administrative tools" => "Windows Tools",
            "dfrgui" => "Defragment and Optimize Drives",
            "livecaptions" => "Live Captions",
            "recoverydrive" => "Recovery Drive",
            "security configuration management" => "Local Security Policy",
            "services" => "Services",
            "voiceaccess" => "Voice Access",
            _ => name,
        }
        .to_owned(),
    )
}

fn should_exclude(name: &str) -> bool {
    let name = name.to_lowercase();
    name.starts_with("uninstall")
        || name.ends_with(" help")
        || name.ends_with(" readme")
        || name.ends_with(" manual")
        || name.ends_with(" support center")
        || name.ends_with(" website")
        || name.ends_with(" release notes")
        || name.starts_with("documentation for ")
        || name == "windows software development kit"
}

fn is_launchable_entry(path: &Path) -> bool {
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("url"))
    {
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("lnk"))
        {
            return resolve_executable(&path.to_string_lossy()).is_none_or(|target| {
                !target
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| {
                        ["url", "htm", "html", "chm"]
                            .iter()
                            .any(|web| value.eq_ignore_ascii_case(web))
                    })
            });
        }
        return true;
    }
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    contents
        .lines()
        .find_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            key.eq_ignore_ascii_case("url").then_some(value)
        })
        .is_some_and(|target| {
            let target = target.trim().to_ascii_lowercase();
            !target.starts_with("http://") && !target.starts_with("https://")
        })
}

fn catalog_priority(path: &Path, name: &str) -> u8 {
    const EVERYDAY_WINDOWS: [&str; 3] = ["accessories", "accessibility", "system tools"];
    const ADVANCED_TOOLS: [&str; 5] = [
        "administrative tools",
        "application verifier",
        "visual studio tools",
        "windows kits",
        "windows powershell",
    ];
    let folders = path
        .ancestors()
        .filter_map(|ancestor| ancestor.file_name()?.to_str())
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    if folders.iter().any(|folder| {
        ADVANCED_TOOLS
            .iter()
            .any(|advanced| folder.starts_with(advanced))
    }) {
        return 2;
    }
    u8::from(
        name == "Windows Tools"
            || folders
                .iter()
                .any(|folder| EVERYDAY_WINDOWS.iter().any(|everyday| folder == everyday)),
    )
}

fn path_sort_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::chromium_web_app_identity;

    #[test]
    fn chromium_web_app_shortcuts_accept_launch_switches_and_browser_proxies() {
        let cases = [
            (
                Some("--profile-directory=Default --app-id=abcdefghijkl"),
                Some(Path::new("chrome.exe")),
                true,
            ),
            (
                Some("--app=https://mail.proton.me/"),
                Some(Path::new("chrome.exe")),
                true,
            ),
            (None, Some(Path::new("chrome_proxy.exe")), true),
            (None, Some(Path::new("msedge_proxy.exe")), true),
            (
                Some("--profile-directory=Default"),
                Some(Path::new("chrome.exe")),
                false,
            ),
            (None, Some(Path::new("ordinary.exe")), false),
        ];

        for (arguments, target, expected) in cases {
            assert_eq!(chromium_web_app_identity(arguments, target), expected);
        }
    }
}
