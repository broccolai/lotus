use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use lotus_core::application::{ApplicationResolution, ResolutionEvidence};

use crate::process_resources::ProcessResourceSample;

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

    #[allow(
        clippy::too_many_lines,
        reason = "the flat snapshot intentionally mirrors the atomic metric inventory"
    )]
    pub fn snapshot(&self) -> ResponsivenessSnapshot {
        ResponsivenessSnapshot {
            input_callbacks: self.input_callbacks.load(Ordering::Relaxed),
            input_hook_lotus_max_us: self.input_hook_lotus_max_us.load(Ordering::Relaxed),
            input_hook_lotus_histogram: load_histogram(&self.input_hook_lotus_histogram),
            input_hook_total_max_us: self.input_hook_total_max_us.load(Ordering::Relaxed),
            input_hook_total_histogram: load_histogram(&self.input_hook_total_histogram),
            input_actions_enqueued: self.input_actions_enqueued.load(Ordering::Relaxed),
            input_actions_coalesced: self.input_actions_coalesced.load(Ordering::Relaxed),
            input_actions_dropped: self.input_actions_dropped.load(Ordering::Relaxed),
            input_mailbox_high_water: self.input_mailbox_high_water.load(Ordering::Relaxed),
            input_wakes_posted: self.input_wakes_posted.load(Ordering::Relaxed),
            input_wakes_coalesced: self.input_wakes_coalesced.load(Ordering::Relaxed),
            input_wake_failures: self.input_wake_failures.load(Ordering::Relaxed),
            input_fail_open_entries: self.input_fail_open_entries.load(Ordering::Relaxed),
            input_fail_open_heartbeat_stale: self
                .input_fail_open_heartbeat_stale
                .load(Ordering::Relaxed),
            input_fail_open_rejected_sequence: self
                .input_fail_open_rejected_sequence
                .load(Ordering::Relaxed),
            input_fail_open_mailbox_full: self
                .input_fail_open_mailbox_full
                .load(Ordering::Relaxed),
            input_fail_open_wake_failure: self
                .input_fail_open_wake_failure
                .load(Ordering::Relaxed),
            input_fail_open_panic: self.input_fail_open_panic.load(Ordering::Relaxed),
            input_fail_open_shutdown: self.input_fail_open_shutdown.load(Ordering::Relaxed),
            input_cleanup_requested: self.input_cleanup_requested.load(Ordering::Relaxed),
            input_cleanup_completed: self.input_cleanup_completed.load(Ordering::Relaxed),
            input_cleanup_redundant_suppressed: self
                .input_cleanup_redundant_suppressed
                .load(Ordering::Relaxed),
            input_cleanup_active_sequence_cancels: self
                .input_cleanup_active_sequence_cancels
                .load(Ordering::Relaxed),
            input_cleanup_idle: self.input_cleanup_idle.load(Ordering::Relaxed),
            input_replay_failures: self.input_replay_failures.load(Ordering::Relaxed),
            input_sequence_cancels: self.input_sequence_cancels.load(Ordering::Relaxed),
            input_win_bare_sequences: self.input_win_bare_sequences.load(Ordering::Relaxed),
            input_win_sequences_disqualified: self
                .input_win_sequences_disqualified
                .load(Ordering::Relaxed),
            input_start_cancel_attempts: self
                .input_start_cancel_attempts
                .load(Ordering::Relaxed),
            input_start_cancel_successes: self
                .input_start_cancel_successes
                .load(Ordering::Relaxed),
            input_start_cancel_failures: self
                .input_start_cancel_failures
                .load(Ordering::Relaxed),
            input_delivery_max_us: self.input_delivery_max_us.load(Ordering::Relaxed),
            input_delivery_histogram: load_histogram(&self.input_delivery_histogram),
            ui_messages: self.ui_messages.load(Ordering::Relaxed),
            ui_message_stalls: self.ui_message_stalls.load(Ordering::Relaxed),
            ui_message_max_us: self.ui_message_max_us.load(Ordering::Relaxed),
            ui_message_histogram: load_histogram(&self.ui_message_histogram),
            ui_phase_max_us: std::array::from_fn(|index| {
                self.ui_phase_max_us[index].load(Ordering::Relaxed)
            }),
            ui_phase_histogram: std::array::from_fn(|phase| {
                self.load_ui_phase_histogram(phase)
            }),
            ui_messages_event_drain: self.ui_messages_event_drain.load(Ordering::Relaxed),
            ui_messages_monitor_sync: self.ui_messages_monitor_sync.load(Ordering::Relaxed),
            ui_messages_frame: self.ui_messages_frame.load(Ordering::Relaxed),
            ui_message_severe: self.ui_message_severe.load(Ordering::Relaxed),
            ui_message_critical: self.ui_message_critical.load(Ordering::Relaxed),
            slow_ui_events: self.slow_ui_events_snapshot(),
            layouts: self.layout_snapshots(),
            window_enumerations: self.window_enumerations.load(Ordering::Relaxed),
            window_enumeration_max_us: self
                .window_enumeration_max_us
                .load(Ordering::Relaxed),
            window_publishes: self.window_publishes.load(Ordering::Relaxed),
            window_unchanged: self.window_unchanged.load(Ordering::Relaxed),
            process_metadata_hits: self.process_metadata_hits.load(Ordering::Relaxed),
            process_metadata_misses: self.process_metadata_misses.load(Ordering::Relaxed),
            tracker: self.tracker_metrics_snapshot(),
            application: self.application_metrics_snapshot(),
            process_resources: self.process_resource_snapshot(),
            badge_events: self.badge_events.load(Ordering::Relaxed),
            badge_scans: self.badge_scans.load(Ordering::Relaxed),
            badge_coalesced: self.badge_coalesced.load(Ordering::Relaxed),
            badge_scan_max_us: self.badge_scan_max_us.load(Ordering::Relaxed),
            badge_scan_histogram: load_histogram(&self.badge_scan_histogram),
            badge_uia_enumeration_max_us: self
                .badge_uia_enumeration_max_us
                .load(Ordering::Relaxed),
            badge_property_reads_max_us: self
                .badge_property_reads_max_us
                .load(Ordering::Relaxed),
            badge_snapshot_comparison_max_us: self
                .badge_snapshot_comparison_max_us
                .load(Ordering::Relaxed),
            badge_elements: self.badge_elements.load(Ordering::Relaxed),
            badge_supported_elements: self.badge_supported_elements.load(Ordering::Relaxed),
            badge_provider_reads: self.badge_provider_reads.load(Ordering::Relaxed),
            flyout_attempts: self.flyout_attempts.load(Ordering::Relaxed),
            flyout_max_us: self.flyout_max_us.load(Ordering::Relaxed),
            flyout_histogram: load_histogram(&self.flyout_histogram),
            flyout_worker_start_max_us: self
                .flyout_worker_start_max_us
                .load(Ordering::Relaxed),
            flyout_discovery_wait_max_us: self
                .flyout_discovery_wait_max_us
                .load(Ordering::Relaxed),
            flyout_bridge_configuration_max_us: self
                .flyout_bridge_configuration_max_us
                .load(Ordering::Relaxed),
            flyout_positioning_max_us: self
                .flyout_positioning_max_us
                .load(Ordering::Relaxed),
            flyout_timeouts: self.flyout_timeouts.load(Ordering::Relaxed),
            flyout_successes: self.flyout_successes.load(Ordering::Relaxed),
            flyout_superseded: self.flyout_superseded.load(Ordering::Relaxed),
            switcher_requests: self.switcher_requests.load(Ordering::Relaxed),
            switcher_results: self.switcher_results.load(Ordering::Relaxed),
            caches: std::array::from_fn(|index| self.cache_snapshot(index)),
        }
    }

    pub fn record_input_callback(&self) {
        saturating_add(&self.input_callbacks, 1);
    }

    fn layout_snapshots(&self) -> [LayoutSnapshot; LAYOUT_OPERATIONS] {
        std::array::from_fn(|index| LayoutSnapshot {
            operation: LayoutOperation::ALL[index],
            calls: self.layout_calls[index].load(Ordering::Relaxed),
            max_us: self.layout_max_us[index].load(Ordering::Relaxed),
            histogram: self.load_layout_histogram(index),
        })
    }

    fn tracker_ui_phase_snapshots(&self) -> [TrackerUiPhaseSnapshot; TRACKER_UI_PHASES] {
        std::array::from_fn(|index| TrackerUiPhaseSnapshot {
            phase: TrackerUiPhase::ALL[index],
            calls: self.tracker_ui_calls[index].load(Ordering::Relaxed),
            max_us: self.tracker_ui_max_us[index].load(Ordering::Relaxed),
            histogram: self.load_tracker_ui_histogram(index),
        })
    }

    fn tracker_metrics_snapshot(&self) -> TrackerMetricsSnapshot {
        TrackerMetricsSnapshot {
            ui_phases: self.tracker_ui_phase_snapshots(),
            refresh_requests: self.tracker_refresh_requests.load(Ordering::Relaxed),
            refresh_requests_coalesced: self
                .tracker_refresh_requests_coalesced
                .load(Ordering::Relaxed),
            worker_refresh_executions: self
                .tracker_worker_refresh_executions
                .load(Ordering::Relaxed),
            ui_wakes_posted: self.tracker_ui_wakes_posted.load(Ordering::Relaxed),
            ui_wakes_coalesced: self.tracker_ui_wakes_coalesced.load(Ordering::Relaxed),
            ui_wake_post_failures: self
                .tracker_ui_wake_post_failures
                .load(Ordering::Relaxed),
        }
    }

    fn application_metrics_snapshot(&self) -> ApplicationMetricsSnapshot {
        ApplicationMetricsSnapshot {
            catalog_generation: self.application_catalog_generation.load(Ordering::Relaxed),
            catalog_entries: self.application_catalog_entries.load(Ordering::Relaxed),
            catalog_duplicate_merges: self
                .application_catalog_duplicate_merges
                .load(Ordering::Relaxed),
            catalog_ambiguous_aliases: self
                .application_catalog_ambiguous_aliases
                .load(Ordering::Relaxed),
            catalog_build_max_us: self
                .application_catalog_build_max_us
                .load(Ordering::Relaxed),
            window_fact_hits: self.window_identity_fact_hits.load(Ordering::Relaxed),
            window_fact_misses: self.window_identity_fact_misses.load(Ordering::Relaxed),
            window_fact_max_us: self.window_identity_fact_max_us.load(Ordering::Relaxed),
            resolution_cache_hits: self
                .application_resolution_cache_hits
                .load(Ordering::Relaxed),
            resolution_cache_misses: self
                .application_resolution_cache_misses
                .load(Ordering::Relaxed),
            resolution_exact_registered: self
                .application_resolution_exact_registered
                .load(Ordering::Relaxed),
            resolution_exact_relaunch: self
                .application_resolution_exact_relaunch
                .load(Ordering::Relaxed),
            resolution_exact_provider: self
                .application_resolution_exact_provider
                .load(Ordering::Relaxed),
            resolution_exact_path: self
                .application_resolution_exact_path
                .load(Ordering::Relaxed),
            resolution_unique_alias: self
                .application_resolution_unique_alias
                .load(Ordering::Relaxed),
            resolution_ambiguous: self
                .application_resolution_ambiguous
                .load(Ordering::Relaxed),
            resolution_unregistered: self
                .application_resolution_unregistered
                .load(Ordering::Relaxed),
            resolution_prevented: self
                .application_resolution_prevented
                .load(Ordering::Relaxed),
            resolution_total_us: self
                .application_resolution_total_us
                .load(Ordering::Relaxed),
            resolution_max_us: self.application_resolution_max_us.load(Ordering::Relaxed),
            dock_projection_calls: self.dock_projection_calls.load(Ordering::Relaxed),
            dock_projection_max_us: self.dock_projection_max_us.load(Ordering::Relaxed),
        }
    }

    fn slow_ui_events_snapshot(&self) -> Vec<SlowUiEvent> {
        self.slow_ui_events
            .lock()
            .map_or_else(|_| Vec::new(), |ring| ring.events.iter().cloned().collect())
    }

    fn process_resource_snapshot(&self) -> ProcessResourceSample {
        self.process_resources.lock().map_or_else(
            |_| crate::process_resources::current_process_resources(),
            |sample| {
                sample.unwrap_or_else(crate::process_resources::current_process_resources)
            },
        )
    }

    fn cache_snapshot(&self, index: usize) -> CacheSnapshot {
        CacheSnapshot {
            class: CacheClass::ALL[index],
            current_entries: self.cache_entries[index].load(Ordering::Relaxed),
            current_bytes: self.cache_bytes[index].load(Ordering::Relaxed),
            budget: self.cache_budgets[index].load(Ordering::Relaxed),
            hits: self.cache_hits[index].load(Ordering::Relaxed),
            misses: self.cache_misses[index].load(Ordering::Relaxed),
            inserts: self.cache_inserts[index].load(Ordering::Relaxed),
            replacements: self.cache_replacements[index].load(Ordering::Relaxed),
            evictions: self.cache_evictions[index].load(Ordering::Relaxed),
            clears: self.cache_clears[index].load(Ordering::Relaxed),
        }
    }

    pub fn record_input_hook_lotus(&self, duration: Duration) {
        record_duration(
            duration_micros(duration),
            &self.input_hook_lotus_max_us,
            &self.input_hook_lotus_histogram,
        );
    }

    pub fn record_input_hook_total(&self, duration: Duration) {
        record_duration(
            duration_micros(duration),
            &self.input_hook_total_max_us,
            &self.input_hook_total_histogram,
        );
    }

    pub fn record_input_action_enqueued(&self) {
        saturating_add(&self.input_actions_enqueued, 1);
    }

    pub fn record_input_action_coalesced(&self) {
        saturating_add(&self.input_actions_coalesced, 1);
    }

    pub fn record_input_action_dropped(&self) {
        saturating_add(&self.input_actions_dropped, 1);
    }

    pub fn record_input_mailbox_depth(&self, depth: u32) {
        self.input_mailbox_high_water
            .fetch_max(u64::from(depth), Ordering::Relaxed);
    }

    pub fn record_input_wake_posted(&self) {
        saturating_add(&self.input_wakes_posted, 1);
    }

    pub fn record_input_wake_coalesced(&self) {
        saturating_add(&self.input_wakes_coalesced, 1);
    }

    pub fn record_input_wake_failure(&self) {
        saturating_add(&self.input_wake_failures, 1);
    }

    pub fn record_input_fail_open(&self, reason: InputFailOpenReason) {
        saturating_add(&self.input_fail_open_entries, 1);
        let counter = match reason {
            InputFailOpenReason::HeartbeatStale => &self.input_fail_open_heartbeat_stale,
            InputFailOpenReason::RejectedSequence => {
                &self.input_fail_open_rejected_sequence
            }
            InputFailOpenReason::MailboxFull => &self.input_fail_open_mailbox_full,
            InputFailOpenReason::WakeFailure => &self.input_fail_open_wake_failure,
            InputFailOpenReason::Panic => &self.input_fail_open_panic,
            InputFailOpenReason::Shutdown => &self.input_fail_open_shutdown,
        };
        saturating_add(counter, 1);
    }

    pub fn record_input_cleanup_requested(&self) {
        saturating_add(&self.input_cleanup_requested, 1);
    }

    pub fn record_input_cleanup_completed(&self) {
        saturating_add(&self.input_cleanup_completed, 1);
    }

    pub fn record_input_cleanup_redundant_suppressed(&self) {
        saturating_add(&self.input_cleanup_redundant_suppressed, 1);
    }

    pub fn record_input_cleanup_active_sequence_cancel(&self) {
        saturating_add(&self.input_cleanup_active_sequence_cancels, 1);
    }

    pub fn record_input_cleanup_idle(&self) {
        saturating_add(&self.input_cleanup_idle, 1);
    }

    pub fn record_input_replay_failure(&self) {
        saturating_add(&self.input_replay_failures, 1);
    }

    pub fn record_input_sequence_cancel(&self) {
        saturating_add(&self.input_sequence_cancels, 1);
    }

    pub fn record_input_win_bare_sequence(&self) {
        saturating_add(&self.input_win_bare_sequences, 1);
    }

    pub fn record_input_win_sequence_disqualified(&self) {
        saturating_add(&self.input_win_sequences_disqualified, 1);
    }

    pub fn record_input_start_cancel_attempt(&self) {
        saturating_add(&self.input_start_cancel_attempts, 1);
    }

    pub fn record_input_start_cancel_success(&self) {
        saturating_add(&self.input_start_cancel_successes, 1);
    }

    pub fn record_input_start_cancel_failure(&self) {
        saturating_add(&self.input_start_cancel_failures, 1);
    }

    pub fn record_input_delivery(&self, duration: Duration) {
        record_duration(
            duration_micros(duration),
            &self.input_delivery_max_us,
            &self.input_delivery_histogram,
        );
    }

    pub fn record_ui_message(&self, duration: Duration) {
        let micros = duration_micros(duration);
        saturating_add(&self.ui_messages, 1);
        if micros >= 16_000 {
            saturating_add(&self.ui_message_stalls, 1);
        }
        record_duration(micros, &self.ui_message_max_us, &self.ui_message_histogram);
    }

    pub fn record_ui_phase(&self, phase: UiMessagePhase, duration: Duration) -> u64 {
        let micros = duration_micros(duration);
        let index = phase.index();
        record_duration(
            micros,
            &self.ui_phase_max_us[index],
            self.ui_phase_histogram_at(index),
        );
        micros
    }

    pub fn record_ui_work(&self, event_drain: bool, monitor_sync: bool, frame: bool) {
        if event_drain {
            saturating_add(&self.ui_messages_event_drain, 1);
        }
        if monitor_sync {
            saturating_add(&self.ui_messages_monitor_sync, 1);
        }
        if frame {
            saturating_add(&self.ui_messages_frame, 1);
        }
    }

    pub fn record_tracker_ui_phase(&self, phase: TrackerUiPhase, duration: Duration) {
        let index = phase.index();
        saturating_add(&self.tracker_ui_calls[index], 1);
        record_duration(
            duration_micros(duration),
            &self.tracker_ui_max_us[index],
            self.tracker_ui_histogram_at(index),
        );
    }

    pub fn record_application_catalog(
        &self,
        generation: u64,
        entries: usize,
        duplicate_merges: usize,
        ambiguous_aliases: usize,
        duration: Duration,
    ) {
        self.application_catalog_generation
            .store(generation, Ordering::Relaxed);
        self.application_catalog_entries
            .store(usize_as_u64(entries), Ordering::Relaxed);
        self.application_catalog_duplicate_merges
            .store(usize_as_u64(duplicate_merges), Ordering::Relaxed);
        self.application_catalog_ambiguous_aliases
            .store(usize_as_u64(ambiguous_aliases), Ordering::Relaxed);
        self.application_catalog_build_max_us
            .fetch_max(duration_micros(duration), Ordering::Relaxed);
    }

    pub fn record_window_identity_fact(&self, cached: bool, duration: Duration) {
        let counter = if cached {
            &self.window_identity_fact_hits
        } else {
            &self.window_identity_fact_misses
        };
        saturating_add(counter, 1);
        if !cached {
            self.window_identity_fact_max_us
                .fetch_max(duration_micros(duration), Ordering::Relaxed);
        }
    }

    pub fn record_application_resolution(
        &self,
        cached: bool,
        resolution: &ApplicationResolution,
    ) {
        let cache_counter = if cached {
            &self.application_resolution_cache_hits
        } else {
            &self.application_resolution_cache_misses
        };
        saturating_add(cache_counter, 1);
        match resolution {
            ApplicationResolution::Resolved { evidence, .. } => {
                let counter = match evidence {
                    ResolutionEvidence::ExactRegisteredId
                    | ResolutionEvidence::ExplicitAssociation => {
                        &self.application_resolution_exact_registered
                    }
                    ResolutionEvidence::ExactRelaunch => {
                        &self.application_resolution_exact_relaunch
                    }
                    ResolutionEvidence::ExactProviderKey => {
                        &self.application_resolution_exact_provider
                    }
                    ResolutionEvidence::ExactExecutablePath => {
                        &self.application_resolution_exact_path
                    }
                    ResolutionEvidence::UniqueExecutableAlias => {
                        &self.application_resolution_unique_alias
                    }
                    ResolutionEvidence::NoMatch => {
                        &self.application_resolution_unregistered
                    }
                };
                saturating_add(counter, 1);
            }
            ApplicationResolution::Ambiguous { .. } => {
                saturating_add(&self.application_resolution_ambiguous, 1);
            }
            ApplicationResolution::Associated { .. } => {
                saturating_add(&self.application_resolution_exact_registered, 1);
            }
            ApplicationResolution::Unregistered { .. } => {
                saturating_add(&self.application_resolution_unregistered, 1);
            }
            ApplicationResolution::Prevented => {
                saturating_add(&self.application_resolution_prevented, 1);
            }
        }
    }

    pub fn record_application_resolution_batch(&self, duration: Duration) {
        let micros = duration_micros(duration);
        saturating_add(&self.application_resolution_total_us, micros);
        self.application_resolution_max_us
            .fetch_max(micros, Ordering::Relaxed);
    }

    pub fn record_dock_projection(&self, duration: Duration) {
        saturating_add(&self.dock_projection_calls, 1);
        self.dock_projection_max_us
            .fetch_max(duration_micros(duration), Ordering::Relaxed);
    }

    pub fn capture_process_resources(&self) {
        let sample = crate::process_resources::current_process_resources();
        if let Ok(mut current) = self.process_resources.lock() {
            *current = Some(sample);
        }
    }

    pub fn record_layout(&self, operation: LayoutOperation, duration: Duration) {
        let index = operation.index();
        let micros = duration_micros(duration);
        saturating_add(&self.layout_calls[index], 1);
        self.layout_max_us[index].fetch_max(micros, Ordering::Relaxed);
        let bucket = histogram_bucket(micros);
        saturating_add(
            &self.layout_histogram[index * HISTOGRAM_BUCKETS + bucket],
            1,
        );
    }

    fn load_layout_histogram(&self, operation: usize) -> [u64; HISTOGRAM_BUCKETS] {
        std::array::from_fn(|bucket| {
            self.layout_histogram[operation * HISTOGRAM_BUCKETS + bucket]
                .load(Ordering::Relaxed)
        })
    }

    fn tracker_ui_histogram_at(&self, phase: usize) -> &[AtomicU64] {
        let start = phase * HISTOGRAM_BUCKETS;
        &self.tracker_ui_histogram[start..start + HISTOGRAM_BUCKETS]
    }

    fn load_tracker_ui_histogram(&self, phase: usize) -> [u64; HISTOGRAM_BUCKETS] {
        let source = self.tracker_ui_histogram_at(phase);
        std::array::from_fn(|index| source[index].load(Ordering::Relaxed))
    }

    fn ui_phase_histogram_at(&self, phase: usize) -> &[AtomicU64] {
        let start = phase * HISTOGRAM_BUCKETS;
        &self.ui_phase_histogram[start..start + HISTOGRAM_BUCKETS]
    }

    fn load_ui_phase_histogram(&self, phase: usize) -> [u64; HISTOGRAM_BUCKETS] {
        let source = self.ui_phase_histogram_at(phase);
        std::array::from_fn(|index| source[index].load(Ordering::Relaxed))
    }

    pub fn record_slow_ui_event(&self, event: SlowUiEvent) {
        if event.total_us < 50_000 {
            return;
        }
        if event.total_us >= 250_000 {
            saturating_add(&self.ui_message_severe, 1);
        }
        if event.total_us >= 1_000_000 {
            saturating_add(&self.ui_message_critical, 1);
        }
        if let Ok(mut ring) = self.slow_ui_events.lock() {
            if ring.events.len() == SLOW_UI_EVENTS {
                let _ = ring.events.pop_front();
            }
            ring.events.push_back(event);
        }
    }

    pub fn record_window_enumeration(&self, duration: Duration) {
        saturating_add(&self.window_enumerations, 1);
        self.window_enumeration_max_us
            .fetch_max(duration_micros(duration), Ordering::Relaxed);
    }

    pub fn record_window_publish(&self) {
        saturating_add(&self.window_publishes, 1);
    }

    pub fn record_window_unchanged(&self) {
        saturating_add(&self.window_unchanged, 1);
    }

    pub fn record_tracker_refresh_request(&self, coalesced: bool) {
        saturating_add(&self.tracker_refresh_requests, 1);
        if coalesced {
            saturating_add(&self.tracker_refresh_requests_coalesced, 1);
        }
    }

    pub fn record_tracker_worker_refresh_execution(&self) {
        saturating_add(&self.tracker_worker_refresh_executions, 1);
    }

    pub fn record_tracker_ui_wake_posted(&self) {
        saturating_add(&self.tracker_ui_wakes_posted, 1);
    }

    pub fn record_tracker_ui_wake_coalesced(&self) {
        saturating_add(&self.tracker_ui_wakes_coalesced, 1);
    }

    pub fn record_tracker_ui_wake_post_failure(&self) {
        saturating_add(&self.tracker_ui_wake_post_failures, 1);
    }

    pub fn record_process_metadata(&self, cached: bool) {
        let counter = if cached {
            &self.process_metadata_hits
        } else {
            &self.process_metadata_misses
        };
        saturating_add(counter, 1);
    }

    pub fn record_badge_event(&self) {
        saturating_add(&self.badge_events, 1);
    }

    pub fn record_badge_coalesced(&self) {
        saturating_add(&self.badge_coalesced, 1);
    }

    pub fn record_badge_scan(&self, duration: Duration) {
        saturating_add(&self.badge_scans, 1);
        record_duration(
            duration_micros(duration),
            &self.badge_scan_max_us,
            &self.badge_scan_histogram,
        );
    }

    pub fn record_badge_phases(
        &self,
        enumeration: Duration,
        property_reads: Duration,
        comparison: Duration,
        elements: usize,
        supported_elements: usize,
        provider_reads: usize,
    ) {
        self.badge_uia_enumeration_max_us
            .fetch_max(duration_micros(enumeration), Ordering::Relaxed);
        self.badge_property_reads_max_us
            .fetch_max(duration_micros(property_reads), Ordering::Relaxed);
        self.badge_snapshot_comparison_max_us
            .fetch_max(duration_micros(comparison), Ordering::Relaxed);
        saturating_add(&self.badge_elements, elements as u64);
        saturating_add(&self.badge_supported_elements, supported_elements as u64);
        saturating_add(&self.badge_provider_reads, provider_reads as u64);
    }

    pub fn record_flyout(&self, duration: Duration) {
        saturating_add(&self.flyout_attempts, 1);
        record_duration(
            duration_micros(duration),
            &self.flyout_max_us,
            &self.flyout_histogram,
        );
    }

    pub fn record_flyout_phases(&self, phases: FlyoutPhaseMetrics) {
        self.flyout_worker_start_max_us
            .fetch_max(duration_micros(phases.worker_start), Ordering::Relaxed);
        self.flyout_discovery_wait_max_us
            .fetch_max(duration_micros(phases.discovery_wait), Ordering::Relaxed);
        self.flyout_bridge_configuration_max_us.fetch_max(
            duration_micros(phases.bridge_configuration),
            Ordering::Relaxed,
        );
        self.flyout_positioning_max_us
            .fetch_max(duration_micros(phases.positioning), Ordering::Relaxed);
        self.record_flyout(phases.total);
        if phases.timeout {
            saturating_add(&self.flyout_timeouts, 1);
        }
        if phases.success {
            saturating_add(&self.flyout_successes, 1);
        }
    }

    pub fn record_flyout_superseded(&self) {
        saturating_add(&self.flyout_superseded, 1);
    }

    pub fn record_switcher_requests(&self, count: usize) {
        saturating_add(&self.switcher_requests, count as u64);
    }

    pub fn record_switcher_results(&self, count: usize) {
        saturating_add(&self.switcher_results, count as u64);
    }

    pub(crate) fn register_cache(&self, class: CacheClass, budget: usize) {
        saturating_add(&self.cache_budgets[class.index()], usize_as_u64(budget));
    }

    pub(crate) fn unregister_cache(&self, class: CacheClass, budget: usize) {
        saturating_sub(&self.cache_budgets[class.index()], usize_as_u64(budget));
    }

    pub(crate) fn record_cache_hit(&self, class: CacheClass) {
        saturating_add(&self.cache_hits[class.index()], 1);
    }

    pub(crate) fn record_cache_miss(&self, class: CacheClass) {
        saturating_add(&self.cache_misses[class.index()], 1);
    }

    pub(crate) fn record_cache_insert(&self, class: CacheClass, bytes: usize) {
        saturating_add(&self.cache_entries[class.index()], 1);
        saturating_add(&self.cache_bytes[class.index()], usize_as_u64(bytes));
        saturating_add(&self.cache_inserts[class.index()], 1);
    }

    pub(crate) fn record_cache_replacement(
        &self,
        class: CacheClass,
        removed: usize,
        added: usize,
    ) {
        saturating_sub(&self.cache_bytes[class.index()], usize_as_u64(removed));
        saturating_add(&self.cache_bytes[class.index()], usize_as_u64(added));
        saturating_add(&self.cache_replacements[class.index()], 1);
    }

    pub(crate) fn record_cache_remove(
        &self,
        class: CacheClass,
        entries: usize,
        bytes: usize,
    ) {
        saturating_sub(&self.cache_entries[class.index()], usize_as_u64(entries));
        saturating_sub(&self.cache_bytes[class.index()], usize_as_u64(bytes));
    }

    pub(crate) fn record_cache_eviction(&self, class: CacheClass, bytes: usize) {
        self.record_cache_remove(class, 1, bytes);
        saturating_add(&self.cache_evictions[class.index()], 1);
    }

    pub(crate) fn record_cache_clear(
        &self,
        class: CacheClass,
        entries: usize,
        bytes: usize,
    ) {
        self.record_cache_remove(class, entries, bytes);
        saturating_add(&self.cache_clears[class.index()], 1);
    }
}

