use super::{
    ApplicationMetricsSnapshot, CacheClass, CacheSnapshot, LAYOUT_OPERATIONS,
    LayoutOperation, LayoutSnapshot, Ordering, ProcessResourceSample,
    ResponsivenessMetrics, ResponsivenessSnapshot, SlowUiEvent, TRACKER_UI_PHASES,
    TrackerMetricsSnapshot, TrackerUiPhase, TrackerUiPhaseSnapshot, load_histogram,
};

impl ResponsivenessMetrics {
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
}
