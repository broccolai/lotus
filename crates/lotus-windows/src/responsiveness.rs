use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::process_resources::ProcessResourceSample;

mod record;
mod report;
mod snapshot;

const HISTOGRAM_BUCKETS: usize = 8;
const UI_PHASES: usize = 9;
const TRACKER_UI_PHASES: usize = 5;
const CACHE_CLASSES: usize = 7;
const LAYOUT_OPERATIONS: usize = 10;
const SLOW_UI_EVENTS: usize = 48;
const HISTOGRAM_LIMITS_US: [u64; HISTOGRAM_BUCKETS - 1] =
    [100, 500, 1_000, 5_000, 10_000, 25_000, 100_000];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputFailOpenReason {
    HeartbeatStale,
    RejectedSequence,
    MailboxFull,
    WakeFailure,
    Panic,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiMessagePhase {
    Tracker,
    Dispatch,
    WindowDrain,
    SettingsDrain,
    SwitcherDrain,
    MonitorDrain,
    Wake,
    MonitorSync,
    Frame,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackerUiPhase {
    PublishedSnapshotObservation,
    SwitcherReconciliation,
    DockModelRebuildForegroundUpdate,
    VisiblePickerReconciliation,
    PresentationStatusSynchronization,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheClass {
    NativeIcons,
    CustomImages,
    SvgRasters,
    D2dBrushes,
    DwriteTextFormats,
    EmbeddedBitmaps,
    D2dRasterBitmaps,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutOperation {
    DockHitTest,
    DockPopup,
    DockDrag,
    SettingsPointer,
    SettingsVisibleRows,
    LauncherHitTest,
    StatusHitTest,
    StatusPopup,
    MonitorHitTest,
    MonitorPopup,
}

impl LayoutOperation {
    const ALL: [Self; LAYOUT_OPERATIONS] = [
        Self::DockHitTest,
        Self::DockPopup,
        Self::DockDrag,
        Self::SettingsPointer,
        Self::SettingsVisibleRows,
        Self::LauncherHitTest,
        Self::StatusHitTest,
        Self::StatusPopup,
        Self::MonitorHitTest,
        Self::MonitorPopup,
    ];

    const fn index(self) -> usize {
        match self {
            Self::DockHitTest => 0,
            Self::DockPopup => 1,
            Self::DockDrag => 2,
            Self::SettingsPointer => 3,
            Self::SettingsVisibleRows => 4,
            Self::LauncherHitTest => 5,
            Self::StatusHitTest => 6,
            Self::StatusPopup => 7,
            Self::MonitorHitTest => 8,
            Self::MonitorPopup => 9,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::DockHitTest => "dock_hit_test",
            Self::DockPopup => "dock_popup",
            Self::DockDrag => "dock_drag",
            Self::SettingsPointer => "settings_pointer",
            Self::SettingsVisibleRows => "settings_visible_rows",
            Self::LauncherHitTest => "launcher_hit_test",
            Self::StatusHitTest => "status_hit_test",
            Self::StatusPopup => "status_popup",
            Self::MonitorHitTest => "monitor_hit_test",
            Self::MonitorPopup => "monitor_popup",
        }
    }
}

impl CacheClass {
    pub(crate) const ALL: [Self; CACHE_CLASSES] = [
        Self::NativeIcons,
        Self::CustomImages,
        Self::SvgRasters,
        Self::D2dBrushes,
        Self::DwriteTextFormats,
        Self::EmbeddedBitmaps,
        Self::D2dRasterBitmaps,
    ];

    const fn index(self) -> usize {
        match self {
            Self::NativeIcons => 0,
            Self::CustomImages => 1,
            Self::SvgRasters => 2,
            Self::D2dBrushes => 3,
            Self::DwriteTextFormats => 4,
            Self::EmbeddedBitmaps => 5,
            Self::D2dRasterBitmaps => 6,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::NativeIcons => "native_icons",
            Self::CustomImages => "custom_images",
            Self::SvgRasters => "svg_rasters",
            Self::D2dBrushes => "d2d_brushes",
            Self::DwriteTextFormats => "dwrite_text_formats",
            Self::EmbeddedBitmaps => "embedded_bitmaps",
            Self::D2dRasterBitmaps => "d2d_raster_bitmaps",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheSnapshot {
    pub class: CacheClass,
    pub current_entries: u64,
    pub current_bytes: u64,
    pub budget: u64,
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub replacements: u64,
    pub evictions: u64,
    pub clears: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LayoutSnapshot {
    operation: LayoutOperation,
    calls: u64,
    max_us: u64,
    histogram: [u64; HISTOGRAM_BUCKETS],
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrackerUiPhaseSnapshot {
    phase: TrackerUiPhase,
    calls: u64,
    max_us: u64,
    histogram: [u64; HISTOGRAM_BUCKETS],
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrackerMetricsSnapshot {
    ui_phases: [TrackerUiPhaseSnapshot; TRACKER_UI_PHASES],
    refresh_requests: u64,
    refresh_requests_coalesced: u64,
    worker_refresh_executions: u64,
    ui_wakes_posted: u64,
    ui_wakes_coalesced: u64,
    ui_wake_post_failures: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ApplicationMetricsSnapshot {
    catalog_generation: u64,
    catalog_entries: u64,
    catalog_duplicate_merges: u64,
    catalog_ambiguous_aliases: u64,
    catalog_build_max_us: u64,
    window_fact_hits: u64,
    window_fact_misses: u64,
    window_fact_max_us: u64,
    resolution_cache_hits: u64,
    resolution_cache_misses: u64,
    resolution_exact_registered: u64,
    resolution_exact_relaunch: u64,
    resolution_exact_provider: u64,
    resolution_exact_path: u64,
    resolution_unique_alias: u64,
    resolution_ambiguous: u64,
    resolution_unregistered: u64,
    resolution_prevented: u64,
    resolution_total_us: u64,
    resolution_max_us: u64,
    dock_projection_calls: u64,
    dock_projection_max_us: u64,
}

#[derive(Clone, Copy)]
pub struct FlyoutPhaseMetrics {
    pub worker_start: Duration,
    pub discovery_wait: Duration,
    pub bridge_configuration: Duration,
    pub positioning: Duration,
    pub total: Duration,
    pub timeout: bool,
    pub success: bool,
}

impl UiMessagePhase {
    pub const fn index(self) -> usize {
        match self {
            Self::Tracker => 0,
            Self::Dispatch => 1,
            Self::WindowDrain => 2,
            Self::SettingsDrain => 3,
            Self::SwitcherDrain => 4,
            Self::MonitorDrain => 5,
            Self::Wake => 6,
            Self::MonitorSync => 7,
            Self::Frame => 8,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Tracker => "tracker",
            Self::Dispatch => "dispatch",
            Self::WindowDrain => "window_drain",
            Self::SettingsDrain => "settings_drain",
            Self::SwitcherDrain => "switcher_drain",
            Self::MonitorDrain => "monitor_drain",
            Self::Wake => "wake",
            Self::MonitorSync => "monitor_sync",
            Self::Frame => "frame",
        }
    }
}

impl TrackerUiPhase {
    const ALL: [Self; TRACKER_UI_PHASES] = [
        Self::PublishedSnapshotObservation,
        Self::SwitcherReconciliation,
        Self::DockModelRebuildForegroundUpdate,
        Self::VisiblePickerReconciliation,
        Self::PresentationStatusSynchronization,
    ];

    const fn index(self) -> usize {
        match self {
            Self::PublishedSnapshotObservation => 0,
            Self::SwitcherReconciliation => 1,
            Self::DockModelRebuildForegroundUpdate => 2,
            Self::VisiblePickerReconciliation => 3,
            Self::PresentationStatusSynchronization => 4,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::PublishedSnapshotObservation => "published_snapshot_observation",
            Self::SwitcherReconciliation => "switcher_reconciliation",
            Self::DockModelRebuildForegroundUpdate => {
                "dock_model_rebuild_foreground_update"
            }
            Self::VisiblePickerReconciliation => "visible_picker_reconciliation",
            Self::PresentationStatusSynchronization => {
                "presentation_status_synchronization"
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlowUiEvent {
    pub timestamp_ms: u64,
    pub message_id: u32,
    pub category: &'static str,
    pub total_us: u64,
    pub accounted_us: u64,
    pub slowest_phase: &'static str,
    pub slowest_phase_us: u64,
    pub window_count: usize,
    pub monitor_replica_count: usize,
    pub dirty_surface_mask: u32,
    pub animating_surface_mask: u32,
    pub graphics_generation: u64,
    pub graphics_recovered: bool,
    pub visible_feature_mask: u32,
    pub input_fail_open: bool,
}

#[derive(Default)]
struct SlowUiEventRing {
    events: std::collections::VecDeque<SlowUiEvent>,
}

pub struct ResponsivenessMetrics {
    input_callbacks: AtomicU64,
    input_hook_lotus_max_us: AtomicU64,
    input_hook_lotus_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    input_hook_total_max_us: AtomicU64,
    input_hook_total_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    input_actions_enqueued: AtomicU64,
    input_actions_coalesced: AtomicU64,
    input_actions_dropped: AtomicU64,
    input_mailbox_high_water: AtomicU64,
    input_wakes_posted: AtomicU64,
    input_wakes_coalesced: AtomicU64,
    input_wake_failures: AtomicU64,
    input_fail_open_entries: AtomicU64,
    input_fail_open_heartbeat_stale: AtomicU64,
    input_fail_open_rejected_sequence: AtomicU64,
    input_fail_open_mailbox_full: AtomicU64,
    input_fail_open_wake_failure: AtomicU64,
    input_fail_open_panic: AtomicU64,
    input_fail_open_shutdown: AtomicU64,
    input_cleanup_requested: AtomicU64,
    input_cleanup_completed: AtomicU64,
    input_cleanup_redundant_suppressed: AtomicU64,
    input_cleanup_active_sequence_cancels: AtomicU64,
    input_cleanup_idle: AtomicU64,
    input_replay_failures: AtomicU64,
    input_sequence_cancels: AtomicU64,
    input_win_bare_sequences: AtomicU64,
    input_win_sequences_disqualified: AtomicU64,
    input_start_cancel_attempts: AtomicU64,
    input_start_cancel_successes: AtomicU64,
    input_start_cancel_failures: AtomicU64,
    input_delivery_max_us: AtomicU64,
    input_delivery_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    ui_messages: AtomicU64,
    ui_message_stalls: AtomicU64,
    ui_message_max_us: AtomicU64,
    ui_message_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    ui_phase_max_us: [AtomicU64; UI_PHASES],
    ui_phase_histogram: [AtomicU64; UI_PHASES * HISTOGRAM_BUCKETS],
    ui_messages_event_drain: AtomicU64,
    ui_messages_monitor_sync: AtomicU64,
    ui_messages_frame: AtomicU64,
    ui_message_severe: AtomicU64,
    ui_message_critical: AtomicU64,
    slow_ui_events: Mutex<SlowUiEventRing>,
    process_resources: Mutex<Option<ProcessResourceSample>>,
    tracker_ui_calls: [AtomicU64; TRACKER_UI_PHASES],
    tracker_ui_max_us: [AtomicU64; TRACKER_UI_PHASES],
    tracker_ui_histogram: [AtomicU64; TRACKER_UI_PHASES * HISTOGRAM_BUCKETS],
    layout_calls: [AtomicU64; LAYOUT_OPERATIONS],
    layout_max_us: [AtomicU64; LAYOUT_OPERATIONS],
    layout_histogram: [AtomicU64; LAYOUT_OPERATIONS * HISTOGRAM_BUCKETS],
    window_enumerations: AtomicU64,
    window_enumeration_max_us: AtomicU64,
    window_publishes: AtomicU64,
    window_unchanged: AtomicU64,
    process_metadata_hits: AtomicU64,
    process_metadata_misses: AtomicU64,
    tracker_refresh_requests: AtomicU64,
    tracker_refresh_requests_coalesced: AtomicU64,
    tracker_worker_refresh_executions: AtomicU64,
    tracker_ui_wakes_posted: AtomicU64,
    tracker_ui_wakes_coalesced: AtomicU64,
    tracker_ui_wake_post_failures: AtomicU64,
    application_catalog_generation: AtomicU64,
    application_catalog_entries: AtomicU64,
    application_catalog_duplicate_merges: AtomicU64,
    application_catalog_ambiguous_aliases: AtomicU64,
    application_catalog_build_max_us: AtomicU64,
    window_identity_fact_hits: AtomicU64,
    window_identity_fact_misses: AtomicU64,
    window_identity_fact_max_us: AtomicU64,
    application_resolution_cache_hits: AtomicU64,
    application_resolution_cache_misses: AtomicU64,
    application_resolution_exact_registered: AtomicU64,
    application_resolution_exact_relaunch: AtomicU64,
    application_resolution_exact_provider: AtomicU64,
    application_resolution_exact_path: AtomicU64,
    application_resolution_unique_alias: AtomicU64,
    application_resolution_ambiguous: AtomicU64,
    application_resolution_unregistered: AtomicU64,
    application_resolution_prevented: AtomicU64,
    application_resolution_total_us: AtomicU64,
    application_resolution_max_us: AtomicU64,
    dock_projection_calls: AtomicU64,
    dock_projection_max_us: AtomicU64,
    badge_events: AtomicU64,
    badge_scans: AtomicU64,
    badge_coalesced: AtomicU64,
    badge_scan_max_us: AtomicU64,
    badge_scan_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    badge_uia_enumeration_max_us: AtomicU64,
    badge_property_reads_max_us: AtomicU64,
    badge_snapshot_comparison_max_us: AtomicU64,
    badge_elements: AtomicU64,
    badge_supported_elements: AtomicU64,
    badge_provider_reads: AtomicU64,
    flyout_attempts: AtomicU64,
    flyout_max_us: AtomicU64,
    flyout_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    flyout_worker_start_max_us: AtomicU64,
    flyout_discovery_wait_max_us: AtomicU64,
    flyout_bridge_configuration_max_us: AtomicU64,
    flyout_positioning_max_us: AtomicU64,
    flyout_timeouts: AtomicU64,
    flyout_successes: AtomicU64,
    flyout_superseded: AtomicU64,
    switcher_requests: AtomicU64,
    switcher_results: AtomicU64,
    cache_entries: [AtomicU64; CACHE_CLASSES],
    cache_bytes: [AtomicU64; CACHE_CLASSES],
    cache_budgets: [AtomicU64; CACHE_CLASSES],
    cache_hits: [AtomicU64; CACHE_CLASSES],
    cache_misses: [AtomicU64; CACHE_CLASSES],
    cache_inserts: [AtomicU64; CACHE_CLASSES],
    cache_replacements: [AtomicU64; CACHE_CLASSES],
    cache_evictions: [AtomicU64; CACHE_CLASSES],
    cache_clears: [AtomicU64; CACHE_CLASSES],
}

pub static METRICS: ResponsivenessMetrics = ResponsivenessMetrics::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponsivenessSnapshot {
    pub input_callbacks: u64,
    pub input_hook_lotus_max_us: u64,
    pub input_hook_lotus_histogram: [u64; HISTOGRAM_BUCKETS],
    pub input_hook_total_max_us: u64,
    pub input_hook_total_histogram: [u64; HISTOGRAM_BUCKETS],
    pub input_actions_enqueued: u64,
    pub input_actions_coalesced: u64,
    pub input_actions_dropped: u64,
    pub input_mailbox_high_water: u64,
    pub input_wakes_posted: u64,
    pub input_wakes_coalesced: u64,
    pub input_wake_failures: u64,
    pub input_fail_open_entries: u64,
    pub input_fail_open_heartbeat_stale: u64,
    pub input_fail_open_rejected_sequence: u64,
    pub input_fail_open_mailbox_full: u64,
    pub input_fail_open_wake_failure: u64,
    pub input_fail_open_panic: u64,
    pub input_fail_open_shutdown: u64,
    pub input_cleanup_requested: u64,
    pub input_cleanup_completed: u64,
    pub input_cleanup_redundant_suppressed: u64,
    pub input_cleanup_active_sequence_cancels: u64,
    pub input_cleanup_idle: u64,
    pub input_replay_failures: u64,
    pub input_sequence_cancels: u64,
    pub input_win_bare_sequences: u64,
    pub input_win_sequences_disqualified: u64,
    pub input_start_cancel_attempts: u64,
    pub input_start_cancel_successes: u64,
    pub input_start_cancel_failures: u64,
    pub input_delivery_max_us: u64,
    pub input_delivery_histogram: [u64; HISTOGRAM_BUCKETS],
    pub ui_messages: u64,
    pub ui_message_stalls: u64,
    pub ui_message_max_us: u64,
    pub ui_message_histogram: [u64; HISTOGRAM_BUCKETS],
    pub ui_phase_max_us: [u64; UI_PHASES],
    pub ui_phase_histogram: [[u64; HISTOGRAM_BUCKETS]; UI_PHASES],
    pub ui_messages_event_drain: u64,
    pub ui_messages_monitor_sync: u64,
    pub ui_messages_frame: u64,
    pub ui_message_severe: u64,
    pub ui_message_critical: u64,
    pub slow_ui_events: Vec<SlowUiEvent>,
    layouts: [LayoutSnapshot; LAYOUT_OPERATIONS],
    pub window_enumerations: u64,
    pub window_enumeration_max_us: u64,
    pub window_publishes: u64,
    pub window_unchanged: u64,
    pub process_metadata_hits: u64,
    pub process_metadata_misses: u64,
    tracker: TrackerMetricsSnapshot,
    application: ApplicationMetricsSnapshot,
    process_resources: ProcessResourceSample,
    pub badge_events: u64,
    pub badge_scans: u64,
    pub badge_coalesced: u64,
    pub badge_scan_max_us: u64,
    pub badge_scan_histogram: [u64; HISTOGRAM_BUCKETS],
    pub badge_uia_enumeration_max_us: u64,
    pub badge_property_reads_max_us: u64,
    pub badge_snapshot_comparison_max_us: u64,
    pub badge_elements: u64,
    pub badge_supported_elements: u64,
    pub badge_provider_reads: u64,
    pub flyout_attempts: u64,
    pub flyout_max_us: u64,
    pub flyout_histogram: [u64; HISTOGRAM_BUCKETS],
    pub flyout_worker_start_max_us: u64,
    pub flyout_discovery_wait_max_us: u64,
    pub flyout_bridge_configuration_max_us: u64,
    pub flyout_positioning_max_us: u64,
    pub flyout_timeouts: u64,
    pub flyout_successes: u64,
    pub flyout_superseded: u64,
    pub switcher_requests: u64,
    pub switcher_results: u64,
    pub caches: [CacheSnapshot; CACHE_CLASSES],
}

impl ResponsivenessMetrics {
    #[allow(
        clippy::too_many_lines,
        reason = "the flat metrics constructor mirrors the atomic field inventory"
    )]
    const fn new() -> Self {
        Self {
            input_callbacks: AtomicU64::new(0),
            input_hook_lotus_max_us: AtomicU64::new(0),
            input_hook_lotus_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            input_hook_total_max_us: AtomicU64::new(0),
            input_hook_total_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            input_actions_enqueued: AtomicU64::new(0),
            input_actions_coalesced: AtomicU64::new(0),
            input_actions_dropped: AtomicU64::new(0),
            input_mailbox_high_water: AtomicU64::new(0),
            input_wakes_posted: AtomicU64::new(0),
            input_wakes_coalesced: AtomicU64::new(0),
            input_wake_failures: AtomicU64::new(0),
            input_fail_open_entries: AtomicU64::new(0),
            input_fail_open_heartbeat_stale: AtomicU64::new(0),
            input_fail_open_rejected_sequence: AtomicU64::new(0),
            input_fail_open_mailbox_full: AtomicU64::new(0),
            input_fail_open_wake_failure: AtomicU64::new(0),
            input_fail_open_panic: AtomicU64::new(0),
            input_fail_open_shutdown: AtomicU64::new(0),
            input_cleanup_requested: AtomicU64::new(0),
            input_cleanup_completed: AtomicU64::new(0),
            input_cleanup_redundant_suppressed: AtomicU64::new(0),
            input_cleanup_active_sequence_cancels: AtomicU64::new(0),
            input_cleanup_idle: AtomicU64::new(0),
            input_replay_failures: AtomicU64::new(0),
            input_sequence_cancels: AtomicU64::new(0),
            input_win_bare_sequences: AtomicU64::new(0),
            input_win_sequences_disqualified: AtomicU64::new(0),
            input_start_cancel_attempts: AtomicU64::new(0),
            input_start_cancel_successes: AtomicU64::new(0),
            input_start_cancel_failures: AtomicU64::new(0),
            input_delivery_max_us: AtomicU64::new(0),
            input_delivery_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            ui_messages: AtomicU64::new(0),
            ui_message_stalls: AtomicU64::new(0),
            ui_message_max_us: AtomicU64::new(0),
            ui_message_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            ui_phase_max_us: [const { AtomicU64::new(0) }; UI_PHASES],
            ui_phase_histogram: [const { AtomicU64::new(0) };
                UI_PHASES * HISTOGRAM_BUCKETS],
            ui_messages_event_drain: AtomicU64::new(0),
            ui_messages_monitor_sync: AtomicU64::new(0),
            ui_messages_frame: AtomicU64::new(0),
            ui_message_severe: AtomicU64::new(0),
            ui_message_critical: AtomicU64::new(0),
            slow_ui_events: Mutex::new(SlowUiEventRing {
                events: std::collections::VecDeque::new(),
            }),
            process_resources: Mutex::new(None),
            tracker_ui_calls: [const { AtomicU64::new(0) }; TRACKER_UI_PHASES],
            tracker_ui_max_us: [const { AtomicU64::new(0) }; TRACKER_UI_PHASES],
            tracker_ui_histogram: [const { AtomicU64::new(0) };
                TRACKER_UI_PHASES * HISTOGRAM_BUCKETS],
            layout_calls: [const { AtomicU64::new(0) }; LAYOUT_OPERATIONS],
            layout_max_us: [const { AtomicU64::new(0) }; LAYOUT_OPERATIONS],
            layout_histogram: [const { AtomicU64::new(0) };
                LAYOUT_OPERATIONS * HISTOGRAM_BUCKETS],
            window_enumerations: AtomicU64::new(0),
            window_enumeration_max_us: AtomicU64::new(0),
            window_publishes: AtomicU64::new(0),
            window_unchanged: AtomicU64::new(0),
            process_metadata_hits: AtomicU64::new(0),
            process_metadata_misses: AtomicU64::new(0),
            tracker_refresh_requests: AtomicU64::new(0),
            tracker_refresh_requests_coalesced: AtomicU64::new(0),
            tracker_worker_refresh_executions: AtomicU64::new(0),
            tracker_ui_wakes_posted: AtomicU64::new(0),
            tracker_ui_wakes_coalesced: AtomicU64::new(0),
            tracker_ui_wake_post_failures: AtomicU64::new(0),
            application_catalog_generation: AtomicU64::new(0),
            application_catalog_entries: AtomicU64::new(0),
            application_catalog_duplicate_merges: AtomicU64::new(0),
            application_catalog_ambiguous_aliases: AtomicU64::new(0),
            application_catalog_build_max_us: AtomicU64::new(0),
            window_identity_fact_hits: AtomicU64::new(0),
            window_identity_fact_misses: AtomicU64::new(0),
            window_identity_fact_max_us: AtomicU64::new(0),
            application_resolution_cache_hits: AtomicU64::new(0),
            application_resolution_cache_misses: AtomicU64::new(0),
            application_resolution_exact_registered: AtomicU64::new(0),
            application_resolution_exact_relaunch: AtomicU64::new(0),
            application_resolution_exact_provider: AtomicU64::new(0),
            application_resolution_exact_path: AtomicU64::new(0),
            application_resolution_unique_alias: AtomicU64::new(0),
            application_resolution_ambiguous: AtomicU64::new(0),
            application_resolution_unregistered: AtomicU64::new(0),
            application_resolution_prevented: AtomicU64::new(0),
            application_resolution_total_us: AtomicU64::new(0),
            application_resolution_max_us: AtomicU64::new(0),
            dock_projection_calls: AtomicU64::new(0),
            dock_projection_max_us: AtomicU64::new(0),
            badge_events: AtomicU64::new(0),
            badge_scans: AtomicU64::new(0),
            badge_coalesced: AtomicU64::new(0),
            badge_scan_max_us: AtomicU64::new(0),
            badge_scan_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            badge_uia_enumeration_max_us: AtomicU64::new(0),
            badge_property_reads_max_us: AtomicU64::new(0),
            badge_snapshot_comparison_max_us: AtomicU64::new(0),
            badge_elements: AtomicU64::new(0),
            badge_supported_elements: AtomicU64::new(0),
            badge_provider_reads: AtomicU64::new(0),
            flyout_attempts: AtomicU64::new(0),
            flyout_max_us: AtomicU64::new(0),
            flyout_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            flyout_worker_start_max_us: AtomicU64::new(0),
            flyout_discovery_wait_max_us: AtomicU64::new(0),
            flyout_bridge_configuration_max_us: AtomicU64::new(0),
            flyout_positioning_max_us: AtomicU64::new(0),
            flyout_timeouts: AtomicU64::new(0),
            flyout_successes: AtomicU64::new(0),
            flyout_superseded: AtomicU64::new(0),
            switcher_requests: AtomicU64::new(0),
            switcher_results: AtomicU64::new(0),
            cache_entries: [const { AtomicU64::new(0) }; CACHE_CLASSES],
            cache_bytes: [const { AtomicU64::new(0) }; CACHE_CLASSES],
            cache_budgets: [const { AtomicU64::new(0) }; CACHE_CLASSES],
            cache_hits: [const { AtomicU64::new(0) }; CACHE_CLASSES],
            cache_misses: [const { AtomicU64::new(0) }; CACHE_CLASSES],
            cache_inserts: [const { AtomicU64::new(0) }; CACHE_CLASSES],
            cache_replacements: [const { AtomicU64::new(0) }; CACHE_CLASSES],
            cache_evictions: [const { AtomicU64::new(0) }; CACHE_CLASSES],
            cache_clears: [const { AtomicU64::new(0) }; CACHE_CLASSES],
        }
    }
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros().min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
}

fn record_duration(duration_us: u64, maximum: &AtomicU64, histogram: &[AtomicU64]) {
    maximum.fetch_max(duration_us, Ordering::Relaxed);
    let bucket = histogram_bucket(duration_us);
    saturating_add(&histogram[bucket], 1);
}

fn histogram_bucket(duration_us: u64) -> usize {
    HISTOGRAM_LIMITS_US
        .iter()
        .position(|limit| duration_us <= *limit)
        .unwrap_or(HISTOGRAM_BUCKETS - 1)
}

fn load_histogram(source: &[AtomicU64; HISTOGRAM_BUCKETS]) -> [u64; HISTOGRAM_BUCKETS] {
    std::array::from_fn(|index| source[index].load(Ordering::Relaxed))
}

fn saturating_add(target: &AtomicU64, amount: u64) {
    let _ = target.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(amount))
    });
}

fn saturating_sub(target: &AtomicU64, amount: u64) {
    let _ = target.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_sub(amount))
    });
}

fn usize_as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
