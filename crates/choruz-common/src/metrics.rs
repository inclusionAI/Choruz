//! Process-wide Prometheus registry.
//!
//! Every Choruz binary serves one `/metrics` endpoint that encodes this
//! registry, so a feature adds a metric by registering it once from a static
//! and incrementing it where the event happens; no handler lists metrics.
//!
//! ```
//! use std::sync::LazyLock;
//! use choruz_common::metrics::{self, IntCounter};
//!
//! static CREATES: LazyLock<IntCounter> = LazyLock::new(|| {
//!     metrics::register_counter("choruz_example_creates_total", "Example cards created.")
//! });
//!
//! CREATES.inc();
//! assert!(metrics::text().contains("choruz_example_creates_total 1"));
//! ```
//!
//! A `LazyLock` registers on first use. A metric that must report `0` before
//! its first event is forced (`LazyLock::force`) at startup by the component
//! that owns it.

use std::sync::LazyLock;

use prometheus::{Encoder, HistogramOpts, Opts, Registry, TextEncoder, core::Collector};
pub use prometheus::{Histogram, IntCounter, IntCounterVec, IntGauge};

/// `Content-Type` of the body [`text`] produces.
pub const TEXT_CONTENT_TYPE: &str = "text/plain; version=0.0.4";

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

/// Registers a monotonically increasing integer counter.
///
/// # Panics
///
/// Panics when `name` is not a valid metric name or is already registered.
/// Every metric is registered exactly once from a static, so a duplicate is a
/// programming error surfaced on first use, not a runtime condition.
pub fn register_counter(name: &str, help: &str) -> IntCounter {
    let counter = IntCounter::new(name, help)
        .unwrap_or_else(|error| panic!("invalid counter {name}: {error}"));
    register(name, counter.clone());
    counter
}

/// Registers a family of integer counters keyed by `label_names`.
///
/// # Panics
///
/// Same contract as [`register_counter`].
pub fn register_counter_vec(name: &str, help: &str, label_names: &[&str]) -> IntCounterVec {
    let counters = IntCounterVec::new(Opts::new(name, help), label_names)
        .unwrap_or_else(|error| panic!("invalid counter vec {name}: {error}"));
    register(name, counters.clone());
    counters
}

/// Registers an integer gauge.
///
/// # Panics
///
/// Same contract as [`register_counter`].
pub fn register_gauge(name: &str, help: &str) -> IntGauge {
    let gauge =
        IntGauge::new(name, help).unwrap_or_else(|error| panic!("invalid gauge {name}: {error}"));
    register(name, gauge.clone());
    gauge
}

/// Registers a histogram with cumulative `buckets` (upper bounds, ascending);
/// the encoder appends the `+Inf` bucket, `_sum` and `_count` samples.
///
/// # Panics
///
/// Same contract as [`register_counter`].
pub fn register_histogram(name: &str, help: &str, buckets: Vec<f64>) -> Histogram {
    let histogram = Histogram::with_opts(HistogramOpts::new(name, help).buckets(buckets))
        .unwrap_or_else(|error| panic!("invalid histogram {name}: {error}"));
    register(name, histogram.clone());
    histogram
}

/// Encodes every registered metric in the Prometheus text exposition format
/// (`# HELP` and `# TYPE` lines followed by the samples).
pub fn text() -> String {
    let mut buffer = Vec::new();
    TextEncoder::new()
        .encode(&REGISTRY.gather(), &mut buffer)
        .expect("encoding into a Vec<u8> cannot fail");
    String::from_utf8(buffer).expect("the text encoder emits UTF-8")
}

fn register(name: &str, collector: impl Collector + 'static) {
    REGISTRY
        .register(Box::new(collector))
        .unwrap_or_else(|error| panic!("register metric {name}: {error}"));
}

#[cfg(test)]
mod tests {
    use super::{register_counter, register_gauge, register_histogram, text};

    fn has_line(text: &str, expected: &str) -> bool {
        text.lines().any(|line| line == expected)
    }

    #[test]
    fn registered_counter_appears_in_text_with_its_type_line() {
        let counter = register_counter("choruz_metrics_test_total", "Test counter.");
        counter.inc();

        let text = text();
        assert!(has_line(&text, "# TYPE choruz_metrics_test_total counter"));
        assert!(has_line(&text, "choruz_metrics_test_total 1"));
    }

    #[test]
    fn gauge_and_histogram_samples_are_encoded() {
        let gauge = register_gauge("choruz_metrics_test_gauge", "Test gauge.");
        gauge.set(7);
        let histogram = register_histogram(
            "choruz_metrics_test_duration",
            "Test histogram.",
            vec![0.05, 0.2, 1.0],
        );
        histogram.observe(0.1);

        let text = text();
        assert!(has_line(&text, "# TYPE choruz_metrics_test_gauge gauge"));
        assert!(has_line(&text, "choruz_metrics_test_gauge 7"));
        assert!(has_line(
            &text,
            "# TYPE choruz_metrics_test_duration histogram"
        ));
        assert!(has_line(
            &text,
            "choruz_metrics_test_duration_bucket{le=\"0.05\"} 0"
        ));
        assert!(has_line(
            &text,
            "choruz_metrics_test_duration_bucket{le=\"0.2\"} 1"
        ));
        assert!(has_line(
            &text,
            "choruz_metrics_test_duration_bucket{le=\"+Inf\"} 1"
        ));
        assert!(has_line(&text, "choruz_metrics_test_duration_count 1"));
    }

    #[test]
    #[should_panic(expected = "register metric choruz_metrics_test_duplicate_total")]
    fn duplicate_registration_panics() {
        let _first = register_counter("choruz_metrics_test_duplicate_total", "First.");
        let _second = register_counter("choruz_metrics_test_duplicate_total", "Second.");
    }
}