impl ResponsivenessSnapshot {
    pub fn to_text(&self) -> String {
        let mut output = String::new();
        self.write_input_metrics(&mut output);
        self.write_system_metrics(&mut output);
        output
    }

    fn write_input_metrics(&self, output: &mut String) {
        let _ = writeln!(output, "input_callbacks={}", self.input_callbacks);
        let _ = writeln!(
            output,
            "input_hook_lotus_max_us={}",
            self.input_hook_lotus_max_us
        );
        write_histogram(
            output,
            "input_hook_lotus_histogram",
            &self.input_hook_lotus_histogram,
        );
        let _ = writeln!(
            output,
            "input_hook_total_max_us={}",
            self.input_hook_total_max_us
        );
        write_histogram(
            output,
            "input_hook_total_histogram",
            &self.input_hook_total_histogram,
        );
        let _ = writeln!(
            output,
            "input_actions_enqueued={}",
            self.input_actions_enqueued
        );
        let _ = writeln!(
            output,
            "input_actions_coalesced={}",
            self.input_actions_coalesced
        );
        let _ = writeln!(
            output,
            "input_actions_dropped={}",
            self.input_actions_dropped
        );
        let _ = writeln!(
            output,
            "input_mailbox_high_water={}",
            self.input_mailbox_high_water
        );
        let _ = writeln!(output, "input_wakes_posted={}", self.input_wakes_posted);
        let _ = writeln!(
            output,
            "input_wakes_coalesced={}",
            self.input_wakes_coalesced
        );
        let _ = writeln!(output, "input_wake_failures={}", self.input_wake_failures);
        self.write_fail_open_metrics(output);
        let _ = writeln!(
            output,
            "input_replay_failures={}",
            self.input_replay_failures
        );
        let _ = writeln!(
            output,
            "input_sequence_cancels={}",
            self.input_sequence_cancels
        );
        let _ = writeln!(
            output,
            "input_win_bare_sequences={}",
            self.input_win_bare_sequences
        );
        let _ = writeln!(
            output,
            "input_win_sequences_disqualified={}",
            self.input_win_sequences_disqualified
        );
        let _ = writeln!(
            output,
            "input_start_cancel_attempts={}",
            self.input_start_cancel_attempts
        );
        let _ = writeln!(
            output,
            "input_start_cancel_successes={}",
            self.input_start_cancel_successes
        );
        let _ = writeln!(
            output,
            "input_start_cancel_failures={}",
            self.input_start_cancel_failures
        );
        let _ = writeln!(
            output,
            "input_delivery_max_us={}",
            self.input_delivery_max_us
        );
        write_histogram(
            output,
            "input_delivery_histogram",
            &self.input_delivery_histogram,
        );
    }

