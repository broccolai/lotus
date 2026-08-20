use lotus_core::settings::DockSettings;
use lotus_core::window::WindowInfo;
use lotus_settings::appearance::theme_for;
use lotus_switcher::model::{RecentOrder, SwitcherSession};
use lotus_ui::frame::{FrameOutcome, FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_ui::theme::Theme;
use lotus_windows::activation::{request_window_close, switch_window};
use lotus_windows::custom_image::CustomImageCache;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::scene_adapter::resolve_icon_with_native;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::switcher_surface::SwitcherCompositionSurfaceState;
use lotus_windows::graphics::{
    DeviceState, SurfaceError, SwitcherHitTarget, SwitcherItem, SwitcherScene,
};
use lotus_windows::interaction::PointerCursor;
use lotus_windows::native_icon::NativeIconCache;
use lotus_windows::search_catalog::SearchCatalogCache;
use lotus_windows::window::{SwitcherEvent, SwitcherWindow};

use crate::app::AppError;
use crate::app::context_menu::ContextMenuRuntime;
use crate::app::launcher::LauncherRuntime;
use crate::app::media::MediaRuntime;
use crate::app::monitors::MonitorDocks;
use crate::app::settings::SettingsRuntime;
use crate::app::status::StatusRuntime;

const SWITCHER_ICON_DIP: u32 = 38;
const NATIVE_ICON_SAMPLE_SCALE: u32 = 2;

pub(super) struct AuxiliaryWindows {
    pub(super) applications: SearchCatalogCache,
    pub(super) launcher: LauncherRuntime,
    pub(super) settings: SettingsRuntime,
    pub(super) context_menu: ContextMenuRuntime,
    pub(super) media: MediaRuntime,
    pub(super) status: StatusRuntime,
    pub(super) monitors: MonitorDocks,
    pub(super) switcher: SwitcherRuntime,
}

impl AuxiliaryWindows {
    pub(super) fn invalidate_surfaces(&mut self) {
        self.launcher.invalidate();
        self.settings.invalidate();
        self.context_menu.invalidate();
        self.switcher.invalidate();
        self.status.invalidate();
        self.monitors.invalidate();
    }
}

pub(super) struct SwitcherRuntime {
    pub(super) window: SwitcherWindow,
    pub(super) surface: Option<ScheduledSurface<SwitcherCompositionSurfaceState>>,
    pub(super) scene: Option<SwitcherScene>,
    pub(super) session: Option<SwitcherSession<WindowInfo>>,
    pub(super) native_icons: NativeIconCache,
    custom_images: CustomImageCache,
    icon_settings: DockSettings,
    pub(super) name_overrides: std::collections::BTreeMap<String, String>,
    recent_windows: RecentOrder<lotus_core::window::WindowId>,
    theme: Theme,
}

impl SwitcherRuntime {
    pub(super) fn new(
        window: SwitcherWindow,
        settings: &DockSettings,
        theme: &Theme,
    ) -> Self {
        Self {
            window,
            surface: None,
            scene: None,
            session: None,
            native_icons: NativeIconCache::default(),
            custom_images: CustomImageCache::default(),
            icon_settings: settings.clone(),
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
        self.record_foreground(foreground);
        let windows = self.recent_windows.arrange(windows, |window| window.id);
        let Some(session) = SwitcherSession::begin(windows, direction) else {
            return Ok(());
        };
        self.name_overrides = settings.application_name_overrides.clone();
        self.icon_settings = settings.clone();
        self.custom_images.clear();
        self.theme = theme_for(settings);
        self.session = Some(session);
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
        self.invalidate();
        Ok(())
    }

    pub(super) fn record_foreground(
        &mut self,
        foreground: Option<lotus_core::window::WindowId>,
    ) {
        if let Some(foreground) = foreground {
            self.recent_windows.record(foreground);
        }
    }

    pub(super) fn cycle(&mut self, direction: lotus_switcher::model::Direction) {
        let Some(session) = &mut self.session else {
            return;
        };
        session.cycle(direction);
        if let Some(scene) = &mut self.scene {
            let _changed = scene.set_selected(session.selected_index());
        }
        self.invalidate();
    }

    pub(super) fn commit(&mut self) {
        let selected = self.session.as_ref().map(|session| session.selected().id);
        self.hide();
        if let Some(selected) = selected {
            self.recent_windows.record(selected);
            let _ = switch_window(selected);
        }
    }

    pub(super) fn hide(&mut self) {
        self.window.hide();
        self.scene = None;
        self.session = None;
    }

    pub(super) fn drain_events(&mut self) -> Vec<SwitcherEvent> {
        self.window.drain_events().collect()
    }

    pub(super) fn handle_window_event(
        &mut self,
        event: SwitcherEvent,
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
                    self.hide();
                    if let Err(error) = request_window_close(window) {
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
                    surface.value_mut().resize(size)?;
                }
            }
            SwitcherEvent::DpiChanged { dpi } => {
                self.rebuild_scene(dpi)?;
            }
            SwitcherEvent::RenderRequested => self.invalidate(),
        }
        Ok(())
    }

    pub(super) fn rebuild_scene(&mut self, dpi: u32) -> Result<(), AppError> {
        let Some(session) = &self.session else {
            return Ok(());
        };
        let icon_size = lotus_ui::geometry::DpiScale::from_system(dpi)
            .physical(SWITCHER_ICON_DIP)
            .saturating_mul(NATIVE_ICON_SAMPLE_SCALE);
        let items = session
            .items()
            .iter()
            .map(|window| SwitcherItem {
                window: window.id,
                title: switcher_title(window, &self.name_overrides),
                icon: resolve_icon_with_native(|| {
                    crate::app::icon_override::resolve_application_icon(
                        &self.icon_settings,
                        &mut self.custom_images,
                        window.app_user_model_id.as_deref(),
                        None,
                        &window.executable_path,
                    )
                    .or_else(|| {
                        self.native_icons
                            .icon(&window.executable_path, icon_size)
                            .ok()
                            .flatten()
                    })
                }),
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
        self.custom_images.clear();
        lotus_windows::backdrop::apply_popup_settings(self.window.handle(), settings);
        if let Some(scene) = &mut self.scene {
            let _ = scene.set_theme(self.theme);
        }
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
        pass.render(surface, |surface| match surface.render_scene(scene) {
            Ok(FrameResult::Presented { needs_animation }) => {
                Ok(FrameOutcome::complete(needs_animation))
            }
            Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
            Err(SurfaceError::DeviceLost(_)) => {
                let _ = graphics.poll();
                graphics.recover()?;
                let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                surface.recover(device)?;
                match surface.render_scene(scene)? {
                    FrameResult::Presented { needs_animation } => {
                        Ok(FrameOutcome::complete(needs_animation))
                    }
                    FrameResult::TargetRecreated => Ok(FrameOutcome::Retry),
                }
            }
            Err(error) => Err(error.into()),
        })
    }
}

fn switcher_title(
    window: &WindowInfo,
    overrides: &std::collections::BTreeMap<String, String>,
) -> String {
    let executable_name = window
        .executable_path
        .file_name()
        .and_then(|name| name.to_str());
    if let Some(name) = executable_name.and_then(|name| {
        overrides
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, display_name)| display_name.trim())
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
    let Some(name) = window
        .executable_path
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    hidden
        .iter()
        .any(|candidate| candidate.trim().eq_ignore_ascii_case(name))
}
