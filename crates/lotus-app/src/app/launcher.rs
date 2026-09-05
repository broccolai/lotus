mod surface;

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Instant;

use lotus_core::launcher_model::{CursorMove as ModelCursorMove, QueryEdit, SelectionMove};
use lotus_core::search::ApplicationEntry;
use lotus_core::settings::DockSettings;
use lotus_search::command::CommandId;
use lotus_search::controller::{SearchController, SearchPresentation};
use lotus_settings::appearance::theme_for;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::FramePass;
use lotus_ui::theme::Theme;
use lotus_windows::clock::local_time;
use lotus_windows::graphics::{DeviceState, GraphicsDevice};
use lotus_windows::icon_hydrator::{LauncherIconClient, LauncherIconRequest};
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::window::{
    CursorMove as WindowCursorMove, DockContextRequest, DockWindow, SearchEdit,
    SearchEvent, SearchWindow, SelectionDirection, SignedPoint,
};
use surface::LauncherSurface;

use crate::app::AppError;
use crate::app::applications::PreparedLauncherCatalog;
use crate::app::dock::DockRuntime;
use crate::app::search_usage::SearchUsageStore;

const MAX_HYDRATED_ICONS: usize = 64;

type DockIcon = lotus_ui::icon::Icon<EmbeddedIcon>;
type LauncherResult = lotus_search::scene::LauncherResult<EmbeddedIcon>;
type LauncherScene = lotus_search::scene::LauncherScene<EmbeddedIcon>;

pub(super) struct LauncherRuntime {
    surface: LauncherSurface,
    controller: SearchController,
    usage_store: SearchUsageStore,
    icon_hydrator: LauncherIconClient,
    hydrated_icons: BTreeMap<LauncherIconKey, lotus_ui::icon::RasterIcon>,
    icon_generation: u64,
    icon_settings_revision: u64,
    icon_request_signature: Option<Vec<LauncherIconKey>>,
    catalog_projection: Option<u64>,
    scene: Option<LauncherScene>,
    presentation: SearchPresentation,
    theme: Theme,
    use_24_hour_time: bool,
    settings: DockSettings,
}

pub(super) enum LauncherSubmission {
    Command(CommandId),
    Calculation(String),
    Application(ApplicationEntry),
}