    fn write_fail_open_metrics(&self, output: &mut String) {
        let _ = writeln!(
            output,
            "input_fail_open_entries={}",
            self.input_fail_open_entries
        );
        let _ = writeln!(
            output,
            "input_fail_open_heartbeat_stale={}",
            self.input_fail_open_heartbeat_stale
        );
        let _ = writeln!(
            output,
            "input_fail_open_rejected_sequence={}",
            self.input_fail_open_rejected_sequence
        );
        let _ = writeln!(
            output,
            "input_fail_open_mailbox_full={}",
            self.input_fail_open_mailbox_full
        );
        let _ = writeln!(
            output,
            "input_fail_open_wake_failure={}",
            self.input_fail_open_wake_failure
        );
        let _ = writeln!(
            output,
            "input_fail_open_panic={}",
            self.input_fail_open_panic
        );
        let _ = writeln!(
            output,
            "input_fail_open_shutdown={}",
            self.input_fail_open_shutdown
        );
        let _ = writeln!(
            output,
            "input_cleanup_requested={}",
            self.input_cleanup_requested
        );
        let _ = writeln!(
            output,
            "input_cleanup_completed={}",
            self.input_cleanup_completed
        );
        let _ = writeln!(
            output,
            "input_cleanup_redundant_suppressed={}",
            self.input_cleanup_redundant_suppressed
        );
        let _ = writeln!(
            output,
            "input_cleanup_active_sequence_cancels={}",
            self.input_cleanup_active_sequence_cancels
        );
        let _ = writeln!(output, "input_cleanup_idle={}", self.input_cleanup_idle);
    }

