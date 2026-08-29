mod applications;
mod assets;
mod events;
mod pickers;

use std::time::Instant;

pub(in crate::app) use applications::application_records;
pub(in crate::app) use events::SettingsEventOutcome;
use lotus_core::settings::{ApplicationIconOverride, DockSettings};
use lotus_settings::scene::{
    SettingsAction, SettingsControl, SettingsScene, SettingsSize, SettingsUpdateActivity,
};
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::WindowHandle;
use lotus_windows::custom_image::CustomImageCache;
use lotus_windows::graphics::settings_surface::SettingsCompositionSurfaceState;
use lotus_windows::graphics::{DeviceState, GraphicsDevice, SurfaceSize};
use lotus_windows::interaction::PointerCursor;
use lotus_windows::native_icon::NativeIconCache;
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::search_catalog::SearchCatalogCache;
use lotus_windows::update::{Release, UpdateChecker, UpdateResult, UpdateStartError};
use lotus_windows::window::{
    SettingsEvent, SettingsKey as WindowSettingsKey, SettingsWindow,
};
pub(in crate::app) use pickers::{ApplicationIconOutcome, ColorOutcome, ColorTarget};

use crate::app::AppError;

pub(in crate::app) struct SettingsRuntime {
    window: SettingsWindow,
    scene: SettingsScene,
    surface: Option<ScheduledSurface<SettingsCompositionSurfaceState>>,
    visible: bool,
    dragging_slider: Option<lotus_settings::scene::SettingsSlider>,
    pressed_control: Option<SettingsControl>,
    native_icons: NativeIconCache,
    custom_images: CustomImageCache,
    update_checker: UpdateChecker,
}

impl SettingsRuntime {
    pub(in crate::app) fn new(
        window: SettingsWindow,
        settings: DockSettings,
        installed: bool,
    ) -> Result<Self, AppError> {
        let scene = SettingsScene::new(window.dpi(), settings, installed)
            .ok_or(AppError::InvalidSettingsScene)?;

        Ok(Self {
            window,
            scene,
            surface: None,
            visible: false,
            dragging_slider: None,
            pressed_control: None,
            native_icons: NativeIconCache::default(),
            custom_images: CustomImageCache::default(),
            update_checker: UpdateChecker::new(),
        })
    }

    pub(in crate::app) fn open(
        &mut self,
        applied: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if self.visible {
            self.window.focus();
            return Ok(());
        }
        self.window.use_material(applied);
        self.scene.end_onboarding();
        self.scene.mark_applied(applied.clone());
        self.show(graphics)
    }

    pub(in crate::app) fn open_onboarding(
        &mut self,
        applied: &DockSettings,
        required: bool,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.window.use_material(applied);
        self.scene.begin_onboarding(applied.clone(), required);
        self.show(graphics)
    }

    fn show(&mut self, graphics: &mut DeviceState) -> Result<(), AppError> {
        let _ = self.scene.set_dpi(self.window.dpi());

        let (width, height) = self.window.client_size()?;
        let _ = self.scene.set_available_size(width, height);
        self.window.set_layout_dpi(self.scene.effective_dpi());
        let size =
            SettingsSize::new(width, height).ok_or(AppError::InvalidSettingsScene)?;
        let surface_size = SurfaceSize::new(size.width(), size.height())
            .ok_or(AppError::ZeroSizedSurface)?;

        if let Some(surface) = &mut self.surface {
            surface.value_mut().resize(surface_size)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ScheduledSurface::new(
                SettingsCompositionSurfaceState::create(
                    device,
                    self.window.handle(),
                    surface_size,
                )?,
            ));
        }

