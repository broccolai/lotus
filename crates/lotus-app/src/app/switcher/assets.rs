use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use lotus_core::application::ApplicationPresentationIcon;
use lotus_core::settings::DockSettings;
use lotus_core::window::{TrackedWindowKey, WindowInfo};
use lotus_switcher::model::SwitcherSession;
use lotus_ui::icon::RasterIcon;
use lotus_windows::icon_hydrator::{
    HydratedSwitcherIcon, SwitcherIconClient, SwitcherIconRequest,
};

use crate::app::applications::ApplicationView;
use crate::app::visuals::{DockIcon, SwitcherScene};

const MAX_RETAINED_SWITCHER_ICONS: usize = 128;

#[derive(Clone)]
struct RetainedSwitcherIcon {
    pixel_size: u32,
    settings_revision: u64,
    presentation_icon: Option<ApplicationPresentationIcon>,
    custom_image_path: Option<PathBuf>,
    icon: RasterIcon,
}

pub(super) struct SwitcherAssets {
    hydrator: SwitcherIconClient,
    pub(super) settings: DockSettings,
    generation: u64,
    settings_revision: u64,
    retained: BTreeMap<TrackedWindowKey, RetainedSwitcherIcon>,
}

impl SwitcherAssets {
    pub(super) fn new(hydrator: SwitcherIconClient, settings: &DockSettings) -> Self {
        Self {
            hydrator,
            settings: settings.clone(),
            generation: 0,
            settings_revision: 0,
            retained: BTreeMap::new(),
        }
    }

    pub(super) fn apply_settings(&mut self, settings: &DockSettings) {
        self.settings = settings.clone();
        self.settings_revision = self.settings_revision.wrapping_add(1);
        self.retained.clear();
    }

    pub(super) fn next_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn cancel(&self) {
        self.hydrator.request_switcher(Vec::new());
    }

    pub(super) fn retain_live_windows(&mut self, windows: &BTreeSet<TrackedWindowKey>) {
        self.retained.retain(|window, _| windows.contains(window));
    }

    pub(super) fn icon(
        &self,
        key: TrackedWindowKey,
        pixel_size: u32,
        presentation: Option<&ApplicationPresentationIcon>,
        custom: Option<&PathBuf>,
    ) -> Option<DockIcon> {
        self.retained
            .get(&key)
            .filter(|icon| {
                icon.matches(pixel_size, self.settings_revision, presentation, custom)
            })
            .map(|icon| DockIcon::Raster(icon.icon.clone()))
    }

    pub(super) fn request_visible(
        &self,
        session: Option<&SwitcherSession<WindowInfo>>,
        scene: Option<&SwitcherScene>,
        applications: &ApplicationView,
    ) {
        let (Some(session), Some(scene)) = (session, scene) else {
            self.hydrator.request_switcher(Vec::new());
            return;
        };
        let pixel_size = super::sampled_icon_size(scene.dpi());
        let requests = scene
            .visible_range_with_margin(2)
            .filter_map(|index| {
                let window = session.items().get(index)?;
                let (presentation_icon, custom_image_path) =
                    super::switcher_icon_sources(window, &self.settings, applications);
                if self.retained.get(&window.key()).is_some_and(|icon| {
                    icon.matches(
                        pixel_size,
                        self.settings_revision,
                        presentation_icon.as_ref(),
                        custom_image_path.as_ref(),
                    )
                }) {
                    return None;
                }
                Some(SwitcherIconRequest {
                    generation: self.generation,
                    window: window.key(),
                    executable_path: window.executable_path.clone(),
                    presentation_icon,
                    custom_image_path,
                    pixel_size,
                    settings_revision: self.settings_revision,
                })
            })
            .collect();
        self.hydrator.request_switcher(requests);
    }

    pub(super) fn drain(
        &mut self,
        results: impl IntoIterator<Item = HydratedSwitcherIcon>,
        scene: &mut Option<SwitcherScene>,
    ) -> bool {
        let mut changed = false;
        for result in results {
            if result.settings_revision != self.settings_revision {
                continue;
            }
            let Some(icon) = result.icon else {
                continue;
            };
            self.retained.insert(
                result.window,
                RetainedSwitcherIcon {
                    pixel_size: result.pixel_size,
                    settings_revision: result.settings_revision,
                    presentation_icon: result.presentation_icon,
                    custom_image_path: result.custom_image_path,
                    icon: icon.clone(),
                },
            );
            if self.retained.len() > MAX_RETAINED_SWITCHER_ICONS {
                let _ = self.retained.pop_first();
            }
            if result.generation == self.generation
                && let Some(scene) = scene
                && super::sampled_icon_size(scene.dpi()) == result.pixel_size
            {
                changed |= scene.set_icon(result.window, Some(DockIcon::Raster(icon)));
            }
        }
        changed
    }
}

impl RetainedSwitcherIcon {
    fn matches(
        &self,
        pixel_size: u32,
        settings_revision: u64,
        presentation: Option<&ApplicationPresentationIcon>,
        custom: Option<&PathBuf>,
    ) -> bool {
        self.pixel_size == pixel_size
            && self.settings_revision == settings_revision
            && self.presentation_icon.as_ref() == presentation
            && self.custom_image_path.as_ref() == custom
    }
}
