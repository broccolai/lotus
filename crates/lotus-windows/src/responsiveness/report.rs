use std::fmt::Write as _;

use super::{HISTOGRAM_BUCKETS, ResponsivenessSnapshot, UiMessagePhase};

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