        self.window.show()?;
        self.visible = true;
        self.invalidate();
        Ok(())
    }

    pub(in crate::app) fn hide(&mut self) {
        self.window.hide();
        self.window.set_pointer_cursor(PointerCursor::Arrow);
        self.visible = false;
        self.dragging_slider = None;
        self.pressed_control = None;
        if let Some(surface) = &mut self.surface {
            surface.stop_animation();
        }
    }

    pub(in crate::app) const fn is_visible(&self) -> bool {
        self.visible
    }

    pub(in crate::app) fn diagnostic_surface_state(&self) -> (bool, bool, bool) {
        let surface = self.surface.as_ref();
        (
            surface.is_some_and(ScheduledSurface::is_dirty),
            surface.is_some_and(ScheduledSurface::is_animating),
            self.visible,
        )
    }

    pub(in crate::app) fn owner(&self) -> WindowHandle {
        self.window.handle()
    }

    pub(in crate::app) fn draft(&self) -> &DockSettings {
        self.scene.draft()
    }

    pub(in crate::app) fn selected_application_id(&self) -> Option<String> {
        self.scene
            .selected_application()
            .map(|application| application.id.clone())
    }

    pub(in crate::app) fn set_applications(
        &mut self,
        applications: Vec<lotus_settings::scene::SettingsApplicationRecord>,
    ) {
        let _ = self.scene.set_applications(applications);
    }

    pub(in crate::app) fn open_application_manager(&mut self, id: &str) {
        let _ = self.scene.open_application_manager(id);
    }

    pub(in crate::app) fn set_dpi(&mut self, dpi: u32) {
        let _ = self.scene.set_dpi(dpi);
        self.window.set_layout_dpi(self.scene.effective_dpi());
    }

    pub(in crate::app) fn onboarding_active(&self) -> bool {
        self.scene.onboarding_required()
    }

    pub(in crate::app) fn end_onboarding(&mut self) {
        self.scene.end_onboarding();
    }

    pub(in crate::app) fn page_is_apps(&self) -> bool {
        self.scene.page() == lotus_settings::scene::SettingsPage::Apps
    }

    fn application_query(&self) -> &str {
        self.scene.application_query()
    }

    fn set_application_query(&mut self, query: &str) -> bool {
        self.scene.set_application_query(query)
    }

    pub(in crate::app) fn applications_are_empty(&self) -> bool {
        self.scene.applications().is_empty()
    }

    pub(in crate::app) fn applications_snapshot(
        &self,
    ) -> Vec<lotus_settings::scene::SettingsApplicationRecord> {
        self.scene.applications().to_vec()
    }

    pub(in crate::app) fn reset_application_icon_override(&mut self, id: &str) {
        self.scene.reset_application_icon_override(id);
    }

    pub(in crate::app) fn merged_application_icon_overrides(
        &self,
        current: &DockSettings,
    ) -> Vec<ApplicationIconOverride> {
        self.scene.merge_application_icon_overrides(current)
    }

    pub(in crate::app) fn mark_applied(&mut self, applied: DockSettings) {
        self.scene.mark_applied(applied);
    }

    pub(in crate::app) fn apply_material(&mut self, applied: &DockSettings) {
        self.window.use_material(applied);
    }

    pub(in crate::app) fn set_update_activity(&mut self, activity: SettingsUpdateActivity) {
        let _ = self.scene.set_update_activity(activity);
    }

    pub(in crate::app) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }

    pub(in crate::app) fn recover_surface(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        if let Some(surface) = &mut self.surface {
            surface.value_mut().recover(device)?;
        }
        Ok(())
    }

    pub(in crate::app) fn drain_events(&mut self) -> Vec<SettingsEvent> {
        self.window.drain_events().collect()
    }

    pub(in crate::app) fn has_pending_events(&self) -> bool {
        self.window.has_pending_events()
    }

    fn resize(
        &mut self,
        graphics: &mut DeviceState,
        width: u32,
        height: u32,
    ) -> Result<(), AppError> {
        let Some(size) = SettingsSize::new(width, height) else {
            return Ok(());
        };
        let _ = self.scene.set_available_size(width, height);
        self.window.set_layout_dpi(self.scene.effective_dpi());
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };

        let size = SurfaceSize::new(size.width(), size.height())
            .ok_or(AppError::ZeroSizedSurface)?;

        match surface.value_mut().resize(size) {
            Ok(()) => {
                self.invalidate();
                Ok(())
            }
            Err(lotus_windows::graphics::SurfaceError::DeviceLost(loss)) => {
                graphics.mark_lost(loss);
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn pointer_moved(&mut self, x: u32, y: u32) -> Option<SettingsAction> {
        let style_started = Instant::now();
        let style = self.scene.pointer_style(x, y);
        METRICS.record_layout(LayoutOperation::SettingsPointer, style_started.elapsed());
        let cursor = if self.dragging_slider.is_some() {
            PointerCursor::HorizontalResize
        } else {
            settings_pointer_cursor(style)
        };
        self.window.set_pointer_cursor(cursor);

        if let Some(slider) = self.dragging_slider {
            let started = Instant::now();
            self.scene.pointer_move(x, y);
            let action = self.scene.set_slider_from_pointer(slider, x);
            METRICS.record_layout(LayoutOperation::SettingsPointer, started.elapsed());
            return Some(action);
        }
        let started = Instant::now();
        if self.scene.pointer_move(x, y) {
            self.invalidate();
        }
        METRICS.record_layout(LayoutOperation::SettingsPointer, started.elapsed());
        None
    }

    fn pointer_left(&mut self) {
        self.window.set_pointer_cursor(PointerCursor::Arrow);
        if self.scene.set_hovered(None) {
            self.invalidate();
        }
    }

    fn pointer_pressed(&mut self, x: u32, y: u32) -> Option<SettingsAction> {
        let started = Instant::now();
        self.scene.pointer_move(x, y);
        self.dragging_slider = self.scene.slider_at(x, y);
        self.pressed_control = self
            .dragging_slider
            .is_none()
            .then(|| self.scene.layout().hit_test(x, y))
            .flatten();
        let action = self
            .dragging_slider
            .map(|slider| self.scene.set_slider_from_pointer(slider, x));
        METRICS.record_layout(LayoutOperation::SettingsPointer, started.elapsed());
        action
    }

    fn pointer_released(&mut self, x: i32, y: i32) -> Option<SettingsAction> {
        if self.dragging_slider.take().is_some() {
            self.pressed_control = None;
            let cursor = u32::try_from(x).ok().zip(u32::try_from(y).ok()).map_or(
                PointerCursor::Arrow,
                |(x, y)| {
                    let started = Instant::now();
                    let style = self.scene.pointer_style(x, y);
                    METRICS
                        .record_layout(LayoutOperation::SettingsPointer, started.elapsed());
                    settings_pointer_cursor(style)
                },
            );
            self.window.set_pointer_cursor(cursor);
            return None;
        }
        let released_control = u32::try_from(x)
            .ok()
            .zip(u32::try_from(y).ok())
            .and_then(|(x, y)| self.scene.layout().hit_test(x, y));
        let pressed_control = self.pressed_control.take();
        if pressed_control.is_none() || pressed_control != released_control {
            return Some(SettingsAction::None);
        }
        Some(self.activation_at(x, y))
    }

    fn pointer_cancelled(&mut self) {
        self.dragging_slider = None;
        self.pressed_control = None;
        self.window.set_pointer_cursor(PointerCursor::Arrow);
    }

    fn activation_at(&mut self, x: i32, y: i32) -> SettingsAction {
        u32::try_from(x).ok().zip(u32::try_from(y).ok()).map_or(
            SettingsAction::None,
            |(x, y)| {
                let started = Instant::now();
                let action = self.scene.pointer_activate(x, y);
                METRICS.record_layout(LayoutOperation::SettingsPointer, started.elapsed());
                action
            },
        )
    }

    fn scrolled(&mut self, direction: i32) -> bool {
        if !self.scene.scroll(direction) {
            return false;
        }
        let entered_apps = self.page_is_apps();
        self.invalidate();
        entered_apps
    }

    fn translated_key(&mut self, key: WindowSettingsKey) -> SettingsAction {
        let key = match key {
            WindowSettingsKey::Escape => lotus_settings::scene::SettingsKey::Escape,
            WindowSettingsKey::Enter | WindowSettingsKey::Space => {
                lotus_settings::scene::SettingsKey::Activate
            }
            WindowSettingsKey::Tab { reverse: false } => {
                lotus_settings::scene::SettingsKey::Tab
            }
            WindowSettingsKey::Tab { reverse: true } => {
                lotus_settings::scene::SettingsKey::ReverseTab
            }
            WindowSettingsKey::Left => lotus_settings::scene::SettingsKey::Left,
            WindowSettingsKey::Right => lotus_settings::scene::SettingsKey::Right,
            WindowSettingsKey::Up => lotus_settings::scene::SettingsKey::Up,
            WindowSettingsKey::Down => lotus_settings::scene::SettingsKey::Down,
            WindowSettingsKey::Save if self.scene.is_dirty() => {
                return SettingsAction::Apply(Box::new(
                    self.scene.draft().clone().normalized(),
                ));
            }
            WindowSettingsKey::Backspace
            | WindowSettingsKey::Save
            | WindowSettingsKey::Paste => {
                return SettingsAction::None;
            }
        };
        self.scene.key(key)
    }

    pub(in crate::app) fn clear_icon_caches(&mut self) {
        self.custom_images.clear();
    }

    pub(in crate::app) fn choose_color(&mut self, target: ColorTarget) -> ColorOutcome {
        let owner = self.owner();
        pickers::choose_color(&mut self.scene, owner, target)
    }

    pub(in crate::app) fn choose_mascot_image(
        &mut self,
        settings_directory: &std::path::Path,
    ) -> pickers::MascotImageOutcome {
        pickers::choose_mascot_image(self.owner(), settings_directory, &mut self.scene)
    }

    pub(in crate::app) fn choose_application_icon(
        &mut self,
        id: &str,
        settings_directory: &std::path::Path,
    ) -> ApplicationIconOutcome {
        let applications = self.applications_snapshot();
        pickers::choose_application_icon(
            id,
            self.owner(),
            settings_directory,
            &mut self.scene,
            &applications,
        )
    }

    pub(in crate::app) fn start_update_check(&mut self) -> Result<bool, UpdateStartError> {
        let started = self
            .update_checker
            .start_check(env!("CARGO_PKG_VERSION"), self.scene.draft().update_channel)?;
        if started {
            self.set_update_activity(SettingsUpdateActivity::Checking);
        }
        Ok(started)
    }

    pub(in crate::app) fn start_update_download(
        &mut self,
        release: Release,
    ) -> Result<bool, UpdateStartError> {
        let started = self.update_checker.start_download(release)?;
        if started {
            self.set_update_activity(SettingsUpdateActivity::Installing);
        }
        Ok(started)
    }

    pub(in crate::app) fn drain_update_results(&self) -> Vec<UpdateResult> {
        self.update_checker.drain().collect()
    }

    pub(in crate::app) fn hydrate_application_previews(
        &mut self,
        cache: &SearchCatalogCache,
        dock_items: &[lotus_core::dock::DockItem],
    ) {
        applications::hydrate_previews(self, cache, dock_items);
    }

    pub(in crate::app) fn render_frame(
        &mut self,
        pass: &mut lotus_ui::frame::FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        assets::render_frame(self, pass, graphics)
    }
}

fn settings_pointer_cursor(
    style: lotus_settings::scene::SettingsPointerStyle,
) -> PointerCursor {
    match style {
        lotus_settings::scene::SettingsPointerStyle::Default => PointerCursor::Arrow,
        lotus_settings::scene::SettingsPointerStyle::Action => PointerCursor::Hand,
        lotus_settings::scene::SettingsPointerStyle::HorizontalAdjustment => {
            PointerCursor::HorizontalResize
        }
    }
}
