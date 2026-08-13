use super::{
    AppError, ContextMenuRuntime, DeviceState, DockSettings, LauncherRuntime, NativeIconCache,
    NonZeroPhysicalSize, SettingsRuntime, SurfaceError, SwitcherCompositionSurfaceState,
    SwitcherEvent, SwitcherItem, SwitcherScene, SwitcherSession, SwitcherWindow, WindowInfo,
    resolve_icon_with_native, switch_window,
};
use lotus_settings::appearance::theme_for;
use lotus_ui::theme::Theme;

const SWITCHER_ICON_DIP: u32 = 38;
const NATIVE_ICON_SAMPLE_SCALE: u32 = 2;

pub(super) struct AuxiliaryWindows {
    pub(super) launcher: LauncherRuntime,
    pub(super) settings: SettingsRuntime,
    pub(super) context_menu: ContextMenuRuntime,
    pub(super) switcher: SwitcherRuntime,
}

pub(super) struct SwitcherRuntime {
    pub(super) window: SwitcherWindow,
    pub(super) surface: Option<SwitcherCompositionSurfaceState>,
    pub(super) scene: Option<SwitcherScene>,
    pub(super) session: Option<SwitcherSession<WindowInfo>>,
    pub(super) native_icons: NativeIconCache,
    pub(super) name_overrides: std::collections::BTreeMap<String, String>,
    theme: Theme,
}

impl SwitcherRuntime {
    pub(super) fn new(window: SwitcherWindow, theme: &Theme) -> Self {
        Self {
            window,
            surface: None,
            scene: None,
            session: None,
            native_icons: NativeIconCache::default(),
            name_overrides: std::collections::BTreeMap::new(),
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
        let mut windows = windows
            .iter()
            .filter(|window| !executable_is_hidden(window, &settings.hidden_executables))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(foreground) = foreground
            && let Some(index) = windows.iter().position(|window| window.id == foreground)
        {
            windows.rotate_left(index);
        }
        let Some(session) = SwitcherSession::begin(windows, direction) else { return Ok(()) };
        self.name_overrides = settings.application_name_overrides.clone();
        self.theme = theme_for(settings);
        self.session = Some(session);
        self.rebuild_scene(self.window.dpi())?;
        let size = self.scene.as_ref().ok_or(AppError::InvalidSwitcherScene)?.desired_size();
        let dpi = self.window.show_centered(foreground, size)?;
        if dpi != self.scene.as_ref().ok_or(AppError::InvalidSwitcherScene)?.dpi() {
            self.rebuild_scene(dpi)?;
            let size = self.scene.as_ref().ok_or(AppError::InvalidSwitcherScene)?.desired_size();
            let _dpi = self.window.show_centered(foreground, size)?;
        }
        self.ensure_surface(graphics)?;
        self.render(graphics)
    }

    pub(super) fn cycle(
        &mut self,
        direction: lotus_switcher::model::Direction,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let Some(session) = &mut self.session else { return Ok(()) };
        session.cycle(direction);
        if let Some(scene) = &mut self.scene {
            let _changed = scene.set_selected(session.selected_index());
        }
        self.render(graphics)
    }

    pub(super) fn commit(&mut self) {
        let selected = self.session.as_ref().map(|session| session.selected().id);
        self.hide();
        if let Some(selected) = selected {
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
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        match event {
            SwitcherEvent::CloseRequested => self.hide(),
            SwitcherEvent::Resized { width, height } => {
                if let Some(size) = NonZeroPhysicalSize::new(width, height)
                    && let Some(surface) = &mut self.surface
                {
                    surface.resize(size)?;
                }
            }
            SwitcherEvent::DpiChanged { dpi } => {
                self.rebuild_scene(dpi)?;
            }
            SwitcherEvent::RenderRequested => self.render(graphics)?,
        }
        Ok(())
    }

    pub(super) fn rebuild_scene(&mut self, dpi: u32) -> Result<(), AppError> {
        let Some(session) = &self.session else { return Ok(()) };
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
                    self.native_icons.icon(&window.executable_path, icon_size).ok().flatten()
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
        lotus_windows::backdrop::apply_popup_settings(self.window.handle(), settings);
        if let Some(scene) = &mut self.scene {
            let _ = scene.set_theme(self.theme);
        }
    }

    pub(super) fn ensure_surface(&mut self, graphics: &mut DeviceState) -> Result<(), AppError> {
        let scene = self.scene.as_ref().ok_or(AppError::InvalidSwitcherScene)?;
        let size = scene.desired_size();
        if let Some(surface) = &mut self.surface {
            surface.resize(size)?;
            return Ok(());
        }
        let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
        self.surface =
            Some(SwitcherCompositionSurfaceState::create(device, self.window.handle(), size)?);
        Ok(())
    }

    pub(super) fn render(&mut self, graphics: &mut DeviceState) -> Result<(), AppError> {
        let (Some(scene), Some(surface)) = (&self.scene, &mut self.surface) else {
            return Ok(());
        };
        match surface.render_scene(scene) {
            Ok(_) => Ok(()),
            Err(SurfaceError::DeviceLost(_)) => {
                graphics.recover()?;
                let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                surface.recover(device)?;
                let _frame = surface.render_scene(scene)?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn switcher_title(
    window: &WindowInfo,
    overrides: &std::collections::BTreeMap<String, String>,
) -> String {
    let executable_name = window.executable_path.file_name().and_then(|name| name.to_str());
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
    let Some(name) = window.executable_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    hidden.iter().any(|candidate| candidate.trim().eq_ignore_ascii_case(name))
}
