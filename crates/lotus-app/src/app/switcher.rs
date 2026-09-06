use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use lotus_core::application::{
    ApplicationIdentity, ApplicationKey, ApplicationPresentationIcon,
    ApplicationResolution, WindowApplicationAssignments,
};
use lotus_core::settings::DockSettings;
use lotus_core::window::{TrackedWindowKey, WindowInfo};
use lotus_settings::appearance::theme_for;
use lotus_switcher::model::{RecentOrder, ReconcileOutcome, SwitcherSession};
use lotus_ui::frame::ScheduledSurface;
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_ui::theme::Theme;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::DeviceState;
use lotus_windows::graphics::switcher_surface::SwitcherCompositionSurfaceState;
use lotus_windows::icon_hydrator::SwitcherIconClient;
use lotus_windows::interaction::PointerCursor;
use lotus_windows::window::{SwitcherEvent, SwitcherWindow};

use crate::app::applications::ApplicationView;
use crate::app::visuals::{SwitcherHitTarget, SwitcherItem, SwitcherScene};
use crate::app::{AppError, activation};

mod assets;
mod surface;

use self::assets::SwitcherAssets;

const SWITCHER_ICON_DIP: u32 = 38;
const NATIVE_ICON_SAMPLE_SCALE: u32 = 2;

pub(super) struct SwitcherRuntime {
    window: SwitcherWindow,
    surface: Option<ScheduledSurface<SwitcherCompositionSurfaceState>>,
    presentation_ready: bool,
    scene: Option<SwitcherScene>,
    session: Option<SwitcherSession<WindowInfo>>,
    assets: SwitcherAssets,
    name_overrides: BTreeMap<String, String>,
    applications: Arc<ApplicationView>,
    recent_windows: RecentOrder<TrackedWindowKey>,
    theme: Theme,
}

impl SwitcherRuntime {
    pub(super) fn new(
        window: SwitcherWindow,
        settings: &DockSettings,
        theme: &Theme,
        icon_hydrator: SwitcherIconClient,
        applications: Arc<ApplicationView>,
    ) -> Self {
        Self {
            window,
            surface: None,
            presentation_ready: false,
            scene: None,
            session: None,
            assets: SwitcherAssets::new(icon_hydrator, settings),
            name_overrides: BTreeMap::new(),
            applications,
            recent_windows: RecentOrder::default(),
            theme: *theme,
        }
    }

    pub(super) fn begin(
        &mut self,
        direction: lotus_switcher::model::Direction,
        foreground: Option<lotus_core::window::WindowId>,
        windows: &[WindowInfo],
        settings: &DockSettings,
        applications: Arc<ApplicationView>,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let windows = windows
            .iter()
            .filter(|window| !executable_is_hidden(window, &settings.hidden_executables))
            .cloned()
            .collect::<Vec<_>>();
        self.record_foreground(foreground.and_then(|id| {
            windows
                .iter()
                .find(|window| window.id == id)
                .map(WindowInfo::key)
        }));
        let windows = self.recent_windows.arrange(windows, WindowInfo::key);
        let Some(session) = SwitcherSession::begin(windows, direction) else {
            return Ok(());
        };
        if self.surface.is_none() && graphics.ready().is_none() {
            return Err(AppError::GraphicsUnavailable);
        }
        self.name_overrides = settings.application_name_overrides.clone();
        self.applications = applications;
        self.assets.settings = settings.clone();
        self.theme = theme_for(settings);
        self.session = Some(session);
        self.assets.next_generation();
        self.rebuild_scene(self.window.dpi())?;
        self.show_centered_for_scene(foreground)?;
        self.ensure_surface(graphics)?;
        self.request_visible_icons();
        self.invalidate();
        Ok(())
    }

    pub(super) fn record_foreground(&mut self, foreground: Option<TrackedWindowKey>) {
        if let Some(foreground) = foreground {
            self.recent_windows.record(foreground);
        }
    }

