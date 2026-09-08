use std::fmt::Write;

use lotus_windows::diagnostics::{SupportReport, capture_support_report};

use super::settings_events::SettingsEventContext;

pub(super) fn capture(context: &SettingsEventContext<'_>) -> SupportReport {
    let integration = context
        .integration
        .diagnostic_snapshot(context.graphics, context.auxiliary);
    let runtime = runtime_state(context);

    capture_support_report(
        context.dock_model.settings(),
        &integration,
        context.window_tracker,
        context.dock_model.items(),
        context.dock_model.application_view().assignments(),
        &runtime,
    )
}

fn runtime_state(context: &SettingsEventContext<'_>) -> String {
    const MAX_SCENE_ROWS: usize = 512;

    let dock = &context.primary_dock;
    let model = &context.dock_model;
    let view = model.application_view();
    let scene = model.scene();
    let size = scene.desired_size();
    let (dirty, animating, visible) = context.auxiliary.diagnostic_surface_masks();
    let mut output = format!(
        "startup_mode={:?}\n\
         graphics_generation={}\n\
         dock_visible={} dock_occluded={} dock_ready={} dock_dirty={} dock_animating={}\n\
         auxiliary_dirty_mask={} auxiliary_animating_mask={} auxiliary_visible_mask={}\n\
         dock_revision={} application_window_revision={} application_binding_revision={}\n\
         scene_width={} scene_height={} scene_items={} scene_truncated={}\n",
        context.startup_mode,
        context.graphics.generation(),
        dock.window().is_visible(),
        dock.window().is_fullscreen_occluded(),
        dock.presentation_ready(),
        dock.is_dirty(),
        dock.is_animating(),
        dirty,
        animating,
        visible,
        model.revision(),
        view.window_revision(),
        view.binding_revision(),
        size.width(),
        size.height(),
        scene.items().len(),
        scene.items().len() > MAX_SCENE_ROWS,
    );

    for (index, item) in scene.items().iter().take(MAX_SCENE_ROWS).enumerate() {
        let _ = writeln!(
            output,
            "scene_index={index} dock_index={} exiting={} model_present={}",
            item.source_index(),
            item.is_exiting(),
            model.items().get(item.source_index()).is_some(),
        );
    }
    output
}
