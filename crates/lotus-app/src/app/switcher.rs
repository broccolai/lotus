use lotus_core::application::{ApplicationIdentity, is_reliable_registered_id};
use lotus_core::settings::DockSettings;
use lotus_core::window::{TrackedWindowKey, WindowInfo};
use lotus_settings::appearance::theme_for;
use lotus_switcher::model::{RecentOrder, ReconcileOutcome, SwitcherSession};
use lotus_ui::frame::{FrameOutcome, FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_ui::theme::Theme;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::assets::SvgAsset;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::switcher_surface::SwitcherCompositionSurfaceState;
use lotus_windows::graphics::{DeviceState, GraphicsDevice, SurfaceError};
use lotus_windows::icon_hydrator::{SwitcherIconClient, SwitcherIconRequest};
use lotus_windows::interaction::PointerCursor;
use lotus_windows::window::{SwitcherEvent, SwitcherWindow};

use crate::app::visuals::{DockIcon, SwitcherHitTarget, SwitcherItem, SwitcherScene};
use crate::app::{AppError, activation};

const SWITCHER_ICON_DIP: u32 = 38;
const NATIVE_ICON_SAMPLE_SCALE: u32 = 2;

pub(super) struct SwitcherRuntime {
    pub(super) window: SwitcherWindow,
    pub(super) surface: Option<ScheduledSurface<SwitcherCompositionSurfaceState>>,
    pub(super) scene: Option<SwitcherScene>,
    pub(super) session: Option<SwitcherSession<WindowInfo>>,
    icon_hydrator: SwitcherIconClient,
    icon_settings: DockSettings,
    icon_generation: u64,
    icon_settings_revision: u64,
    pub(super) name_overrides: std::collections::BTreeMap<String, String>,
    recent_windows: RecentOrder<TrackedWindowKey>,
    theme: Theme,
}

impl SwitcherRuntime {
    pub(super) fn diagnostic_surface_state(&self) -> (bool, bool, bool) {
        let surface = self.surface.as_ref();
        (
            surface.is_some_and(ScheduledSurface::is_dirty),
            surface.is_some_and(ScheduledSurface::is_animating),
            self.session.is_some(),
        )
    }

    pub(super) fn new(
        window: SwitcherWindow,
        settings: &DockSettings,
        theme: &Theme,
        icon_hydrator: SwitcherIconClient,
    ) -> Self {
        Self {
            window,
            surface: None,
            scene: None,
            session: None,
            icon_hydrator,
            icon_settings: settings.clone(),
            icon_generation: 0,
            icon_settings_revision: 0,
            name_overrides: std::collections::BTreeMap::new(),
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
        self.icon_settings = settings.clone();
        self.theme = theme_for(settings);
        self.session = Some(session);
        self.icon_generation = self.icon_generation.wrapping_add(1);
        self.rebuild_scene(self.window.dpi())?;
        let size = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidSwitcherScene)?
            .desired_size();
        let dpi = self.window.show_centered(foreground, size)?;
        if dpi
            != self
                .scene
                .as_ref()
                .ok_or(AppError::InvalidSwitcherScene)?
                .dpi()
        {
            self.rebuild_scene(dpi)?;
            let size = self
                .scene
                .as_ref()
                .ok_or(AppError::InvalidSwitcherScene)?
                .desired_size();
            let _dpi = self.window.show_centered(foreground, size)?;
        }
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

    pub(super) fn reconcile_windows(
        &mut self,
        windows: &[WindowInfo],
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.recent_windows
            .retain(windows.iter().map(WindowInfo::key));
        let Some(session) = &mut self.session else {
            return Ok(());
        };
        let latest = windows
            .iter()
            .filter(|window| {
                !executable_is_hidden(window, &self.icon_settings.hidden_executables)
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
                self.icon_generation = self.icon_generation.wrapping_add(1);
                self.rebuild_scene(self.window.dpi())?;
                self.recenter_visible_window()?;
                self.ensure_surface(graphics)?;
                self.request_visible_icons();
                self.invalidate();
            }
            ReconcileOutcome::Refreshed => {
                self.icon_generation = self.icon_generation.wrapping_add(1);
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
        let size = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidSwitcherScene)?
            .desired_size();
        let foreground = lotus_windows::activation::foreground_window();
        let dpi = self.window.show_centered(foreground, size)?;
        if self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidSwitcherScene)?
            .dpi()
            != dpi
        {
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
        self.icon_hydrator.request_switcher(Vec::new());
        self.scene = None;
        self.session = None;
    }

    pub(super) fn abandon(&mut self) {
        self.window.hide();
        self.surface = None;
        self.scene = None;
        self.session = None;
    }

    pub(super) fn drain_events(&mut self) -> Vec<SwitcherEvent> {
        self.window.drain_events().collect()
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
                        show_error(
                            self.window.handle(),
                            "Lotus",
                            &format!("Lotus could not close that window.\n\n{error}"),
                        );
                    }
                }
            }
            SwitcherEvent::Resized { width, height } => {
                if let Some(size) = NonZeroPhysicalSize::new(width, height)
                    && let Some(surface) = &mut self.surface
                {
                    match surface.value_mut().resize(size) {
                        Ok(()) => {}
                        Err(SurfaceError::DeviceLost(loss)) => graphics.mark_lost(loss),
                        Err(error) => return Err(error.into()),
                    }
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
        let items = session
            .items()
            .iter()
            .map(|window| SwitcherItem {
                key: window.key(),
                title: switcher_title(window, &self.name_overrides),
                icon: None,
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
        self.icon_settings = settings.clone();
        self.icon_settings_revision = self.icon_settings_revision.wrapping_add(1);
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
        let Some(scene) = &mut self.scene else {
            return false;
        };
        let dpi = scene.dpi();
        let icon_size = sampled_icon_size(dpi);
        let mut changed = false;

        for result in results {
            if result.generation != self.icon_generation
                || result.settings_revision != self.icon_settings_revision
                || result.pixel_size != icon_size
            {
                continue;
            }
            changed |= scene.set_icon(result.window, result.icon.map(DockIcon::Raster));
        }
        if changed {
            self.invalidate();
        }
        changed
    }

    pub(super) fn ensure_surface(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let scene = self.scene.as_ref().ok_or(AppError::InvalidSwitcherScene)?;
        let size = scene.desired_size();
        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(size)?;
            return Ok(());
        }
        let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
        self.surface = Some(ScheduledSurface::new(
            SwitcherCompositionSurfaceState::create(device, self.window.handle(), size)?,
        ));
        Ok(())
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }

    pub(super) fn recover_surface(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        if let Some(surface) = &mut self.surface {
            surface.value_mut().recover(device)?;
        }
        Ok(())
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if self.session.is_none() {
            if let Some(surface) = &mut self.surface {
                surface.stop_animation();
            }
            return Ok(());
        }
        let (Some(scene), Some(surface)) = (&self.scene, &mut self.surface) else {
            return Ok(());
        };
        let presentation = scene.presentation(SvgAsset::FluentDismiss);
        pass.render(surface, |surface| {
            match surface.render_scene(&presentation) {
                Ok(FrameResult::Presented { needs_animation }) => {
                    Ok(FrameOutcome::complete(needs_animation))
                }
                Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
                Err(SurfaceError::DeviceLost(loss)) => {
                    graphics.mark_lost(loss);
                    Ok(FrameOutcome::complete(false))
                }
                Err(error) => Err(error.into()),
            }
        })
    }
}

impl SwitcherRuntime {
    fn request_visible_icons(&self) {
        let (Some(session), Some(scene)) = (&self.session, &self.scene) else {
            self.icon_hydrator.request_switcher(Vec::new());
            return;
        };
        let pixel_size = sampled_icon_size(scene.dpi());
        let requests = scene
            .visible_range_with_margin(2)
            .filter_map(|index| {
                let window = session.items().get(index)?;
                Some(SwitcherIconRequest {
                    generation: self.icon_generation,
                    window: window.id,
                    executable_path: window.executable_path.clone(),
                    custom_image_path: crate::app::icon_override::application_icon_path(
                        &self.icon_settings,
                        window.app_user_model_id.as_deref(),
                        None,
                        &window.executable_path,
                    ),
                    pixel_size,
                    settings_revision: self.icon_settings_revision,
                })
            })
            .collect();
        self.icon_hydrator.request_switcher(requests);
    }
}

fn sampled_icon_size(dpi: u32) -> u32 {
    lotus_ui::geometry::DpiScale::from_system(dpi)
        .physical(SWITCHER_ICON_DIP)
        .saturating_mul(NATIVE_ICON_SAMPLE_SCALE)
}

fn switcher_title(
    window: &WindowInfo,
    overrides: &std::collections::BTreeMap<String, String>,
) -> String {
    if let Some(name) = overrides.iter().find_map(|(identifier, display_name)| {
        application_identifier_matches_window(identifier, window)
            .then_some(display_name.trim())
            .filter(|display_name| !display_name.is_empty())
    }) {
        return name.to_owned();
    }
    window
        .executable_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Application")
        .to_owned()
}

fn executable_is_hidden(window: &WindowInfo, hidden: &[String]) -> bool {
    hidden.iter().any(|candidate| {
        window
            .application_identity()
            .has_executable_alias(candidate)
    })
}

fn application_identifier_matches_window(identifier: &str, window: &WindowInfo) -> bool {
    if is_reliable_registered_id(identifier) {
        return ApplicationIdentity::new(Some(identifier), None, None, std::iter::empty())
            .match_strength(&window.application_identity())
            .is_match();
    }

    window
        .application_identity()
        .has_executable_alias(identifier)
}