    pub(super) fn owns_window(&self, window: lotus_windows::WindowHandle) -> bool {
        self.window.handle() == window
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.window.has_pending_events()
    }

    pub(super) fn reconcile_windows(
        &mut self,
        windows: &[WindowInfo],
        applications: Arc<ApplicationView>,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.applications = applications;
        let live_windows = windows.iter().map(WindowInfo::key).collect::<BTreeSet<_>>();
        self.assets.retain_live_windows(&live_windows);
        self.recent_windows
            .retain(windows.iter().map(WindowInfo::key));
        let Some(session) = &mut self.session else {
            return Ok(());
        };
        let latest = windows
            .iter()
            .filter(|window| {
                !executable_is_hidden(window, &self.assets.settings.hidden_executables)
            })
            .cloned()
            .collect::<Vec<_>>();
        let outcome = session.reconcile(&latest, WindowInfo::key);
        match outcome {
            ReconcileOutcome::Unchanged => {}
            ReconcileOutcome::Empty { removed } => {
                lotus_windows::diagnostics::record_diagnostic(
                    "activation.switcher_entries_pruned",
                    &format!("{removed} Alt+Tab entries disappeared before commit"),
                );
                self.hide();
            }
            ReconcileOutcome::Pruned { removed } => {
                lotus_windows::diagnostics::record_diagnostic(
                    "activation.switcher_entries_pruned",
                    &format!(
                        "{removed} Alt+Tab entries disappeared during an active session"
                    ),
                );
                self.assets.next_generation();
                self.rebuild_scene(self.window.dpi())?;
                self.recenter_visible_window()?;
                self.ensure_surface(graphics)?;
                self.request_visible_icons();
                self.invalidate();
            }
            ReconcileOutcome::Refreshed => {
                self.assets.next_generation();
                self.rebuild_scene(self.window.dpi())?;
                self.recenter_visible_window()?;
                self.ensure_surface(graphics)?;
                self.request_visible_icons();
                self.invalidate();
            }
        }
        Ok(())
    }

    fn recenter_visible_window(&mut self) -> Result<(), AppError> {
        let foreground = lotus_windows::activation::foreground_window();
        self.show_centered_for_scene(foreground)
    }

