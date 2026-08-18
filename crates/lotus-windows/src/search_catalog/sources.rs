use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::{env, fs};

use lotus_core::search::ApplicationEntry;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_CommonPrograms, FOLDERID_Desktop,
    FOLDERID_Programs, FOLDERID_PublicDesktop, IEnumShellItems, IShellItem,
    KF_FLAG_DEFAULT, SHGetKnownFolderItem, SHGetKnownFolderPath, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING,
};
use windows::core::{GUID, PWSTR};

use super::super::launch::{ComApartment, resolve_executable};
use super::shortcuts::{is_chromium_web_app_shortcut, shortcut_entry};

pub(super) fn discover_start_menu_entries() -> Vec<ApplicationEntry> {
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

fn discover_apps_folder_entries() -> Vec<ApplicationEntry> {
    let Some(_apartment) = ComApartment::enter() else {
        return Vec::new();
    };
    let Ok(folder) = (unsafe {
        SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
    }) else {
        return Vec::new();
    };
    let Ok(enumerator) =
        (unsafe { folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) })
    else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    loop {
        let mut item = [None];
        let mut fetched = 0;
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
    Some(
        ApplicationEntry::new(name, target.clone(), Some(target))
            .with_app_user_model_id(identity),
    )
}

fn shell_item_text(
    item: &IShellItem,
    format: windows::Win32::UI::Shell::SIGDN,
) -> Option<String> {
    let text = unsafe { item.GetDisplayName(format) }.ok()?;
    let text = CoTaskMemPath(text);
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
        .map(|(_, _, _, name, path)| shortcut_entry(name, &path))
        .collect()
}

fn known_folder(folder: &GUID) -> Option<PathBuf> {
    let path = unsafe { SHGetKnownFolderPath(folder, KF_FLAG_DEFAULT, None) }.ok()?;
    let path = CoTaskMemPath(path);
    unsafe { path.0.to_string() }.ok().map(PathBuf::from)
}

struct CoTaskMemPath(PWSTR);

impl Drop for CoTaskMemPath {
    fn drop(&mut self) {
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
