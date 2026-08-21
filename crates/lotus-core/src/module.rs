use crate::settings::DockSettings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleId {
    Dock,
    Search,
    AltTab,
    Media,
    Status,
    ControlCenter,
}

impl ModuleId {
    pub const ALL: [Self; 6] = [
        Self::Dock,
        Self::Search,
        Self::AltTab,
        Self::Media,
        Self::Status,
        Self::ControlCenter,
    ];

    const fn mask(self) -> u8 {
        match self {
            Self::Dock => 1 << 0,
            Self::Search => 1 << 1,
            Self::AltTab => 1 << 2,
            Self::Media => 1 << 3,
            Self::Status => 1 << 4,
            Self::ControlCenter => 1 << 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModuleSet(u8);

impl ModuleSet {
    pub fn from_settings(settings: &DockSettings) -> Self {
        let mut modules = Self::default();
        modules.set(ModuleId::Dock, settings.show_app_dock);
        modules.set(ModuleId::Search, settings.search_enabled);
        modules.set(ModuleId::AltTab, settings.alt_tab_enabled);
        modules.set(ModuleId::Media, settings.show_media_controls);
        modules.set(ModuleId::Status, settings.show_system_status);
        modules.set(
            ModuleId::ControlCenter,
            settings.show_system_status && settings.show_background_apps_status,
        );
        modules
    }

    pub const fn contains(self, module: ModuleId) -> bool {
        self.0 & module.mask() != 0
    }

    pub fn set(&mut self, module: ModuleId, enabled: bool) {
        if enabled {
            self.0 |= module.mask();
        } else {
            self.0 &= !module.mask();
        }
    }
}
