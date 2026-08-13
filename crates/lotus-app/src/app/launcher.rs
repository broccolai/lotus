use lotus_settings::appearance::theme_for;
use lotus_ui::theme::Theme;

use super::{
    AppError, DeviceState, DockIcon, DockRuntime, DockSettings, DockWindow, Duration,
    LauncherCompositionSurfaceState, LauncherResult, LauncherScene, NativeIconCache, Path,
    SearchCatalogCache, SearchController, SearchEvent, SearchPresentation,
    SearchUsageStore, SearchWindow, SelectionDirection, SelectionMove, SurfaceError,
    SurfaceSize, WindowHandle, launch_target, local_time_24h, resize_launcher_surface,
    show_error,
};

pub(super) struct LauncherRuntime {
    pub(super) window: SearchWindow,
    pub(super) controller: SearchController,
    pub(super) catalog: SearchCatalogCache,
    pub(super) native_icons: NativeIconCache,
    pub(super) scene: Option<LauncherScene>,
    pub(super) surface: Option<LauncherCompositionSurfaceState>,
    pub(super) presentation: SearchPresentation,
    pub(super) visible: bool,
    theme: Theme,
}

impl LauncherRuntime {
    pub(super) fn new(
        window: SearchWindow,
        result_limit: u32,
        theme: &Theme,
        usage: lotus_core::search::SearchUsage,
        usage_store: SearchUsageStore,
    ) -> Self {
        Self {
            window,
            controller: SearchController::new(
                usize::try_from(result_limit).unwrap_or(8),
                usage,
                usage_store,
            ),
            catalog: SearchCatalogCache::new(),
            native_icons: NativeIconCache::default(),
            scene: None,
            surface: None,
            presentation: SearchPresentation::default(),
            visible: false,
            theme: *theme,
        }
    }

    pub(super) const fn is_visible(&self) -> bool {
        self.visible
    }

    pub(super) const fn needs_animation(&self) -> bool {
        self.visible && self.presentation.is_animating()
    }

    pub(super) fn toggle(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<bool, AppError> {
        if self.visible {
            self.hide();
            return Ok(false);
        }

        let _ = self.catalog.refresh_if_stale(Duration::from_mins(5));
        if let Some(ready) = self.catalog.ready_catalog(
            dock_model.items(),
            &dock_model.settings().hidden_executables,
        ) {
            self.controller.begin(Some(ready.generation), ready.catalog);
        } else {
            self.controller.begin(
                None,
                self.catalog.catalog(
                    dock_model.items(),
                    &dock_model.settings().hidden_executables,
                ),
            );
        }
        self.presentation.begin();
        self.rebuild_scene(dock.dpi())?;

        let scene = self.scene.as_ref().ok_or(AppError::InvalidLauncherScene)?;
        let desired = scene.desired_size();
        self.window
            .show_sized(dock.handle(), desired.width(), desired.height())?;
        if self.window.dpi() != scene.dpi() {
            self.rebuild_scene(self.window.dpi())?;
            let desired = self
                .scene
                .as_ref()
                .ok_or(AppError::InvalidLauncherScene)?
                .desired_size();
            self.window
                .show_sized(dock.handle(), desired.width(), desired.height())?;
        }

        let desired = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidLauncherScene)?
            .desired_size();
        let size = SurfaceSize::from(desired);
        if let Some(surface) = &mut self.surface {
            resize_launcher_surface(graphics, surface, size)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(LauncherCompositionSurfaceState::create(
                device,
                self.window.handle(),
                size,
            )?);
        }
        self.visible = true;
        let needs_animation = self.render(graphics)?;
        self.window.focus();
        Ok(needs_animation)
    }

