use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Instant;

use lotus_core::launcher_model::{CursorMove as ModelCursorMove, QueryEdit, SelectionMove};
use lotus_core::settings::DockSettings;
use lotus_search::command::CommandId;
use lotus_search::controller::{SearchController, SearchPresentation};
use lotus_search::usage::SearchUsageStore;
use lotus_settings::appearance::theme_for;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::{FrameOutcome, FramePass, ScheduledSurface};
use lotus_ui::theme::Theme;
use lotus_windows::WindowHandle;
use lotus_windows::activation::launch_target;
use lotus_windows::clipboard::read_text;
use lotus_windows::clock::local_time;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::launcher_surface::LauncherCompositionSurfaceState;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{DeviceState, GraphicsDevice, SurfaceError, SurfaceSize};
use lotus_windows::icon_hydrator::{LauncherIconClient, LauncherIconRequest};
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::window::{
    CursorMove as WindowCursorMove, DockContextRequest, DockWindow, SearchEdit,
    SearchEvent, SearchWindow, SelectionDirection, SignedPoint,
};

use crate::app::AppError;
use crate::app::applications::PreparedLauncherCatalog;
use crate::app::dock::DockRuntime;
use crate::app::runtime::resize_launcher_surface;

const MAX_HYDRATED_ICONS: usize = 64;

type DockIcon = lotus_ui::icon::Icon<EmbeddedIcon>;
type LauncherResult = lotus_search::scene::LauncherResult<EmbeddedIcon>;
type LauncherScene = lotus_search::scene::LauncherScene<EmbeddedIcon>;

pub(super) struct LauncherRuntime {
    pub(super) window: SearchWindow,
    pub(super) controller: SearchController,
    icon_hydrator: LauncherIconClient,
    hydrated_icons: BTreeMap<LauncherIconKey, lotus_ui::icon::RasterIcon>,
    icon_generation: u64,
    icon_settings_revision: u64,
    icon_request_signature: Option<Vec<LauncherIconKey>>,
    catalog_projection: Option<u64>,
    pub(super) scene: Option<LauncherScene>,
    pub(super) surface: Option<ScheduledSurface<LauncherCompositionSurfaceState>>,
    pub(super) presentation: SearchPresentation,
    pub(super) visible: bool,
    last_applied_size: Option<SurfaceSize>,
    child_popup_open: bool,
    theme: Theme,
    use_24_hour_time: bool,
    settings: DockSettings,
}

pub(super) enum LauncherSubmission {
    Command(CommandId),
    Calculation(String),
}

pub(super) enum LauncherEventOutcome {
    None,
    Submission(LauncherSubmission),
    OpenFileLocation { anchor: SignedPoint, path: String },
}

impl LauncherRuntime {
    pub(super) fn new(
        window: SearchWindow,
        settings: DockSettings,
        theme: &Theme,
        usage: lotus_core::search::SearchUsage,
        usage_store: SearchUsageStore,
        icon_hydrator: LauncherIconClient,
    ) -> Self {
        Self {
            window,
            controller: SearchController::new(
                usize::try_from(settings.search_result_limit).unwrap_or(8),
                usage,
                usage_store,
            ),
            icon_hydrator,
            hydrated_icons: BTreeMap::new(),
            icon_generation: 0,
            icon_settings_revision: 0,
            icon_request_signature: None,
            catalog_projection: None,
            scene: None,
            surface: None,
            presentation: SearchPresentation::default(),
            visible: false,
            last_applied_size: None,
            child_popup_open: false,
            theme: *theme,
            use_24_hour_time: settings.use_24_hour_time,
            settings,
        }
    }

    pub(super) const fn is_visible(&self) -> bool {
        self.visible
    }

    pub(super) fn diagnostic_surface_state(&self) -> (bool, bool, bool) {
        let surface = self.surface.as_ref();
        (
            surface.is_some_and(ScheduledSurface::is_dirty),
            surface.is_some_and(ScheduledSurface::is_animating),
            self.visible,
        )
    }