    fn show_centered_for_scene(
        &mut self,
        foreground: Option<lotus_core::window::WindowId>,
    ) -> Result<(), AppError> {
        let size = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidSwitcherScene)?
            .desired_size();
        let dpi = self.window.show_centered(foreground, size)?;
        let current_dpi = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidSwitcherScene)?
            .dpi();
        if dpi != current_dpi {
            self.rebuild_scene(dpi)?;
            let size = self
                .scene
                .as_ref()
                .ok_or(AppError::InvalidSwitcherScene)?
                .desired_size();
            let _ = self.window.show_centered(foreground, size)?;
        }
        Ok(())
    }

    pub(super) fn cycle_by(&mut self, delta: i32) {
        let Some(session) = &mut self.session else {
            return;
        };
        session.cycle_by(delta);
        if let Some(scene) = &mut self.scene {
            let _changed = scene.set_selected(session.selected_index());
        }
        self.request_visible_icons();
        self.invalidate();
    }

    pub(super) fn commit(&mut self) {
        let selected = self
            .session
            .as_ref()
            .map(|session| session.selected().key());
        self.hide();
        if let Some(selected) = selected {
            match activation::activate_exact(selected) {
                Ok(outcome) => {
                    if let Some(key) = outcome.focused_key() {
                        self.recent_windows.record(key);
                    } else if matches!(
                        outcome,
                        activation::ActivationOutcome::ForegroundDenied
                    ) {
                        lotus_windows::diagnostics::record_diagnostic(
                            "activation.switcher_foreground_denied",
                            "Windows denied the committed Alt+Tab foreground change",
                        );
                    }
                }
                Err(error) => {
                    lotus_windows::diagnostics::record_error(
                        "alt_tab.switch_window",
                        &error,
                    );
                }
            }
        }
    }

    pub(super) fn hide(&mut self) {
        self.window.hide();
        self.scene = None;
        self.session = None;
    }

    pub(super) fn abandon(&mut self) {
        self.window.hide();
        self.surface = None;
        self.presentation_ready = false;
        self.assets.cancel();
        self.scene = None;
        self.session = None;
    }

    pub(super) fn drain_events_up_to(&mut self, limit: usize) -> Vec<SwitcherEvent> {
        self.window.drain_events_up_to(limit).collect()
    }

    pub(super) fn handle_window_event(
        &mut self,
        event: SwitcherEvent,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        match event {
            SwitcherEvent::CloseRequested => self.hide(),
            SwitcherEvent::PointerMoved { x, y } => {
                let Some(scene) = &mut self.scene else {
                    return Ok(());
                };
                let target = scene.hit_test(x, y);
                self.window.set_pointer_cursor(
                    if matches!(target, Some(SwitcherHitTarget::Close(_))) {
                        PointerCursor::Hand
                    } else {
                        PointerCursor::Arrow
                    },
                );
                if scene.pointer_move(x, y) {
                    self.invalidate();
                }
            }
            SwitcherEvent::PointerLeft => {
                self.window.set_pointer_cursor(PointerCursor::Arrow);
                if self.scene.as_mut().is_some_and(SwitcherScene::pointer_left) {
                    self.invalidate();
                }
            }
            SwitcherEvent::PointerReleased { x, y } => {
                let target = self.scene.as_ref().and_then(|scene| scene.hit_test(x, y));
                if let Some(SwitcherHitTarget::Close(window)) = target {
                    let key = self
                        .session
                        .as_ref()
                        .and_then(|session| {
                            session
                                .items()
                                .iter()
                                .find(|candidate| candidate.key() == window)
                        })
                        .map(WindowInfo::key);
                    self.hide();
                    if let Some(key) = key
                        && let Err(error) = activation::request_close(key, false)
                    {
                        lotus_windows::diagnostics::record_error(
                            "activation.switcher_close",
                            &error,
                        );
                        show_error(
                            self.window.handle(),
                            "Lotus",
                            &format!("Lotus could not close that window.\n\n{error}"),
                        );
                    }
                }
            }
            SwitcherEvent::Resized { width, height } => {
                if let Some(size) = NonZeroPhysicalSize::new(width, height) {
                    self.resize_surface(size, graphics)?;
                }
            }
            SwitcherEvent::DpiChanged { dpi } => {
                self.rebuild_scene(dpi)?;
                self.request_visible_icons();
            }
            SwitcherEvent::RenderRequested => self.invalidate(),
        }
        Ok(())
    }

    pub(super) fn rebuild_scene(&mut self, dpi: u32) -> Result<(), AppError> {
        let Some(session) = &self.session else {
            return Ok(());
        };
        let pixel_size = sampled_icon_size(dpi);
        let items = session
            .items()
            .iter()
            .map(|window| {
                let (presentation_icon, custom_image_path) = switcher_icon_sources(
                    window,
                    &self.assets.settings,
                    &self.applications,
                );
                SwitcherItem {
                    key: window.key(),
                    title: switcher_title(window, &self.name_overrides, &self.applications),
                    icon: self.assets.icon(
                        window.key(),
                        pixel_size,
                        presentation_icon.as_ref(),
                        custom_image_path.as_ref(),
                    ),
                }
            })
            .collect();
        self.scene = SwitcherScene::new(dpi, items, session.selected_index());
        if let Some(scene) = &mut self.scene {
            let _ = scene.set_theme(self.theme);
        }
        if self.scene.is_none() {
            return Err(AppError::InvalidSwitcherScene);
        }
        Ok(())
    }

    pub(super) fn apply_settings(&mut self, settings: &DockSettings) {
        self.theme = theme_for(settings);
        self.assets.apply_settings(settings);
        lotus_windows::backdrop::apply_popup_settings(self.window.handle(), settings);
        if let Some(scene) = &mut self.scene {
            let _ = scene.set_theme(self.theme);
        }
        self.request_visible_icons();
    }

    pub(super) fn drain_hydrated_icons(
        &mut self,
        results: impl IntoIterator<Item = lotus_windows::icon_hydrator::HydratedSwitcherIcon>,
    ) -> bool {
        let changed = self.assets.drain(results, &mut self.scene);
        if changed {
            self.invalidate();
        }
        changed
    }
}

