use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const HISTOGRAM_BUCKETS: usize = 8;
const HISTOGRAM_LIMITS_US: [u64; HISTOGRAM_BUCKETS - 1] =
    [100, 500, 1_000, 5_000, 10_000, 25_000, 100_000];

pub struct ResponsivenessMetrics {
    input_callbacks: AtomicU64,
    input_callback_max_us: AtomicU64,
    input_callback_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    input_actions_enqueued: AtomicU64,
    input_actions_coalesced: AtomicU64,
    input_actions_dropped: AtomicU64,
    input_mailbox_high_water: AtomicU64,
    input_wakes_posted: AtomicU64,
    input_wakes_coalesced: AtomicU64,
    input_wake_failures: AtomicU64,
    input_fail_open_entries: AtomicU64,
    input_replay_failures: AtomicU64,
    input_sequence_cancels: AtomicU64,
    input_delivery_max_us: AtomicU64,
    input_delivery_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    ui_messages: AtomicU64,
    ui_message_stalls: AtomicU64,
    ui_message_max_us: AtomicU64,
    ui_message_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    window_enumerations: AtomicU64,
    window_enumeration_max_us: AtomicU64,
    window_publishes: AtomicU64,
    window_unchanged: AtomicU64,
    process_metadata_hits: AtomicU64,
    process_metadata_misses: AtomicU64,
    badge_events: AtomicU64,
    badge_scans: AtomicU64,
    badge_coalesced: AtomicU64,
    badge_scan_max_us: AtomicU64,
    badge_scan_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    flyout_attempts: AtomicU64,
    flyout_max_us: AtomicU64,
    flyout_histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    switcher_requests: AtomicU64,
    switcher_results: AtomicU64,
    cache_entries: AtomicU64,
    cache_bytes: AtomicU64,
    cache_evictions: AtomicU64,
}

pub static METRICS: ResponsivenessMetrics = ResponsivenessMetrics::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponsivenessSnapshot {
    pub input_callbacks: u64,
    pub input_callback_max_us: u64,
    pub input_callback_histogram: [u64; HISTOGRAM_BUCKETS],
    pub input_actions_enqueued: u64,
    pub input_actions_coalesced: u64,
    pub input_actions_dropped: u64,
    pub input_mailbox_high_water: u64,
    pub input_wakes_posted: u64,
    pub input_wakes_coalesced: u64,
    pub input_wake_failures: u64,
    pub input_fail_open_entries: u64,
    pub input_replay_failures: u64,
    pub input_sequence_cancels: u64,
    pub input_delivery_max_us: u64,
    pub input_delivery_histogram: [u64; HISTOGRAM_BUCKETS],
    pub ui_messages: u64,
    pub ui_message_stalls: u64,
    pub ui_message_max_us: u64,
    pub ui_message_histogram: [u64; HISTOGRAM_BUCKETS],
    pub window_enumerations: u64,
    pub window_enumeration_max_us: u64,
    pub window_publishes: u64,
    pub window_unchanged: u64,
    pub process_metadata_hits: u64,
    pub process_metadata_misses: u64,
    pub badge_events: u64,
    pub badge_scans: u64,
    pub badge_coalesced: u64,
    pub badge_scan_max_us: u64,
    pub badge_scan_histogram: [u64; HISTOGRAM_BUCKETS],
    pub flyout_attempts: u64,
    pub flyout_max_us: u64,
    pub flyout_histogram: [u64; HISTOGRAM_BUCKETS],
    pub switcher_requests: u64,
    pub switcher_results: u64,
    pub cache_entries: u64,
    pub cache_bytes: u64,
    pub cache_evictions: u64,
}