    pub(super) fn open(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        catalog: PreparedLauncherCatalog,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if self.surface.is_none() && graphics.ready().is_none() {
            return Err(AppError::GraphicsUnavailable);
        }

        self.prepare_catalog(dock_model, catalog);
        self.presentation.begin();
        self.rebuild_scene(dock.dpi())?;

        let scene = self.scene.as_ref().ok_or(AppError::InvalidLauncherScene)?;
        let desired = scene.desired_size();
        self.window
            .open(dock.handle(), desired.width(), desired.height())?;
        if self.window.dpi() != scene.dpi() {
            self.rebuild_scene(self.window.dpi())?;
            let corrected = self
                .scene
                .as_ref()
                .ok_or(AppError::InvalidLauncherScene)?
                .desired_size();
            self.window.apply_geometry(
                dock.handle(),
                corrected.width(),
                corrected.height(),
            )?;
        }

        let desired = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidLauncherScene)?
            .desired_size();
        let size = SurfaceSize::new(desired.width(), desired.height())
            .ok_or(AppError::ZeroSizedSurface)?;
        if let Some(surface) = &mut self.surface {
            resize_launcher_surface(graphics, surface.value_mut(), size)?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ScheduledSurface::new(
                LauncherCompositionSurfaceState::create(
                    device,
                    self.window.handle(),
                    size,
                )?,
            ));
        }
        self.visible = true;
        self.last_applied_size = Some(size);
        Ok(())
    }

    pub(super) fn refresh_catalog_if_ready(
        &mut self,
        dock: &DockWindow,
        catalog: PreparedLauncherCatalog,
        graphics: &mut DeviceState,
    ) -> Result<bool, AppError> {
        let Some(generation) = catalog.generation else {
            return Ok(false);
        };

        if !self.controller.refresh_catalog(generation, catalog.catalog) {
            return Ok(false);
        }

        if !self.visible {
            return Ok(true);
        }

        let size_before = self.desired_size();
        self.rebuild_scene(self.window.dpi())?;
        if self.desired_size() != size_before {
            self.apply_geometry_if_changed(dock, graphics, false)?;
        }

        Ok(true)
    }

    pub(super) fn catalog_generation(&self) -> Option<u64> {
        self.controller.catalog_generation()
    }

    pub(super) fn drain_hydrated_icons(
        &mut self,
        results: impl IntoIterator<Item = lotus_windows::icon_hydrator::HydratedLauncherIcon>,
    ) -> Result<bool, AppError> {
        let Some(dpi) = self.scene.as_ref().map(LauncherScene::dpi) else {
            return Ok(false);
        };
        let icon_size = launcher_icon_size(dpi);
        let current_keys = self
            .controller
            .results()
            .iter()
            .map(|entry| {
                LauncherIconKey::from_entry(entry, icon_size, self.icon_settings_revision)
            })
            .collect::<Vec<_>>();
        let mut changed = false;
        for result in results {
            if result.generation != self.icon_generation
                || result.pixel_size != icon_size
                || result.settings_revision != self.icon_settings_revision
            {
                continue;
            }
            let key = LauncherIconKey {
                identity: result.identity,
                pixel_size: result.pixel_size,
                settings_revision: result.settings_revision,
            };
            if !current_keys.contains(&key) {
                continue;
            }
            if let Some(icon) = result.icon
                && self.hydrated_icons.get(&key) != Some(&icon)
            {
                if self.hydrated_icons.len() >= MAX_HYDRATED_ICONS {
                    let _discarded = self.hydrated_icons.pop_first();
                }
                self.hydrated_icons.insert(key, icon);
                changed = true;
            }
        }
        if changed {
            self.rebuild_scene(dpi)?;
            self.invalidate();
        }
        Ok(changed)
    }

    pub(super) fn hide(&mut self) {
        self.window.hide();
        self.visible = false;
        self.last_applied_size = None;
        self.child_popup_open = false;
        self.presentation.finish();
        if let Some(surface) = &mut self.surface {
            surface.stop_animation();
        }
    }

    pub(super) fn suspend_for_child_popup(&mut self) {
        self.child_popup_open = true;
        self.window.suspend_for_child_popup();
    }

    pub(super) fn resume_after_child_popup_if_visible(&mut self) {
        self.child_popup_open = false;
        if self.visible {
            self.window.resume_after_child_popup();
        }
    }

    pub(super) fn focus_if_visible(&mut self) {
        if self.visible && !self.child_popup_open {
            let _ = self.window.focus();
        }
    }

    pub(super) fn rebuild_scene(&mut self, dpi: u32) -> Result<(), AppError> {
        let results = self.results_with_icons(dpi);
        let mut scene = LauncherScene::new(
            dpi,
            self.controller.query(),
            self.controller.mode(),
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
        let _ = scene.set_footer_time(local_time(self.use_24_hour_time));
        let _ = scene.set_presentation_progress(self.presentation.progress());
        self.scene = Some(scene);
        self.request_visible_icons();
        Ok(())
    }

    fn iconless_results(&self) -> Vec<LauncherResult> {
        if let Some(calculation) = self.controller.selected_calculation() {
            return vec![LauncherResult::calculator(
                format!("= {}", calculation.value),
                DockIcon::Embedded(EmbeddedIcon::FluentCalculator),
            )];
        }
        if self.controller.is_command_mode() {
            return self
                .controller
                .commands()
                .iter()
                .map(|entry| LauncherResult::command(entry.title, command_icon(entry.id)))
                .collect();
        }

        self.controller
            .results()
            .iter()
            .map(|entry| LauncherResult::new(&entry.name))
            .collect()
    }

    fn results_with_icons(&self, dpi: u32) -> Vec<LauncherResult> {
        if self.controller.is_command_mode() || self.controller.is_calculator_mode() {
            return self.iconless_results();
        }

        let icon_size = launcher_icon_size(dpi);
        self.controller
            .results()
            .iter()
            .map(|entry| {
                let key = LauncherIconKey::from_entry(
                    entry,
                    icon_size,
                    self.icon_settings_revision,
                );
                self.hydrated_icons.get(&key).map_or_else(
                    || LauncherResult::new(&entry.name),
                    |icon| {
                        LauncherResult::with_icon(
                            &entry.name,
                            DockIcon::Raster(icon.clone()),
                        )
                    },
                )
            })
            .collect()
    }

    fn prepare_catalog(
        &mut self,
        dock_model: &DockRuntime,
        catalog: PreparedLauncherCatalog,
    ) {
        let projection = catalog_projection(dock_model);
        let ready_generation = catalog.generation;
        if self.catalog_projection == Some(projection)
            && self.controller.catalog_generation() == ready_generation
        {
            self.controller.restart();
            return;
        }

        self.controller.begin(catalog.generation, catalog.catalog);
        self.catalog_projection = Some(projection);
    }

    fn request_visible_icons(&mut self) {
        let Some(scene) = &self.scene else {
            self.icon_hydrator.request_launcher(Vec::new());
            return;
        };
        if self.controller.is_command_mode() || self.controller.is_calculator_mode() {
            self.icon_request_signature = None;
            self.icon_hydrator.request_launcher(Vec::new());
            return;
        }
        let icon_size = scene.result_icon_size().get();
        let keys = self
            .controller
            .results()
            .iter()
            .map(|entry| {
                LauncherIconKey::from_entry(entry, icon_size, self.icon_settings_revision)
            })
            .collect::<Vec<_>>();
        if self.icon_request_signature.as_ref() == Some(&keys) {
            return;
        }
        self.icon_request_signature = Some(keys.clone());
        self.icon_generation = self.icon_generation.wrapping_add(1);

        let requests = self
            .controller
            .results()
            .iter()
            .zip(keys)
            .filter(|(_, key)| !self.hydrated_icons.contains_key(key))
            .map(|(entry, key)| LauncherIconRequest {
                generation: self.icon_generation,
                identity: key.identity,
                icon_source: entry.icon_source.clone().into(),
                custom_image_path: crate::app::icon_override::application_icon_path(
                    &self.settings,
                    entry.app_user_model_id.as_deref(),
                    Some(&entry.launch_target),
                    Path::new(&entry.icon_source),
                ),
                pixel_size: icon_size,
                settings_revision: self.icon_settings_revision,
            })
            .collect();
        self.icon_hydrator.request_launcher(requests);
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

    pub(super) fn handle_event(
        &mut self,
        event: SearchEvent,
        dock: &DockWindow,
        graphics: &mut DeviceState,
        dock_model: &DockRuntime,
    ) -> Result<LauncherEventOutcome, AppError> {
        if let SearchEvent::ContextMenuRequested(request) = event {
            let context = self.file_location_context(request)?;
            return Ok(match context {
                Some((anchor, path)) => {
                    LauncherEventOutcome::OpenFileLocation { anchor, path }
                }
                None => LauncherEventOutcome::None,
            });
        }

        let size_before = self.desired_size();
        let mut presentation_changed = false;
        let mut submission = None;
        match event {
            SearchEvent::TextInput(character) => {
                self.controller.push_character(character);
                self.rebuild_scene(self.window.dpi())?;
                presentation_changed = true;
            }
            SearchEvent::Edit(edit) => {
                if self.controller.edit_query(model_query_edit(edit)) {
                    self.rebuild_scene(self.window.dpi())?;
                    presentation_changed = true;
                }
            }
            SearchEvent::PasteRequested => {
                if let Ok(text) = read_text()
                    && self.controller.insert_text(&text)
                {
                    self.rebuild_scene(self.window.dpi())?;
                    presentation_changed = true;
                }
            }
            SearchEvent::MoveSelection(direction) => {
                self.move_selection(direction)?;
                presentation_changed = true;
            }
            SearchEvent::DismissRequested => self.hide(),
            SearchEvent::SubmitRequested => submission = self.submit(dock.handle()),
            SearchEvent::Resized { width, height } => {
                if let (Some(size), Some(surface)) =
                    (SurfaceSize::new(width, height), self.surface.as_mut())
                    && self.last_applied_size != Some(size)
                {
                    resize_launcher_surface(graphics, surface.value_mut(), size)?;
                    self.last_applied_size = Some(size);
                    presentation_changed = true;
                }
            }
            SearchEvent::DpiChanged { dpi } => {
                self.rebuild_scene(dpi)?;
                presentation_changed = true;
            }
            SearchEvent::ClockRefreshRequested => {
                presentation_changed = self.scene.as_mut().is_some_and(|scene| {
                    scene
                        .set_footer_time(local_time(dock_model.settings().use_24_hour_time))
                });
            }
            SearchEvent::FocusRefreshRequested => {
                if !self.child_popup_open {
                    let _ = self.window.focus();
                }
            }
            SearchEvent::RenderRequested => presentation_changed = true,
            SearchEvent::PointerMoved { x, y } => {
                let hovered = self.result_at(x, y);
                presentation_changed = self.set_hovered_result(hovered);
            }
            SearchEvent::PointerLeft => {
                presentation_changed = self.set_hovered_result(None);
            }
            SearchEvent::PointerReleased { x, y } => {
                if let Some(index) = self.result_at(x, y) {
                    let _ = self.select_result(index)?;
                    submission = self.submit(dock.handle());
                }
            }
            SearchEvent::ContextMenuRequested(_) => {
                unreachable!("handled before event routing")
            }
        }

        if presentation_changed && self.is_visible() {
            if self.desired_size() != size_before {
                self.apply_geometry_if_changed(dock, graphics, false)?;
            }
            self.invalidate();
        }
        Ok(submission.map_or(LauncherEventOutcome::None, LauncherEventOutcome::Submission))
    }

    pub(super) fn result_at(&self, x: i32, y: i32) -> Option<usize> {
        let x = u32::try_from(x).ok()?;
        let y = u32::try_from(y).ok()?;
        let started = Instant::now();
        let result = self.scene.as_ref()?.layout().hit_test_result(x, y);
        METRICS.record_layout(LayoutOperation::LauncherHitTest, started.elapsed());
        result
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

    pub(super) fn file_location_context(
        &mut self,
        request: DockContextRequest,
    ) -> Result<Option<(SignedPoint, String)>, AppError> {
        let DockContextRequest::Pointer { screen, client, .. } = request else {
            return Ok(None);
        };
        if self.controller.is_command_mode() || self.controller.is_calculator_mode() {
            return Ok(None);
        }
        let Some(index) = self.result_at(client.x, client.y) else {
            return Ok(None);
        };
        let Some(entry) = self.controller.results().get(index) else {
            return Ok(None);
        };
        let Some(path) = lotus_windows::launch::application_file_location(
            &entry.launch_target,
            &entry.icon_source,
        ) else {
            return Ok(None);
        };
        let path = path.to_string_lossy().into_owned();

        let _ = self.select_result(index)?;
        self.invalidate();
        Ok(Some((screen, path)))
    }

    pub(super) fn submit(&mut self, owner: WindowHandle) -> Option<LauncherSubmission> {
        let submission = self
            .controller
            .selected_command()
            .map(LauncherSubmission::Command)
            .or_else(|| {
                self.controller.selected_calculation().map(|calculation| {
                    LauncherSubmission::Calculation(calculation.value.clone())
                })
            });
        let selected = self.controller.selected_entry().cloned();
        self.hide();
        if submission.is_none()
            && let Some(entry) = selected
        {
            match launch_target(&entry.launch_target, entry.invocation_arguments()) {
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
        submission
    }

    pub(super) fn advance_animation(&mut self) {
        if !self
            .surface
            .as_ref()
            .is_some_and(ScheduledSurface::is_animating)
        {
            return;
        }

        self.presentation.advance();
        if let Some(scene) = &mut self.scene {
            let _ = scene.set_presentation_progress(self.presentation.progress());
        }
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

    pub(super) fn refresh_placement(
        &mut self,
        dock: &DockWindow,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.apply_geometry_if_changed(dock, graphics, true)
    }

    fn apply_geometry_if_changed(
        &mut self,
        dock: &DockWindow,
        graphics: &mut DeviceState,
        reposition: bool,
    ) -> Result<(), AppError> {
        let desired = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidLauncherScene)?
            .desired_size();
        let size = SurfaceSize::new(desired.width(), desired.height())
            .ok_or(AppError::ZeroSizedSurface)?;
        let size_changed = self.last_applied_size != Some(size);
        if size_changed || reposition {
            self.window
                .apply_geometry(dock.handle(), desired.width(), desired.height())?;
        }
        if size_changed && let Some(surface) = &mut self.surface {
            resize_launcher_surface(graphics, surface.value_mut(), size)?;
        }
        self.last_applied_size = Some(size);
        Ok(())
    }

    fn desired_size(&self) -> Option<lotus_search::scene::LauncherSize> {
        self.scene.as_ref().map(LauncherScene::desired_size)
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.visible {
            if let Some(surface) = &mut self.surface {
                surface.stop_animation();
            }
            return Ok(());
        }
        let scene = self.scene.as_ref().ok_or(AppError::InvalidLauncherScene)?;
        let surface = self
            .surface
            .as_mut()
            .ok_or(AppError::InvalidLauncherScene)?;
        let content = scene.render_presentation(EmbeddedIcon::FluentSearch);
        let motion = scene.presentation();
        let render = |surface: &mut LauncherCompositionSurfaceState| {
            surface.render_scene(
                &content,
                motion.scale,
                motion.opacity,
                scene.needs_animation(),
            )
        };
        pass.render(surface, |surface| match render(surface) {
            Ok(FrameResult::Presented { needs_animation }) => {
                Ok(FrameOutcome::complete(needs_animation))
            }
            Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
            Err(SurfaceError::DeviceLost(loss)) => {
                graphics.mark_lost(loss);
                Ok(FrameOutcome::complete(false))
            }
            Err(error) => Err(error.into()),
        })
    }

    pub(super) fn drain_events(&mut self) -> Vec<SearchEvent> {
        self.window.drain_events().collect()
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.window.has_pending_events()
    }

    pub(super) fn apply_settings(
        &mut self,
        settings: &DockSettings,
        dock: &DockWindow,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        lotus_windows::backdrop::apply_search_settings(self.window.handle(), settings);
        self.settings = settings.clone();
        self.hydrated_icons.clear();
        self.icon_settings_revision = self.icon_settings_revision.wrapping_add(1);
        self.icon_request_signature = None;
        self.catalog_projection = None;
        let next_theme = theme_for(settings);
        let theme_changed = self.theme != next_theme;
        self.theme = next_theme;
        let time_format_changed = self.use_24_hour_time != settings.use_24_hour_time;
        self.use_24_hour_time = settings.use_24_hour_time;
        let limit = usize::try_from(settings.search_result_limit).unwrap_or(8);
        let result_limit_changed = self.controller.set_result_limit(limit);
        if (result_limit_changed || theme_changed || time_format_changed) && self.visible {
            let size_before = self.desired_size();
            self.rebuild_scene(self.window.dpi())?;
            if result_limit_changed && self.desired_size() != size_before {
                self.apply_geometry_if_changed(dock, graphics, false)?;
            }
            if let Some(surface) = &mut self.surface {
                surface.invalidate();
            }
        }
        Ok(())
    }
}

const fn model_query_edit(edit: SearchEdit) -> QueryEdit {
    match edit {
        SearchEdit::DeleteBackward => QueryEdit::DeleteBackward,
        SearchEdit::DeletePreviousWord => QueryEdit::DeletePreviousWord,
        SearchEdit::DeleteForward => QueryEdit::DeleteForward,
        SearchEdit::MoveCursor(movement) => QueryEdit::MoveCursor(match movement {
            WindowCursorMove::Home => ModelCursorMove::Home,
            WindowCursorMove::End => ModelCursorMove::End,
            WindowCursorMove::Previous => ModelCursorMove::Previous,
            WindowCursorMove::Next => ModelCursorMove::Next,
        }),
        SearchEdit::SelectAll => QueryEdit::SelectAll,
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LauncherIconKey {
    identity: String,
    pixel_size: u32,
    settings_revision: u64,
}

impl LauncherIconKey {
    fn from_entry(
        entry: &lotus_core::search::ApplicationEntry,
        pixel_size: u32,
        settings_revision: u64,
    ) -> Self {
        Self {
            identity: format!(
                "{}\u{1f}{}\u{1f}{}",
                entry.app_user_model_id.as_deref().unwrap_or_default(),
                entry.launch_target,
                entry.icon_source,
            ),
            pixel_size,
            settings_revision,
        }
    }
}

fn catalog_projection(dock_model: &DockRuntime) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for item in dock_model.items() {
        item.id.hash(&mut hasher);
        item.display_name.hash(&mut hasher);
        item.launch_target.hash(&mut hasher);
        item.executable_path.hash(&mut hasher);
        item.app_user_model_id.hash(&mut hasher);
        item.is_pinned.hash(&mut hasher);
    }
    dock_model.settings().hidden_executables.hash(&mut hasher);
    hasher.finish()
}

fn launcher_icon_size(dpi: u32) -> u32 {
    lotus_ui::geometry::DpiScale::from_system(dpi).physical(26)
}

const fn command_icon(command: CommandId) -> DockIcon {
    let asset = match command {
        CommandId::OpenSettings => EmbeddedIcon::FluentSettings,
        CommandId::OpenVolumeMixer => EmbeddedIcon::FluentVolume,
        CommandId::OpenNotificationArea => EmbeddedIcon::FluentTray,
        CommandId::ShowDesktop => EmbeddedIcon::FluentDesktop,
        CommandId::LockComputer => EmbeddedIcon::FluentLock,
        CommandId::RestartComputer => EmbeddedIcon::FluentRestart,
        CommandId::ShutDownComputer => EmbeddedIcon::FluentPower,
        CommandId::QuitLotus => EmbeddedIcon::FluentDismiss,
    };
    DockIcon::Embedded(asset)
}
