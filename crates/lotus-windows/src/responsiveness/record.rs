use lotus_core::application::{ApplicationResolution, ResolutionEvidence};

use super::{
    AtomicU64, CacheClass, Duration, FlyoutPhaseMetrics, HISTOGRAM_BUCKETS,
    InputFailOpenReason, LayoutOperation, Ordering, ResponsivenessMetrics, SLOW_UI_EVENTS,
    SlowUiEvent, TrackerUiPhase, UiMessagePhase, duration_micros, histogram_bucket,
    record_duration, saturating_add, saturating_sub, usize_as_u64,
};

static LAST_SLOW_EVENT_LOG_MS: AtomicU64 = AtomicU64::new(0);

impl ResponsivenessMetrics {
    pub fn record_input_callback(&self) {
        saturating_add(&self.input_callbacks, 1);
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

    pub(super) fn load_layout_histogram(
        &self,
        operation: usize,
    ) -> [u64; HISTOGRAM_BUCKETS] {
        std::array::from_fn(|bucket| {
            self.layout_histogram[operation * HISTOGRAM_BUCKETS + bucket]
                .load(Ordering::Relaxed)
        })
    }

    fn tracker_ui_histogram_at(&self, phase: usize) -> &[AtomicU64] {
        let start = phase * HISTOGRAM_BUCKETS;
        &self.tracker_ui_histogram[start..start + HISTOGRAM_BUCKETS]
    }

    pub(super) fn load_tracker_ui_histogram(
        &self,
        phase: usize,
    ) -> [u64; HISTOGRAM_BUCKETS] {
        let source = self.tracker_ui_histogram_at(phase);
        std::array::from_fn(|index| source[index].load(Ordering::Relaxed))
    }

    fn ui_phase_histogram_at(&self, phase: usize) -> &[AtomicU64] {
        let start = phase * HISTOGRAM_BUCKETS;
        &self.ui_phase_histogram[start..start + HISTOGRAM_BUCKETS]
    }

    pub(super) fn load_ui_phase_histogram(&self, phase: usize) -> [u64; HISTOGRAM_BUCKETS] {
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
        let previous = LAST_SLOW_EVENT_LOG_MS.load(Ordering::Relaxed);
        if (previous == 0 || event.timestamp_ms.saturating_sub(previous) >= 1_000)
            && LAST_SLOW_EVENT_LOG_MS
                .compare_exchange(
                    previous,
                    event.timestamp_ms,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
        {
            crate::diagnostics::record_state(
                "ui.slow_event",
                &[
                    ("timestamp_ms", event.timestamp_ms),
                    ("message_id", u64::from(event.message_id)),
                    ("total_us", event.total_us),
                    ("slowest_phase_us", event.slowest_phase_us),
                    ("dirty_mask", u64::from(event.dirty_surface_mask)),
                    ("visible_mask", u64::from(event.visible_feature_mask)),
                    ("graphics_generation", event.graphics_generation),
                    ("input_fail_open", u64::from(event.input_fail_open)),
                ],
            );
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