    fn write_system_metrics(&self, output: &mut String) {
        let _ = writeln!(output, "ui_messages={}", self.ui_messages);
        let _ = writeln!(output, "ui_message_stalls={}", self.ui_message_stalls);
        let _ = writeln!(output, "ui_message_max_us={}", self.ui_message_max_us);
        write_histogram(output, "ui_message_histogram", &self.ui_message_histogram);
        for phase in [
            UiMessagePhase::Tracker,
            UiMessagePhase::Dispatch,
            UiMessagePhase::WindowDrain,
            UiMessagePhase::SettingsDrain,
            UiMessagePhase::SwitcherDrain,
            UiMessagePhase::MonitorDrain,
            UiMessagePhase::Wake,
            UiMessagePhase::MonitorSync,
            UiMessagePhase::Frame,
        ] {
            let index = phase.index();
            let _ = writeln!(
                output,
                "ui_phase_{}_max_us={}",
                phase.name(),
                self.ui_phase_max_us[index]
            );
            write_histogram(
                output,
                &format!("ui_phase_{}_histogram", phase.name()),
                &self.ui_phase_histogram[index],
            );
        }
        let _ = writeln!(
            output,
            "ui_messages_event_drain={}",
            self.ui_messages_event_drain
        );
        let _ = writeln!(
            output,
            "ui_messages_monitor_sync={}",
            self.ui_messages_monitor_sync
        );
        let _ = writeln!(output, "ui_messages_frame={}", self.ui_messages_frame);
        let _ = writeln!(output, "ui_message_severe={}", self.ui_message_severe);
        let _ = writeln!(output, "ui_message_critical={}", self.ui_message_critical);
        for event in &self.slow_ui_events {
            let _ = writeln!(
                output,
                "slow_ui_event=timestamp_ms:{},message:{},category:{},total_us:{},accounted_us:{},slowest_phase:{},slowest_phase_us:{},windows:{},monitors:{},dirty:{},animating:{},graphics_generation:{},graphics_recovered:{},visible:{},input_fail_open:{}",
                event.timestamp_ms,
                event.message_id,
                event.category,
                event.total_us,
                event.accounted_us,
                event.slowest_phase,
                event.slowest_phase_us,
                event.window_count,
                event.monitor_replica_count,
                event.dirty_surface_mask,
                event.animating_surface_mask,
                event.graphics_generation,
                event.graphics_recovered,
                event.visible_feature_mask,
                event.input_fail_open,
            );
        }
        for layout in &self.layouts {
            let prefix = format!("layout_{}", layout.operation.name());
            let _ = writeln!(output, "{prefix}_calls={}", layout.calls);
            let _ = writeln!(output, "{prefix}_max_us={}", layout.max_us);
            write_histogram(output, &format!("{prefix}_histogram"), &layout.histogram);
        }
        self.write_tracker_metrics(output);
        self.write_worker_metrics(output);
    }