pub(super) enum LauncherEventOutcome {
    None,
    PasteRequested,
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
            surface: LauncherSurface::new(window),
            controller: SearchController::new(
                usize::try_from(settings.search_result_limit).unwrap_or(8),
                usage,
            ),
            usage_store,
            icon_hydrator,
            hydrated_icons: BTreeMap::new(),
            icon_generation: 0,
            icon_settings_revision: 0,
            icon_request_signature: None,
            catalog_projection: None,
            scene: None,
            presentation: SearchPresentation::default(),
            theme: *theme,
            use_24_hour_time: settings.use_24_hour_time,
            settings,
        }
    }

    pub(super) const fn is_visible(&self) -> bool {
        self.surface.is_visible()
    }

    pub(super) fn diagnostic_surface_state(&self) -> (bool, bool, bool) {
        self.surface.diagnostic_state()
    }

    pub(super) fn open(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        catalog: PreparedLauncherCatalog,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.surface.has_graphics_surface() && graphics.ready().is_none() {
            return Err(AppError::GraphicsUnavailable);
        }

        self.prepare_catalog(dock_model, catalog);
        self.presentation.begin();
        self.rebuild_scene(dock.dpi())?;

        let scene = self.scene.as_ref().ok_or(AppError::InvalidLauncherScene)?;
        let desired = scene.desired_size();
        self.surface.open_window(dock, desired)?;
        if self.surface.dpi() != scene.dpi() {
            self.rebuild_scene(self.surface.dpi())?;
            let corrected = self
                .scene
                .as_ref()
                .ok_or(AppError::InvalidLauncherScene)?
                .desired_size();
            self.surface.correct_open_geometry(dock, corrected)?;
        }

        let desired = self
            .scene
            .as_ref()
            .ok_or(AppError::InvalidLauncherScene)?
            .desired_size();
        self.surface.commit_open(desired, graphics)?;
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

        if !self.is_visible() {
            return Ok(true);
        }

        let size_before = self.desired_size();
        self.rebuild_scene(self.surface.dpi())?;
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
        self.presentation.finish();
        self.surface.hide();
    }

    pub(super) fn suspend_for_child_popup(&mut self) {
        self.surface.suspend_for_child_popup();
    }

    pub(super) fn resume_after_child_popup_if_visible(&mut self, restore_focus: bool) {
        self.surface
            .resume_after_child_popup_if_visible(restore_focus);
    }

    pub(super) fn focus_if_visible(&mut self) {
        self.surface.focus_if_visible();
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
        self.rebuild_scene(self.surface.dpi())
    }

    pub(super) fn handle_event(
        &mut self,
        event: SearchEvent,
        dock: &DockWindow,
        graphics: &mut DeviceState,
        dock_model: &DockRuntime,
    ) -> Result<LauncherEventOutcome, AppError> {
        if !self.is_visible() {
            return Ok(LauncherEventOutcome::None);
        }
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
                self.rebuild_scene(self.surface.dpi())?;
                presentation_changed = true;
            }
            SearchEvent::Edit(edit) => {
                if self.controller.edit_query(model_query_edit(edit)) {
                    self.rebuild_scene(self.surface.dpi())?;
                    presentation_changed = true;
                }
            }
            SearchEvent::PasteRequested => return Ok(LauncherEventOutcome::PasteRequested),
            SearchEvent::MoveSelection(direction) => {
                self.move_selection(direction)?;
                presentation_changed = true;
            }
            SearchEvent::DismissRequested(request) => {
                if self.surface.accepts_dismiss(request) {
                    self.hide();
                }
            }
            SearchEvent::SubmitRequested => submission = self.submit(),
            SearchEvent::Resized { width, height } => {
                if self.surface.resize(graphics, width, height)? {
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
                self.surface.focus_if_visible();
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
                    submission = self.submit();
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
            self.rebuild_scene(self.surface.dpi())?;
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

    pub(super) fn paste(
        &mut self,
        text: &str,
        dock: &DockWindow,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.is_visible() || !self.controller.insert_text(text) {
            return Ok(());
        }

        let size_before = self.desired_size();
        self.rebuild_scene(self.surface.dpi())?;
        if self.desired_size() != size_before {
            self.apply_geometry_if_changed(dock, graphics, false)?;
        }
        self.invalidate();
        Ok(())
    }

    pub(super) fn submit(&mut self) -> Option<LauncherSubmission> {
        let submission = self
            .controller
            .selected_command()
            .map(LauncherSubmission::Command)
            .or_else(|| {
                self.controller.selected_calculation().map(|calculation| {
                    LauncherSubmission::Calculation(calculation.value.clone())
                })
            })
            .or_else(|| {
                self.controller
                    .selected_entry()
                    .cloned()
                    .map(LauncherSubmission::Application)
            });
        self.hide();
        submission
    }

    pub(super) fn record_successful_launch(&mut self, launch_target: &str) {
        if self.controller.record_launch(launch_target) {
            let _ = self.usage_store.save(self.controller.usage());
        }
    }

    pub(super) fn advance_animation(&mut self) {
        if !self.surface.is_animating() {
            return;
        }

        self.presentation.advance();
        if let Some(scene) = &mut self.scene {
            let _ = scene.set_presentation_progress(self.presentation.progress());
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.surface.invalidate();
    }

    pub(super) fn recover_surface(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        self.surface.recover(device)
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
        self.surface
            .apply_geometry(dock, desired, graphics, reposition)
    }

    fn desired_size(&self) -> Option<lotus_search::scene::LauncherSize> {
        self.scene.as_ref().map(LauncherScene::desired_size)
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.is_visible() {
            self.surface.stop_animation();
            return Ok(());
        }
        let scene = self.scene.as_ref().ok_or(AppError::InvalidLauncherScene)?;
        self.surface.render_frame(pass, graphics, scene)
    }

    pub(super) fn drain_events(&mut self) -> Vec<SearchEvent> {
        self.surface.drain_events()
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.surface.has_pending_events()
    }

    pub(super) fn apply_settings(
        &mut self,
        settings: &DockSettings,
        dock: &DockWindow,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.surface.use_material(settings);
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
        if (result_limit_changed || theme_changed || time_format_changed)
            && self.is_visible()
        {
            let size_before = self.desired_size();
            self.rebuild_scene(self.surface.dpi())?;
            if result_limit_changed && self.desired_size() != size_before {
                self.apply_geometry_if_changed(dock, graphics, false)?;
            }
            self.surface.invalidate();
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
        entry: &ApplicationEntry,
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