impl SwitcherRuntime {
    fn request_visible_icons(&mut self) {
        self.assets.request_visible(
            self.session.as_ref(),
            self.scene.as_ref(),
            &self.applications,
        );
    }
}

fn switcher_icon_sources(
    window: &WindowInfo,
    settings: &DockSettings,
    applications: &ApplicationView,
) -> (Option<ApplicationPresentationIcon>, Option<PathBuf>) {
    let presentation_icon = applications
        .assignments()
        .presentation_by_window
        .get(&window.key())
        .map(|presentation| presentation.icon.clone());
    let identity = window_override_identity(window, applications);
    let custom_image_path =
        crate::app::icon_override::application_icon_path_for_identity(settings, &identity);
    (presentation_icon, custom_image_path)
}

fn sampled_icon_size(dpi: u32) -> u32 {
    lotus_ui::geometry::DpiScale::from_system(dpi)
        .physical(SWITCHER_ICON_DIP)
        .saturating_mul(NATIVE_ICON_SAMPLE_SCALE)
}

fn switcher_title(
    window: &WindowInfo,
    overrides: &BTreeMap<String, String>,
    applications: &ApplicationView,
) -> String {
    let key = window_application_key(window, applications.assignments());
    if let Some(name) = overrides.iter().find_map(|(identifier, display_name)| {
        applications
            .catalog()
            .key_for_external_identifier(identifier)
            .is_some_and(|candidate| candidate == key)
            .then_some(display_name.trim())
            .filter(|display_name| !display_name.is_empty())
    }) {
        return name.to_owned();
    }
    applications
        .assignments()
        .presentation_by_window
        .get(&window.key())
        .map_or_else(
            || "Application".to_owned(),
            |presentation| presentation.display_name.clone(),
        )
}

fn executable_is_hidden(window: &WindowInfo, hidden: &[String]) -> bool {
    hidden.iter().any(|candidate| {
        window
            .application_identity()
            .has_executable_alias(candidate)
    })
}

fn window_application_key(
    window: &WindowInfo,
    assignments: &WindowApplicationAssignments,
) -> ApplicationKey {
    match assignments.by_window.get(&window.key()) {
        Some(
            ApplicationResolution::Resolved { key, .. }
            | ApplicationResolution::Associated { key }
            | ApplicationResolution::Unregistered { key, .. },
        ) => key.clone(),
        Some(ApplicationResolution::Ambiguous { .. }) | None => {
            ApplicationKey::Ephemeral(window.key())
        }
    }
}

fn window_override_identity(
    window: &WindowInfo,
    applications: &ApplicationView,
) -> ApplicationIdentity {
    let key = window_application_key(window, applications.assignments());
    if let Some(application) = applications
        .catalog()
        .application_index_for_key(&key)
        .and_then(|index| applications.catalog().application(index))
    {
        return application.application_identity();
    }
    let stable_id = match &key {
        ApplicationKey::Registered(value)
        | ApplicationKey::LaunchSignature(value)
        | ApplicationKey::ExecutablePath(value) => Some(value.as_str()),
        ApplicationKey::Ephemeral(_) => None,
    };
    ApplicationIdentity::from_path(
        window.application_facts.reliable_id(),
        stable_id,
        Some(&window.executable_path),
        std::iter::empty(),
    )
}