impl ResponsivenessMetrics {
    const fn new() -> Self {
        Self {
            input_callbacks: AtomicU64::new(0),
            input_callback_max_us: AtomicU64::new(0),
            input_callback_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            input_actions_enqueued: AtomicU64::new(0),
            input_actions_coalesced: AtomicU64::new(0),
            input_actions_dropped: AtomicU64::new(0),
            input_mailbox_high_water: AtomicU64::new(0),
            input_wakes_posted: AtomicU64::new(0),
            input_wakes_coalesced: AtomicU64::new(0),
            input_wake_failures: AtomicU64::new(0),
            input_fail_open_entries: AtomicU64::new(0),
            input_replay_failures: AtomicU64::new(0),
            input_sequence_cancels: AtomicU64::new(0),
            input_delivery_max_us: AtomicU64::new(0),
            input_delivery_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            ui_messages: AtomicU64::new(0),
            ui_message_stalls: AtomicU64::new(0),
            ui_message_max_us: AtomicU64::new(0),
            ui_message_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            window_enumerations: AtomicU64::new(0),
            window_enumeration_max_us: AtomicU64::new(0),
            window_publishes: AtomicU64::new(0),
            window_unchanged: AtomicU64::new(0),
            process_metadata_hits: AtomicU64::new(0),
            process_metadata_misses: AtomicU64::new(0),
            badge_events: AtomicU64::new(0),
            badge_scans: AtomicU64::new(0),
            badge_coalesced: AtomicU64::new(0),
            badge_scan_max_us: AtomicU64::new(0),
            badge_scan_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            flyout_attempts: AtomicU64::new(0),
            flyout_max_us: AtomicU64::new(0),
            flyout_histogram: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            switcher_requests: AtomicU64::new(0),
            switcher_results: AtomicU64::new(0),
            cache_entries: AtomicU64::new(0),
            cache_bytes: AtomicU64::new(0),
            cache_evictions: AtomicU64::new(0),
        }
    }

    pub fn snapshot(&self) -> ResponsivenessSnapshot {
        ResponsivenessSnapshot {
            input_callbacks: self.input_callbacks.load(Ordering::Relaxed),
            input_callback_max_us: self.input_callback_max_us.load(Ordering::Relaxed),
            input_callback_histogram: load_histogram(&self.input_callback_histogram),
            input_actions_enqueued: self.input_actions_enqueued.load(Ordering::Relaxed),
            input_actions_coalesced: self.input_actions_coalesced.load(Ordering::Relaxed),
            input_actions_dropped: self.input_actions_dropped.load(Ordering::Relaxed),
            input_mailbox_high_water: self.input_mailbox_high_water.load(Ordering::Relaxed),
            input_wakes_posted: self.input_wakes_posted.load(Ordering::Relaxed),
            input_wakes_coalesced: self.input_wakes_coalesced.load(Ordering::Relaxed),
            input_wake_failures: self.input_wake_failures.load(Ordering::Relaxed),
            input_fail_open_entries: self.input_fail_open_entries.load(Ordering::Relaxed),
            input_replay_failures: self.input_replay_failures.load(Ordering::Relaxed),
            input_sequence_cancels: self.input_sequence_cancels.load(Ordering::Relaxed),
            input_delivery_max_us: self.input_delivery_max_us.load(Ordering::Relaxed),
            input_delivery_histogram: load_histogram(&self.input_delivery_histogram),
            ui_messages: self.ui_messages.load(Ordering::Relaxed),
            ui_message_stalls: self.ui_message_stalls.load(Ordering::Relaxed),
            ui_message_max_us: self.ui_message_max_us.load(Ordering::Relaxed),
            ui_message_histogram: load_histogram(&self.ui_message_histogram),
            window_enumerations: self.window_enumerations.load(Ordering::Relaxed),
            window_enumeration_max_us: self
                .window_enumeration_max_us
                .load(Ordering::Relaxed),
            window_publishes: self.window_publishes.load(Ordering::Relaxed),
            window_unchanged: self.window_unchanged.load(Ordering::Relaxed),
            process_metadata_hits: self.process_metadata_hits.load(Ordering::Relaxed),
            process_metadata_misses: self.process_metadata_misses.load(Ordering::Relaxed),
            badge_events: self.badge_events.load(Ordering::Relaxed),
            badge_scans: self.badge_scans.load(Ordering::Relaxed),
            badge_coalesced: self.badge_coalesced.load(Ordering::Relaxed),
            badge_scan_max_us: self.badge_scan_max_us.load(Ordering::Relaxed),
            badge_scan_histogram: load_histogram(&self.badge_scan_histogram),
            flyout_attempts: self.flyout_attempts.load(Ordering::Relaxed),
            flyout_max_us: self.flyout_max_us.load(Ordering::Relaxed),
            flyout_histogram: load_histogram(&self.flyout_histogram),
            switcher_requests: self.switcher_requests.load(Ordering::Relaxed),
            switcher_results: self.switcher_results.load(Ordering::Relaxed),
            cache_entries: self.cache_entries.load(Ordering::Relaxed),
            cache_bytes: self.cache_bytes.load(Ordering::Relaxed),
            cache_evictions: self.cache_evictions.load(Ordering::Relaxed),
        }
    }