    fn write_tracker_metrics(&self, output: &mut String) {
        for phase in &self.tracker.ui_phases {
            let prefix = format!("tracker_ui_{}", phase.phase.name());
            let _ = writeln!(output, "{prefix}_calls={}", phase.calls);
            let _ = writeln!(output, "{prefix}_max_us={}", phase.max_us);
            write_histogram(output, &format!("{prefix}_histogram"), &phase.histogram);
        }
        let _ = writeln!(output, "window_enumerations={}", self.window_enumerations);
        let _ = writeln!(
            output,
            "window_enumeration_max_us={}",
            self.window_enumeration_max_us
        );
        let _ = writeln!(output, "window_publishes={}", self.window_publishes);
        let _ = writeln!(output, "window_unchanged={}", self.window_unchanged);
        let _ = writeln!(
            output,
            "process_metadata_hits={}",
            self.process_metadata_hits
        );
        let _ = writeln!(
            output,
            "process_metadata_misses={}",
            self.process_metadata_misses
        );
        let _ = writeln!(
            output,
            "tracker_refresh_requests={}",
            self.tracker.refresh_requests
        );
        let _ = writeln!(
            output,
            "tracker_refresh_requests_coalesced={}",
            self.tracker.refresh_requests_coalesced
        );
        let _ = writeln!(
            output,
            "tracker_worker_refresh_executions={}",
            self.tracker.worker_refresh_executions
        );
        let _ = writeln!(
            output,
            "tracker_ui_wakes_posted={}",
            self.tracker.ui_wakes_posted
        );
        let _ = writeln!(
            output,
            "tracker_ui_wakes_coalesced={}",
            self.tracker.ui_wakes_coalesced
        );
        let _ = writeln!(
            output,
            "tracker_ui_wake_post_failures={}",
            self.tracker.ui_wake_post_failures
        );
        self.write_application_metrics(output);
        self.write_process_resource_metrics(output);
    }

