//! Shared metric helpers for instrumentation libraries: seconds-based
//! duration histograms ([`crate::metrics::duration_histogram`]) and the
//! [`crate::metrics::InFlightGuard`] for in-flight work.

use opentelemetry::{
    KeyValue,
    metrics::{Histogram, Meter, UpDownCounter},
};

/// Explicit histogram bucket boundaries, in seconds, for request,
/// message-processing, and job durations.
pub const DURATION_HISTOGRAM_BOUNDARIES: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

/// Builds an `f64` histogram named `name` that records durations in
/// seconds (unit `s`) into [`DURATION_HISTOGRAM_BOUNDARIES`] buckets.
#[must_use]
pub fn duration_histogram(
    meter: &Meter,
    name: &'static str,
    description: &'static str,
) -> Histogram<f64> {
    meter
        .f64_histogram(name)
        .with_unit("s")
        .with_description(description)
        .with_boundaries(DURATION_HISTOGRAM_BOUNDARIES.to_vec())
        .build()
}

/// Counts one unit of in-flight work on an up-down counter for as long
/// as the guard lives: [`InFlightGuard::start`] adds 1 and dropping the
/// guard adds -1 with the same attributes, so cancelled or panicking
/// work is still subtracted.
pub struct InFlightGuard {
    counter: UpDownCounter<i64>,
    attributes: Vec<KeyValue>,
}

impl InFlightGuard {
    /// Adds 1 to `counter` with `attributes` and returns the guard that
    /// subtracts it again on drop.
    #[must_use = "dropping the guard immediately ends the in-flight measurement"]
    pub fn start(counter: &UpDownCounter<i64>, attributes: Vec<KeyValue>) -> Self {
        counter.add(1, &attributes);
        Self {
            counter: counter.clone(),
            attributes,
        }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.add(-1, &self.attributes);
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry::{Value, metrics::MeterProvider as _};

    use super::*;
    use crate::test_support::MetricsHarness;

    #[test]
    fn duration_histograms_use_the_shared_second_boundaries() {
        let harness = MetricsHarness::default();
        let meter = harness.provider().meter("test");
        let histogram = duration_histogram(&meter, "test.duration", "duration of test work");
        histogram.record(0.2, &[]);

        let points = harness.f64_histogram("test.duration");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].count, 1);
        assert_eq!(points[0].bounds, DURATION_HISTOGRAM_BOUNDARIES.to_vec());
    }

    #[test]
    fn in_flight_guards_count_while_alive_and_subtract_on_drop() {
        let harness = MetricsHarness::default();
        let meter = harness.provider().meter("test");
        let counter = meter.i64_up_down_counter("test.in_flight").build();
        let guard = InFlightGuard::start(&counter, vec![KeyValue::new("k", "v")]);

        let points = harness.i64_sum("test.in_flight");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].value, 1);
        assert_eq!(points[0].attribute("k"), Some(&Value::from("v")));

        drop(guard);
        let points = harness.i64_sum("test.in_flight");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].value, 0);
    }
}