    pub(super) fn refresh_catalog_if_ready(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<bool, AppError> {
        let Some(ready) = self.catalog.ready_catalog(
            dock_model.items(),
            &dock_model.settings().hidden_executables,
        ) else {
            return Ok(false);
        };
        if !self
            .controller
            .refresh_catalog(ready.generation, ready.catalog)
        {
            return Ok(false);
        }
        if !self.visible {
            return Ok(false);
        }
        self.rebuild_scene(self.window.dpi())?;
        self.sync_size(dock, graphics)?;
        Ok(true)
    }

    pub(super) fn hide(&mut self) {
        self.window.hide();
        self.visible = false;
        self.presentation.finish();
    }

    pub(super) fn rebuild_scene(&mut self, dpi: u32) -> Result<(), AppError> {
        let iconless_results = self
            .controller
            .results()
            .iter()
            .map(|entry| LauncherResult::new(&entry.name))
            .collect::<Vec<_>>();
        let icon_size = LauncherScene::new(
            dpi,
            self.controller.query(),
            iconless_results,
            self.controller.selected_index(),
        )
        .ok_or(AppError::InvalidLauncherScene)?
        .result_icon_size()
        .get();
        let results = self
            .controller
            .results()
            .iter()
            .map(|entry| {
                let icon = self
                    .native_icons
                    .icon(Path::new(&entry.icon_source), icon_size)
                    .ok()
                    .flatten()
                    .map(DockIcon::Raster);
                icon.map_or_else(
                    || LauncherResult::new(&entry.name),
                    |icon| LauncherResult::with_icon(&entry.name, icon),
                )
            })
            .collect();
        let mut scene = LauncherScene::new(
            dpi,
            self.controller.query(),
            results,
            self.controller.selected_index(),
        )
        .ok_or(AppError::InvalidLauncherScene)?;
        let _ = scene.set_theme(self.theme);
        scene.set_result_viewport(
            self.controller.visible_start(),
            self.controller.total_results(),
        );
        scene.set_query_cursor(self.controller.query_cursor());
        let _ = scene.set_footer_time(local_time_24h());
        let _ = scene.set_presentation_progress(self.presentation.progress());
        self.scene = Some(scene);
        Ok(())
    }

    pub(super) fn move_selection(
        &mut self,
        direction: SelectionDirection,
    ) -> Result<(), AppError> {
        self.controller.move_selection(match direction {
            SelectionDirection::Previous => SelectionMove::Previous,
            SelectionDirection::Next => SelectionMove::Next,
        });
        self.rebuild_scene(self.window.dpi())
    }

    pub(super) fn result_at(&self, x: i32, y: i32) -> Option<usize> {
        let x = u32::try_from(x).ok()?;
        let y = u32::try_from(y).ok()?;
        self.scene.as_ref()?.layout().hit_test_result(x, y)
    }

    pub(super) fn set_hovered_result(&mut self, hovered: Option<usize>) -> bool {
        self.scene
            .as_mut()
            .is_some_and(|scene| scene.set_hovered(hovered))
    }

    pub(super) fn select_result(&mut self, index: usize) -> Result<bool, AppError> {
        let changed = self.controller.select_index(index);
        if changed {
            self.rebuild_scene(self.window.dpi())?;
        }
        Ok(changed)
    }

    pub(super) fn submit(&mut self, owner: WindowHandle) {
        let selected = self.controller.selected_entry().cloned();
        self.hide();
        if let Some(entry) = selected {
            match launch_target(&entry.launch_target, None) {
                Ok(()) => {
                    let _ = self.controller.record_launch(&entry.launch_target);
                }
                Err(error) => {
                    show_error(
                        owner,
                        "Lotus Search",
                        &format!("Lotus could not open {}.\n\n{error}", entry.name),
                    );
                }
            }
        }
    }

    pub(super) fn advance_animation(&mut self) {
        self.presentation.advance();
        if let Some(scene) = &mut self.scene {
            let _ = scene.set_presentation_progress(self.presentation.progress());
        }
    }

    pub(super) fn sync_size(
        &mut self,
        dock: &DockWindow,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let desired = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidLauncherScene)?
            .desired_size();
        self.window
            .show_sized(dock.handle(), desired.width(), desired.height())?;
        if let Some(surface) = &mut self.surface {
            resize_launcher_surface(graphics, surface, SurfaceSize::from(desired))?;
        }
        Ok(())
    }

    pub(super) fn render(&mut self, graphics: &mut DeviceState) -> Result<bool, AppError> {
        if !self.visible {
            return Ok(false);
        }
        let scene = self.scene.as_ref().ok_or(AppError::InvalidLauncherScene)?;
        let surface = self
            .surface
            .as_mut()
            .ok_or(AppError::InvalidLauncherScene)?;
        match surface.render_scene(scene) {
            Ok(frame) => Ok(frame.needs_animation()),
            Err(SurfaceError::DeviceLost(_)) => {
                let _ = graphics.poll();
                graphics.recover()?;
                let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                surface.recover(device)?;
                Ok(surface.render_scene(scene)?.needs_animation())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn drain_events(&mut self) -> Vec<SearchEvent> {
        self.window.drain_events().collect()
    }

    pub(super) fn apply_settings(
        &mut self,
        settings: &DockSettings,
        dock: &DockWindow,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        lotus_windows::backdrop::apply_search_settings(self.window.handle(), settings);
        let next_theme = theme_for(settings);
        let theme_changed = self.theme != next_theme;
        self.theme = next_theme;
        let limit = usize::try_from(settings.search_result_limit).unwrap_or(8);
        if (self.controller.set_result_limit(limit) || theme_changed) && self.visible {
            self.rebuild_scene(self.window.dpi())?;
            self.sync_size(dock, graphics)?;
            let _ = self.render(graphics)?;
        }
        Ok(())
    }
}
