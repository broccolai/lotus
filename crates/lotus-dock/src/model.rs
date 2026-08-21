use std::path::Path;

use lotus_core::activation::{ActivationDecision, decide_activation};
use lotus_core::application::{
    ApplicationIdentity, is_reliable_application_identity, is_shared_host_executable,
};
use lotus_core::dock::DockItem;
use lotus_core::settings::{DockSettings, PinnedApp, SettingsStore, SettingsStoreError};
use lotus_core::window::{WindowId, WindowInfo};

pub fn project_snapshot<F>(
    settings: &DockSettings,
    windows: &[WindowInfo],
    resolve_executable: F,
) -> Vec<DockItem>
where
    F: FnMut(&str) -> Option<String>,
{
    lotus_core::dock::project_dock(
        windows,
        lotus_core::dock::DockProjection {
            pinned_apps: &settings.pinned_apps,
            hidden_executables: &settings.hidden_executables,
            item_order: &settings.item_order,
            show_unpinned_running_apps: settings.show_unpinned_running_apps,
        },
        resolve_executable,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsImpact {
    pub changed: bool,
    pub restart_required: bool,
}

pub struct DockModel {
    settings: DockSettings,
    settings_store: SettingsStore,
    items: Vec<DockItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinLaunch {
    pub id: String,
    pub name: String,
    pub target: String,
    pub arguments: Option<String>,
    pub icon_source: Option<String>,
    pub app_user_model_id: Option<String>,
}

pub struct PinUpgrade {
    pub current_id: String,
    pub launch: PinLaunch,
}

pub struct PinExecutableAlias {
    pub registered_id: String,
    pub app_user_model_id: Option<String>,
    pub executable_name: String,
}

impl DockModel {
    pub const fn new(
        settings: DockSettings,
        settings_store: SettingsStore,
        items: Vec<DockItem>,
    ) -> Self {
        Self {
            settings,
            settings_store,
            items,
        }
    }

    pub const fn settings(&self) -> &DockSettings {
        &self.settings
    }

    pub fn settings_directory(&self) -> &Path {
        self.settings_store.directory()
    }

    pub fn items(&self) -> &[DockItem] {
        &self.items
    }

    pub fn rebuild(&mut self, items: Vec<DockItem>) {
        self.items = items;
    }

    pub fn apply_settings(
        &mut self,
        next: DockSettings,
        items: Vec<DockItem>,
    ) -> Result<SettingsImpact, SettingsStoreError> {
        let next = next.normalized();
        if self.settings == next {
            return Ok(SettingsImpact {
                changed: false,
                restart_required: false,
            });
        }

        let previous = self.settings.clone();
        self.settings_store.save(&next)?;
        self.settings = next;
        self.items = items;

        Ok(SettingsImpact {
            changed: true,
            restart_required: restart_required(&previous, &self.settings),
        })
    }

    pub fn persist_reorder(
        &mut self,
        source_index: usize,
        insertion_slot: usize,
    ) -> Result<bool, SettingsStoreError> {
        let Some(destination) =
            insertion_destination(self.items.len(), source_index, insertion_slot)
        else {
            return Ok(false);
        };
        if source_index == destination {
            return Ok(false);
        }

        let mut reordered = self.items.clone();
        let moved = reordered.remove(source_index);
        reordered.insert(destination, moved);
        let mut settings = self.settings.clone();
        let (target_index, insert_after) = if insertion_slot == self.items.len() {
            (self.items.len() - 1, true)
        } else {
            (insertion_slot, false)
        };
        let source_id = &self.items[source_index].id;
        let target_id = &self.items[target_index].id;
        let mut full_order =
            Vec::with_capacity(settings.item_order.len() + self.items.len());
        for id in settings
            .item_order
            .iter()
            .chain(self.items.iter().map(|item| &item.id))
        {
            if !full_order
                .iter()
                .any(|saved: &String| saved.eq_ignore_ascii_case(id))
            {
                full_order.push(id.clone());
            }
        }
        full_order.retain(|id| !id.eq_ignore_ascii_case(source_id));
        let Some(target_position) = full_order
            .iter()
            .position(|id| id.eq_ignore_ascii_case(target_id))
        else {
            return Ok(false);
        };
        full_order.insert(
            target_position + usize::from(insert_after),
            source_id.clone(),
        );
        settings.item_order = full_order;
        self.settings_store.save(&settings)?;

        self.settings = settings;
        self.items = reordered;
        Ok(true)
    }

    pub fn activation(
        &self,
        source_index: usize,
        foreground: Option<&WindowId>,
    ) -> Option<(ActivationDecision<WindowId>, &DockItem)> {
        let item = self.items.get(source_index)?;
        let windows = item
            .windows
            .iter()
            .map(|window| window.id)
            .collect::<Vec<_>>();
        Some((decide_activation(&windows, foreground), item))
    }

    pub fn set_pinned(
        &mut self,
        source_index: usize,
        pinned: bool,
        launch: Option<PinLaunch>,
    ) -> Result<bool, SettingsStoreError> {
        let Some(item) = self.items.get(source_index) else {
            return Ok(false);
        };
        if item.is_pinned == pinned {
            return Ok(false);
        }

        let mut settings = self.settings.clone();
        if pinned {
            let launch = launch.unwrap_or_else(|| PinLaunch {
                id: item.id.clone(),
                name: item.display_name.clone(),
                target: item.launch_target.clone(),
                arguments: item.arguments.clone(),
                icon_source: Some(item.icon_source.clone()),
                app_user_model_id: item
                    .windows
                    .first()
                    .and_then(|window| window.app_user_model_id.clone()),
            });
            if settings.pinned_apps.iter().any(|pin| {
                pin.application_identity(None)
                    .match_strength(&launch.identity())
                    .is_match()
            }) {
                return Ok(false);
            }
            settings.pinned_apps.push(PinnedApp {
                id: launch.id,
                name: launch.name,
                launch_target: launch.target,
                arguments: launch.arguments,
                icon_source: launch.icon_source,
                app_user_model_id: launch.app_user_model_id,
                match_executables: executable_alias(&item.executable_path)
                    .into_iter()
                    .collect(),
            });
            insert_item_order(&mut settings.item_order, &self.items, source_index);
        } else {
            settings
                .pinned_apps
                .retain(|pin| !pin.id.eq_ignore_ascii_case(&item.id));
        }

        let settings = settings.normalized();
        self.settings_store.save(&settings)?;
        self.settings = settings;
        Ok(true)
    }

    pub fn upgrade_pins(
        &mut self,
        upgrades: Vec<PinUpgrade>,
    ) -> Result<bool, SettingsStoreError> {
        if upgrades.is_empty() {
            return Ok(false);
        }

        let mut settings = self.settings.clone();
        let mut changed = false;
        for upgrade in upgrades {
            let launch = upgrade.launch.clone();
            let Some(pin) = settings
                .pinned_apps
                .iter_mut()
                .find(|pin| pin.id.eq_ignore_ascii_case(&upgrade.current_id))
            else {
                continue;
            };
            let previous_id = pin.id.clone();
            let previous_target = pin.launch_target.clone();
            if pin.id.eq_ignore_ascii_case(&upgrade.launch.id)
                && pin.name == upgrade.launch.name
                && pin.launch_target == upgrade.launch.target
                && pin.arguments == upgrade.launch.arguments
                && pin.icon_source == upgrade.launch.icon_source
                && pin.app_user_model_id == upgrade.launch.app_user_model_id
            {
                continue;
            }

            replace_order_identity(&mut settings.item_order, &pin.id, &upgrade.launch.id);
            pin.id = upgrade.launch.id;
            pin.name = upgrade.launch.name;
            pin.launch_target = upgrade.launch.target;
            pin.arguments = upgrade.launch.arguments;
            pin.icon_source = upgrade.launch.icon_source;
            pin.app_user_model_id = upgrade.launch.app_user_model_id;
            migrate_icon_overrides(&mut settings, &previous_id, &previous_target, &launch);
            changed = true;
        }
        if !changed {
            return Ok(false);
        }

        let settings = settings.normalized();
        self.settings_store.save(&settings)?;
        self.settings = settings;
        Ok(true)
    }

    pub fn reconcile_pin_executables(
        &mut self,
        aliases: Vec<PinExecutableAlias>,
    ) -> Result<bool, SettingsStoreError> {
        if aliases.is_empty() {
            return Ok(false);
        }

        let mut settings = self.settings.clone();
        let mut changed = false;
        for alias in aliases {
            if is_shared_host_executable(&alias.executable_name) {
                continue;
            }
            let pin_identity =
                if let Some(pin) = pin_for_registered_application(&mut settings, &alias) {
                    let previous = (pin.id.clone(), pin.launch_target.clone());
                    if let Some(identity) = registered_alias_identity(&alias)
                        && pin.app_user_model_id.as_deref() != Some(identity)
                    {
                        pin.app_user_model_id = Some(identity.to_owned());
                        changed = true;
                    }
                    if !pin
                        .match_executables
                        .iter()
                        .any(|saved| saved.eq_ignore_ascii_case(&alias.executable_name))
                    {
                        pin.match_executables.push(alias.executable_name.clone());
                        changed = true;
                    }
                    Some(previous)
                } else {
                    None
                };
            reconcile_icon_override_alias(
                &mut settings,
                &alias,
                pin_identity.as_ref(),
                &mut changed,
            );
        }
        if !changed {
            return Ok(false);
        }

        let settings = settings.normalized();
        self.settings_store.save(&settings)?;
        self.settings = settings;
        Ok(true)
    }
}

fn reconcile_icon_override_alias(
    settings: &mut DockSettings,
    alias: &PinExecutableAlias,
    legacy_pin: Option<&(String, String)>,
    changed: &mut bool,
) {
    if is_shared_host_executable(&alias.executable_name) {
        return;
    }
    let identity = registered_alias_identity(alias);
    let Some(identity) = identity else {
        return;
    };
    let Some(override_) =
        settings
            .application_icon_overrides
            .iter_mut()
            .find(|override_| {
                override_
                    .app_user_model_id
                    .as_deref()
                    .is_some_and(|saved| saved.eq_ignore_ascii_case(identity))
                    || override_.id.eq_ignore_ascii_case(identity)
                    || legacy_pin.is_some_and(|(id, target)| {
                        override_.id.eq_ignore_ascii_case(id)
                            || override_.id.eq_ignore_ascii_case(target)
                    })
            })
    else {
        return;
    };
    if override_.app_user_model_id.as_deref() != Some(identity) {
        override_.app_user_model_id = Some(identity.to_owned());
        *changed = true;
    }
    if override_
        .match_executables
        .iter()
        .any(|saved| saved.eq_ignore_ascii_case(&alias.executable_name))
    {
        return;
    }
    override_
        .match_executables
        .push(alias.executable_name.clone());
    *changed = true;
}

fn registered_alias_identity(alias: &PinExecutableAlias) -> Option<&str> {
    alias
        .app_user_model_id
        .as_deref()
        .filter(|identity| is_reliable_application_identity(identity))
        .or_else(|| {
            is_reliable_application_identity(&alias.registered_id)
                .then_some(alias.registered_id.as_str())
        })
}

fn migrate_icon_overrides(
    settings: &mut DockSettings,
    previous_id: &str,
    previous_target: &str,
    launch: &PinLaunch,
) {
    let app_user_model_id = launch
        .app_user_model_id
        .as_deref()
        .filter(|identity| is_reliable_application_identity(identity))
        .map(str::to_owned);
    let stable_id = app_user_model_id.as_deref().unwrap_or(&launch.id);
    for override_ in &mut settings.application_icon_overrides {
        let previous = override_.id.eq_ignore_ascii_case(previous_id)
            || override_.id.eq_ignore_ascii_case(previous_target);
        if !previous {
            continue;
        }
        stable_id.clone_into(&mut override_.id);
        override_.app_user_model_id.clone_from(&app_user_model_id);
        override_
            .match_executables
            .retain(|executable| !is_shared_host_executable(executable));
    }
}

fn pin_for_registered_application<'a>(
    settings: &'a mut DockSettings,
    alias: &PinExecutableAlias,
) -> Option<&'a mut PinnedApp> {
    let candidates = [
        Some(alias.registered_id.as_str()),
        alias.app_user_model_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|identity| is_reliable_application_identity(identity))
    .map(|identity| {
        ApplicationIdentity::new(
            Some(identity),
            Some(identity),
            None,
            std::iter::once(alias.executable_name.as_str()),
        )
    })
    .collect::<Vec<_>>();
    let strict = settings.pinned_apps.iter().position(|pin| {
        candidates.iter().any(|candidate| {
            pin.application_identity(None)
                .match_strength(candidate)
                .is_match()
        })
    });
    let index = strict.or_else(|| legacy_pin_alias(settings, alias))?;
    settings.pinned_apps.get_mut(index)
}

fn legacy_pin_alias(settings: &DockSettings, alias: &PinExecutableAlias) -> Option<usize> {
    if is_shared_host_executable(&alias.executable_name) {
        return None;
    }

    let matches = settings
        .pinned_apps
        .iter()
        .enumerate()
        .filter(|(_, pin)| {
            pin.application_identity(None)
                .reliable_registered_id()
                .is_none()
                && pin
                    .application_identity(None)
                    .has_executable_alias(&alias.executable_name)
        })
        .map(|(index, _)| index)
        .take(2)
        .collect::<Vec<_>>();
    let [index] = matches.as_slice() else {
        return None;
    };
    Some(*index)
}

impl PinLaunch {
    fn identity(&self) -> ApplicationIdentity {
        ApplicationIdentity::new(
            self.app_user_model_id.as_deref(),
            Some(&self.id),
            Some(&self.target),
            std::iter::empty(),
        )
    }
}

fn executable_alias(path: &str) -> Option<String> {
    let executable = path.rsplit(['\\', '/']).next()?;
    (!is_shared_host_executable(executable) && !executable.is_empty())
        .then(|| executable.into())
}

fn replace_order_identity(order: &mut [String], previous: &str, current: &str) {
    for identity in order {
        if identity.eq_ignore_ascii_case(previous) {
            current.clone_into(identity);
        }
    }
}

fn insert_item_order(order: &mut Vec<String>, items: &[DockItem], source_index: usize) {
    let id = &items[source_index].id;
    if order.iter().any(|saved| saved.eq_ignore_ascii_case(id)) {
        return;
    }
    let next = items
        .iter()
        .skip(source_index + 1)
        .find_map(|item| {
            order
                .iter()
                .position(|saved| saved.eq_ignore_ascii_case(&item.id))
        })
        .unwrap_or(order.len());
    order.insert(next, id.clone());
}

fn restart_required(previous: &DockSettings, current: &DockSettings) -> bool {
    previous.replace_windows_taskbar != current.replace_windows_taskbar
        || previous.exclusive_taskbar_replacement != current.exclusive_taskbar_replacement
        || previous.search_enabled != current.search_enabled
        || previous.search_open_with_windows_key != current.search_open_with_windows_key
        || previous.alt_tab_enabled != current.alt_tab_enabled
        || previous.notification_badge_style != current.notification_badge_style
        || (current.replace_windows_taskbar
            && (previous.icon_size != current.icon_size
                || previous.vertical_padding != current.vertical_padding
                || previous.bottom_offset != current.bottom_offset))
}

fn insertion_destination(
    item_count: usize,
    source_index: usize,
    insertion_slot: usize,
) -> Option<usize> {
    if insertion_slot > item_count || item_count == 0 {
        return None;
    }
    let (target_index, insert_after) = if insertion_slot == item_count {
        (item_count - 1, true)
    } else {
        (insertion_slot, false)
    };
    lotus_core::reorder::destination_index(
        item_count,
        source_index,
        target_index,
        insert_after,
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use lotus_core::settings::{ApplicationIconOverride, PinnedApp};
    use lotus_core::window::{WindowId, WindowInfo};

    use super::*;

    #[test]
    fn reconciliation_absorbs_a_registered_application_with_a_changed_executable()
    -> Result<(), Box<dyn Error>> {
        let directory = std::env::temp_dir().join(format!(
            "lotus-dock-model-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let store = SettingsStore::new(&directory);
        let mut settings = DockSettings::default();
        settings.pinned_apps.push(PinnedApp {
            id: "com.squirrel.discord.discord".into(),
            name: "Discord".into(),
            launch_target: r"C:\ProgramData\Microsoft\Windows\Start Menu\Discord.lnk"
                .into(),
            arguments: None,
            icon_source: None,
            app_user_model_id: Some("com.squirrel.discord.discord".into()),
            match_executables: vec!["Discord.exe".into()],
        });
        settings.pinned_apps.push(PinnedApp {
            id: "com.electron.app".into(),
            name: "Generic Electron App".into(),
            launch_target: r"C:\Apps\GenericElectron.exe".into(),
            arguments: None,
            icon_source: None,
            app_user_model_id: Some("com.electron.app".into()),
            match_executables: vec!["GenericElectron.exe".into()],
        });
        settings
            .application_icon_overrides
            .push(ApplicationIconOverride {
                id: "com.squirrel.discord.discord".into(),
                image_path: r"C:\Lotus\assets\app-icons\discord.png".into(),
                app_user_model_id: Some("com.squirrel.discord.discord".into()),
                match_executables: vec!["Discord.exe".into()],
            });
        let mut model = DockModel::new(settings, store, Vec::new());

        assert!(model.reconcile_pin_executables(vec![
            PinExecutableAlias {
                registered_id: "COM.SQUIRREL.DISCORD.DISCORD".into(),
                app_user_model_id: Some("com.squirrel.discord.discord".into()),
                executable_name: "DiscordCanary.exe".into(),
            },
            PinExecutableAlias {
                registered_id: "com.electron.app".into(),
                app_user_model_id: Some("com.electron.app".into()),
                executable_name: "AnotherElectron.exe".into(),
            },
            PinExecutableAlias {
                registered_id: "COM.SQUIRREL.DISCORD.DISCORD".into(),
                app_user_model_id: Some("com.squirrel.discord.discord".into()),
                executable_name: "msedge.exe".into(),
            },
        ])?);
        assert_eq!(
            model.settings().pinned_apps[0].match_executables,
            ["Discord.exe", "DiscordCanary.exe"]
        );
        assert_eq!(
            SettingsStore::new(&directory).load()?.settings.pinned_apps[0]
                .match_executables,
            ["Discord.exe", "DiscordCanary.exe"]
        );
        assert_eq!(
            model.settings().pinned_apps[1].match_executables,
            ["GenericElectron.exe"]
        );
        assert!(
            model.upgrade_pins(vec![PinUpgrade {
                current_id: "com.squirrel.discord.discord".into(),
                launch: PinLaunch {
                    id: "vendor.discord.stable".into(),
                    name: "Discord".into(),
                    target:
                        r"C:\ProgramData\Microsoft\Windows\Start Menu\Discord Stable.lnk"
                            .into(),
                    arguments: None,
                    icon_source: None,
                    app_user_model_id: Some("vendor.discord.stable".into()),
                },
            }])?
        );
        assert_migrated_icon_override(model.settings());
        let persisted = SettingsStore::new(&directory).load()?.settings;
        assert_migrated_icon_override(&persisted);

        let windows = [WindowInfo {
            id: WindowId::new(1),
            process_id: 1,
            title: "Discord".into(),
            executable_path:
                r"C:\Users\someone\AppData\Local\DiscordCanary\DiscordCanary.exe".into(),
            app_user_model_id: None,
        }];
        let items = project_snapshot(model.settings(), &windows, |_| None);
        assert!(items.iter().all(|item| item.is_pinned));
        assert_eq!(
            items.iter().filter(|item| item.windows == windows).count(),
            1
        );

        let generic_window = [WindowInfo {
            id: WindowId::new(2),
            process_id: 2,
            title: "Another Electron App".into(),
            executable_path: r"C:\Apps\AnotherElectron.exe".into(),
            app_user_model_id: Some("com.electron.app".into()),
        }];
        assert!(
            project_snapshot(model.settings(), &generic_window, |_| None)
                .iter()
                .any(|item| !item.is_pinned)
        );

        fs::remove_dir_all(directory)?;
        Ok(())
    }

    fn assert_migrated_icon_override(settings: &DockSettings) {
        let override_ = &settings.application_icon_overrides[0];
        assert_eq!(override_.id, "vendor.discord.stable");
        assert_eq!(
            override_.app_user_model_id.as_deref(),
            Some("vendor.discord.stable")
        );
        assert_eq!(
            override_.image_path,
            r"C:\Lotus\assets\app-icons\discord.png"
        );
    }
}
