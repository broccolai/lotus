use lotus_core::activation::{ActivationDecision, decide_activation};
use lotus_core::dock::DockItem;
use lotus_core::window::{TrackedWindowKey, WindowId, WindowInfo};
use lotus_windows::activation::{
    ActivationError, execute_activation, force_window_close, request_window_close,
    switch_window,
};

#[derive(Debug, Eq, PartialEq)]
pub(super) enum ActivationOutcome {
    Focused(TrackedWindowKey),
    Minimized,
    Launched,
    TargetDisappeared,
    NoLiveTarget,
    ForegroundDenied,
    CloseRequested,
    AlreadyClosed,
}

impl ActivationOutcome {
    pub(super) const fn focused_key(&self) -> Option<TrackedWindowKey> {
        match self {
            Self::Focused(key) => Some(*key),
            Self::Minimized
            | Self::Launched
            | Self::TargetDisappeared
            | Self::NoLiveTarget
            | Self::ForegroundDenied
            | Self::CloseRequested
            | Self::AlreadyClosed => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeOutcome {
    Completed,
    Disappeared,
    ForegroundDenied,
}

pub(super) fn activate_exact(
    key: TrackedWindowKey,
) -> Result<ActivationOutcome, ActivationError> {
    activate_exact_with(key, native_switch)
}

pub(super) fn launch(item: &DockItem) -> Result<ActivationOutcome, ActivationError> {
    execute_activation(ActivationDecision::Launch, item)?;
    Ok(ActivationOutcome::Launched)
}

pub(super) fn activate_application(
    item: &DockItem,
    foreground: Option<WindowId>,
) -> Result<ActivationOutcome, ActivationError> {
    activate_application_with(
        item,
        foreground,
        |decision| native_activation(decision, item),
        || native_activation(ActivationDecision::Launch, item),
    )
}

pub(super) fn focus_application(
    item: &DockItem,
    preferred: Option<TrackedWindowKey>,
) -> Result<ActivationOutcome, ActivationError> {
    focus_application_with(
        item,
        preferred,
        &mut |decision| native_activation(decision, item),
        &mut || native_activation(ActivationDecision::Launch, item),
    )
}

fn focus_application_with(
    item: &DockItem,
    preferred: Option<TrackedWindowKey>,
    operation: &mut impl FnMut(
        ActivationDecision<TrackedWindowKey>,
    ) -> Result<NativeOutcome, ActivationError>,
    launch: &mut impl FnMut() -> Result<NativeOutcome, ActivationError>,
) -> Result<ActivationOutcome, ActivationError> {
    let keys = item.windows.iter().map(WindowInfo::key).collect::<Vec<_>>();
    let first = preferred
        .filter(|key| keys.contains(key))
        .or_else(|| keys.first().copied());
    let Some(first) = first else {
        return no_live_application_outcome(item, launch);
    };
    retry_or_launch(item, &keys, first, operation, launch)
}

pub(super) fn request_close(
    key: TrackedWindowKey,
    force: bool,
) -> Result<ActivationOutcome, ActivationError> {
    request_close_with(key, force, native_close)
}

fn activate_exact_with(
    key: TrackedWindowKey,
    operation: impl FnOnce(TrackedWindowKey) -> Result<NativeOutcome, ActivationError>,
) -> Result<ActivationOutcome, ActivationError> {
    match operation(key)? {
        NativeOutcome::Completed => Ok(ActivationOutcome::Focused(key)),
        NativeOutcome::Disappeared => {
            record_stale_target(
                "activation.stale_target",
                "window disappeared before activation",
            );
            Ok(ActivationOutcome::TargetDisappeared)
        }
        NativeOutcome::ForegroundDenied => Ok(ActivationOutcome::ForegroundDenied),
    }
}

fn activate_application_with(
    item: &DockItem,
    foreground: Option<WindowId>,
    mut operation: impl FnMut(
        ActivationDecision<TrackedWindowKey>,
    ) -> Result<NativeOutcome, ActivationError>,
    mut launch: impl FnMut() -> Result<NativeOutcome, ActivationError>,
) -> Result<ActivationOutcome, ActivationError> {
    let keys = item.windows.iter().map(WindowInfo::key).collect::<Vec<_>>();
    let foreground =
        foreground.and_then(|id| keys.iter().copied().find(|key| key.id == id));
    match decide_activation(&keys, foreground.as_ref()) {
        ActivationDecision::Launch => no_live_application_outcome(item, &mut launch),
        ActivationDecision::Minimize(key) => {
            match operation(ActivationDecision::Minimize(key))? {
                NativeOutcome::Completed => Ok(ActivationOutcome::Minimized),
                NativeOutcome::ForegroundDenied => Ok(ActivationOutcome::ForegroundDenied),
                NativeOutcome::Disappeared => {
                    record_stale_target(
                        "activation.stale_target",
                        "foreground window disappeared before minimization",
                    );
                    retry_or_launch(item, &keys, key, &mut operation, &mut launch)
                }
            }
        }
        ActivationDecision::Focus(first) => {
            retry_or_launch(item, &keys, first, &mut operation, &mut launch)
        }
    }
}

fn retry_or_launch(
    item: &DockItem,
    keys: &[TrackedWindowKey],
    first: TrackedWindowKey,
    operation: &mut impl FnMut(
        ActivationDecision<TrackedWindowKey>,
    ) -> Result<NativeOutcome, ActivationError>,
    launch: &mut impl FnMut() -> Result<NativeOutcome, ActivationError>,
) -> Result<ActivationOutcome, ActivationError> {
    for key in
        std::iter::once(first).chain(keys.iter().copied().filter(|key| *key != first))
    {
        match operation(ActivationDecision::Focus(key))? {
            NativeOutcome::Completed => {
                if key != first {
                    lotus_windows::diagnostics::record_diagnostic(
                        "activation.application_fallback",
                        "focused another live window for the application",
                    );
                }
                return Ok(ActivationOutcome::Focused(key));
            }
            NativeOutcome::Disappeared => record_stale_target(
                "activation.stale_target",
                "application target disappeared before activation",
            ),
            NativeOutcome::ForegroundDenied => {
                return Ok(ActivationOutcome::ForegroundDenied);
            }
        }
    }
    no_live_application_outcome(item, launch)
}

fn no_live_application_outcome(
    item: &DockItem,
    launch: &mut impl FnMut() -> Result<NativeOutcome, ActivationError>,
) -> Result<ActivationOutcome, ActivationError> {
    if item.is_pinned {
        return launch_outcome(launch);
    }
    lotus_windows::diagnostics::record_diagnostic(
        "activation.no_live_target",
        "unpinned application had no current live window",
    );
    Ok(ActivationOutcome::NoLiveTarget)
}

fn launch_outcome(
    launch: &mut impl FnMut() -> Result<NativeOutcome, ActivationError>,
) -> Result<ActivationOutcome, ActivationError> {
    match launch()? {
        NativeOutcome::Completed => Ok(ActivationOutcome::Launched),
        NativeOutcome::Disappeared => Ok(ActivationOutcome::NoLiveTarget),
        NativeOutcome::ForegroundDenied => Ok(ActivationOutcome::ForegroundDenied),
    }
}

fn request_close_with(
    key: TrackedWindowKey,
    force: bool,
    operation: impl FnOnce(TrackedWindowKey, bool) -> Result<NativeOutcome, ActivationError>,
) -> Result<ActivationOutcome, ActivationError> {
    match operation(key, force)? {
        NativeOutcome::Completed => Ok(ActivationOutcome::CloseRequested),
        NativeOutcome::Disappeared => {
            lotus_windows::diagnostics::record_diagnostic(
                "activation.already_completed_close",
                "window disappeared before close could be delivered",
            );
            Ok(ActivationOutcome::AlreadyClosed)
        }
        NativeOutcome::ForegroundDenied => Ok(ActivationOutcome::ForegroundDenied),
    }
}

fn native_switch(key: TrackedWindowKey) -> Result<NativeOutcome, ActivationError> {
    switch_window(key)
        .map(|()| NativeOutcome::Completed)
        .or_else(classify_native_error)
}

fn native_activation(
    decision: ActivationDecision<TrackedWindowKey>,
    item: &DockItem,
) -> Result<NativeOutcome, ActivationError> {
    execute_activation(decision, item)
        .map(|()| NativeOutcome::Completed)
        .or_else(classify_native_error)
}

fn native_close(
    key: TrackedWindowKey,
    force: bool,
) -> Result<NativeOutcome, ActivationError> {
    let result = if force {
        force_window_close(key)
    } else {
        request_window_close(key)
    };
    result
        .map(|()| NativeOutcome::Completed)
        .or_else(classify_native_error)
}

fn classify_native_error(error: ActivationError) -> Result<NativeOutcome, ActivationError> {
    match error {
        ActivationError::MissingWindow(_) => Ok(NativeOutcome::Disappeared),
        ActivationError::IdentityMismatch { .. } => {
            record_stale_target(
                "activation.identity_mismatch",
                "HWND was recycled by another process",
            );
            Ok(NativeOutcome::Disappeared)
        }
        ActivationError::RetiredWindow(_) => {
            record_stale_target(
                "activation.identity_mismatch",
                "HWND no longer has the tracker-published incarnation",
            );
            Ok(NativeOutcome::Disappeared)
        }
        ActivationError::ForegroundDenied(_) => Ok(NativeOutcome::ForegroundDenied),
        error => Err(error),
    }
}

fn record_stale_target(context: &str, message: &str) {
    lotus_windows::diagnostics::record_diagnostic(context, message);
}