    fn write_application_metrics(&self, output: &mut String) {
        for (name, value) in [
            (
                "application_catalog_generation",
                self.application.catalog_generation,
            ),
            (
                "application_catalog_entries",
                self.application.catalog_entries,
            ),
            (
                "application_catalog_duplicate_merges",
                self.application.catalog_duplicate_merges,
            ),
            (
                "application_catalog_ambiguous_aliases",
                self.application.catalog_ambiguous_aliases,
            ),
            (
                "application_catalog_build_max_us",
                self.application.catalog_build_max_us,
            ),
            (
                "window_identity_fact_hits",
                self.application.window_fact_hits,
            ),
            (
                "window_identity_fact_misses",
                self.application.window_fact_misses,
            ),
            (
                "window_identity_fact_max_us",
                self.application.window_fact_max_us,
            ),
            (
                "application_resolution_cache_hits",
                self.application.resolution_cache_hits,
            ),
            (
                "application_resolution_cache_misses",
                self.application.resolution_cache_misses,
            ),
            (
                "application_resolution_exact_registered",
                self.application.resolution_exact_registered,
            ),
            (
                "application_resolution_exact_relaunch",
                self.application.resolution_exact_relaunch,
            ),
            (
                "application_resolution_exact_provider",
                self.application.resolution_exact_provider,
            ),
            (
                "application_resolution_exact_path",
                self.application.resolution_exact_path,
            ),
            (
                "application_resolution_unique_alias",
                self.application.resolution_unique_alias,
            ),
            (
                "application_resolution_ambiguous",
                self.application.resolution_ambiguous,
            ),
            (
                "application_resolution_unregistered",
                self.application.resolution_unregistered,
            ),
            (
                "application_resolution_prevented",
                self.application.resolution_prevented,
            ),
            (
                "application_resolution_total_us",
                self.application.resolution_total_us,
            ),
            (
                "application_resolution_max_us",
                self.application.resolution_max_us,
            ),
            (
                "dock_projection_calls",
                self.application.dock_projection_calls,
            ),
            (
                "dock_projection_max_us",
                self.application.dock_projection_max_us,
            ),
        ] {
            let _ = writeln!(output, "{name}={value}");
        }
    }