    pub fn record_input_callback(&self) {
        saturating_add(&self.input_callbacks, 1);
    }

    pub fn record_input_callback_duration(&self, duration: Duration) {
        record_duration(
            duration_micros(duration),
            &self.input_callback_max_us,
            &self.input_callback_histogram,
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

    pub fn record_input_fail_open(&self) {
        saturating_add(&self.input_fail_open_entries, 1);
    }

    pub fn record_input_replay_failure(&self) {
        saturating_add(&self.input_replay_failures, 1);
    }

    pub fn record_input_sequence_cancel(&self) {
        saturating_add(&self.input_sequence_cancels, 1);
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

    pub fn record_flyout(&self, duration: Duration) {
        saturating_add(&self.flyout_attempts, 1);
        record_duration(
            duration_micros(duration),
            &self.flyout_max_us,
            &self.flyout_histogram,
        );
    }

    pub fn record_switcher_requests(&self, count: usize) {
        saturating_add(&self.switcher_requests, count as u64);
    }

    pub fn record_switcher_results(&self, count: usize) {
        saturating_add(&self.switcher_results, count as u64);
    }

    pub(crate) fn record_cache_insert(&self, bytes: usize) {
        saturating_add(&self.cache_entries, 1);
        saturating_add(&self.cache_bytes, usize_as_u64(bytes));
    }

    pub(crate) fn record_cache_bytes_replaced(&self, removed: usize, added: usize) {
        saturating_sub(&self.cache_bytes, usize_as_u64(removed));
        saturating_add(&self.cache_bytes, usize_as_u64(added));
    }

    pub(crate) fn record_cache_remove(&self, entries: usize, bytes: usize) {
        saturating_sub(&self.cache_entries, usize_as_u64(entries));
        saturating_sub(&self.cache_bytes, usize_as_u64(bytes));
    }

    pub(crate) fn record_cache_eviction(&self, bytes: usize) {
        self.record_cache_remove(1, bytes);
        saturating_add(&self.cache_evictions, 1);
    }
}

impl ResponsivenessSnapshot {
    pub fn to_text(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "input_callbacks={}", self.input_callbacks);
        let _ = writeln!(
            output,
            "input_callback_max_us={}",
            self.input_callback_max_us
        );
        write_histogram(
            &mut output,
            "input_callback_histogram",
            &self.input_callback_histogram,
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
        let _ = writeln!(
            output,
            "input_fail_open_entries={}",
            self.input_fail_open_entries
        );
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
            "input_delivery_max_us={}",
            self.input_delivery_max_us
        );
        write_histogram(
            &mut output,
            "input_delivery_histogram",
            &self.input_delivery_histogram,
        );
        let _ = writeln!(output, "ui_messages={}", self.ui_messages);
        let _ = writeln!(output, "ui_message_stalls={}", self.ui_message_stalls);
        let _ = writeln!(output, "ui_message_max_us={}", self.ui_message_max_us);
        write_histogram(
            &mut output,
            "ui_message_histogram",
            &self.ui_message_histogram,
        );
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
        let _ = writeln!(output, "badge_events={}", self.badge_events);
        let _ = writeln!(output, "badge_scans={}", self.badge_scans);
        let _ = writeln!(output, "badge_coalesced={}", self.badge_coalesced);
        let _ = writeln!(output, "badge_scan_max_us={}", self.badge_scan_max_us);
        write_histogram(
            &mut output,
            "badge_scan_histogram",
            &self.badge_scan_histogram,
        );
        let _ = writeln!(output, "flyout_attempts={}", self.flyout_attempts);
        let _ = writeln!(output, "flyout_max_us={}", self.flyout_max_us);
        write_histogram(&mut output, "flyout_histogram", &self.flyout_histogram);
        let _ = writeln!(output, "switcher_requests={}", self.switcher_requests);
        let _ = writeln!(output, "switcher_results={}", self.switcher_results);
        let _ = writeln!(output, "cache_entries={}", self.cache_entries);
        let _ = writeln!(output, "cache_bytes={}", self.cache_bytes);
        let _ = writeln!(output, "cache_evictions={}", self.cache_evictions);
        output
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