    fn write_process_resource_metrics(&self, output: &mut String) {
        let _ = writeln!(
            output,
            "process_resources_success={}",
            self.process_resources.success
        );
        let _ = writeln!(
            output,
            "process_resources_working_set_bytes={}",
            self.process_resources.working_set_bytes
        );
        let _ = writeln!(
            output,
            "process_resources_private_bytes={}",
            self.process_resources.private_bytes
        );
        let _ = writeln!(
            output,
            "process_resources_handle_count={}",
            self.process_resources.handle_count
        );
        let _ = writeln!(
            output,
            "process_resources_thread_count={}",
            self.process_resources.thread_count
        );
    }

    fn write_worker_metrics(&self, output: &mut String) {
        let _ = writeln!(output, "badge_events={}", self.badge_events);
        let _ = writeln!(output, "badge_scans={}", self.badge_scans);
        let _ = writeln!(output, "badge_coalesced={}", self.badge_coalesced);
        let _ = writeln!(output, "badge_scan_max_us={}", self.badge_scan_max_us);
        write_histogram(output, "badge_scan_histogram", &self.badge_scan_histogram);
        let _ = writeln!(
            output,
            "badge_uia_enumeration_max_us={}",
            self.badge_uia_enumeration_max_us
        );
        let _ = writeln!(
            output,
            "badge_property_reads_max_us={}",
            self.badge_property_reads_max_us
        );
        let _ = writeln!(
            output,
            "badge_snapshot_comparison_max_us={}",
            self.badge_snapshot_comparison_max_us
        );
        let _ = writeln!(output, "badge_elements={}", self.badge_elements);
        let _ = writeln!(
            output,
            "badge_supported_elements={}",
            self.badge_supported_elements
        );
        let _ = writeln!(output, "badge_provider_reads={}", self.badge_provider_reads);
        let _ = writeln!(output, "flyout_attempts={}", self.flyout_attempts);
        let _ = writeln!(output, "flyout_max_us={}", self.flyout_max_us);
        write_histogram(output, "flyout_histogram", &self.flyout_histogram);
        let _ = writeln!(
            output,
            "flyout_worker_start_max_us={}",
            self.flyout_worker_start_max_us
        );
        let _ = writeln!(
            output,
            "flyout_discovery_wait_max_us={}",
            self.flyout_discovery_wait_max_us
        );
        let _ = writeln!(
            output,
            "flyout_bridge_configuration_max_us={}",
            self.flyout_bridge_configuration_max_us
        );
        let _ = writeln!(
            output,
            "flyout_positioning_max_us={}",
            self.flyout_positioning_max_us
        );
        let _ = writeln!(output, "flyout_timeouts={}", self.flyout_timeouts);
        let _ = writeln!(output, "flyout_successes={}", self.flyout_successes);
        let _ = writeln!(output, "flyout_superseded={}", self.flyout_superseded);
        let _ = writeln!(output, "switcher_requests={}", self.switcher_requests);
        let _ = writeln!(output, "switcher_results={}", self.switcher_results);
        for cache in &self.caches {
            let prefix = format!("cache_{}", cache.class.name());
            let _ = writeln!(output, "{prefix}_entries={}", cache.current_entries);
            let _ = writeln!(output, "{prefix}_bytes={}", cache.current_bytes);
            let _ = writeln!(output, "{prefix}_budget={}", cache.budget);
            let _ = writeln!(output, "{prefix}_hits={}", cache.hits);
            let _ = writeln!(output, "{prefix}_misses={}", cache.misses);
            let _ = writeln!(output, "{prefix}_inserts={}", cache.inserts);
            let _ = writeln!(output, "{prefix}_replacements={}", cache.replacements);
            let _ = writeln!(output, "{prefix}_evictions={}", cache.evictions);
            let _ = writeln!(output, "{prefix}_clears={}", cache.clears);
        }
    }
}

fn write_histogram(output: &mut String, name: &str, histogram: &[u64; HISTOGRAM_BUCKETS]) {
    let _ = write!(output, "{name}=");
    for (index, value) in histogram.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let _ = write!(output, "{value}");
    }
    output.push('\n');
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
