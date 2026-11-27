//! An in-memory meter provider and readable data points for asserting
//! on recorded metrics ([`crate::test_support::MetricsHarness`]) —
//! compiled only for tests or when the `test-support` feature is enabled.

use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::metrics::{
    InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
    data::{AggregatedMetrics, Metric, MetricData, ResourceMetrics, ScopeMetrics},
};

/// An `SdkMeterProvider` backed by an [`InMemoryMetricExporter`], for
/// asserting on the metrics recorded against
/// [`MetricsHarness::provider`] without touching any global state.
pub struct MetricsHarness {
    provider: SdkMeterProvider,
    exporter: InMemoryMetricExporter,
}

impl Default for MetricsHarness {
    fn default() -> Self {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        Self { provider, exporter }
    }
}

impl MetricsHarness {
    /// The provider to pass into the code under test.
    #[must_use]
    pub fn provider(&self) -> &SdkMeterProvider {
        &self.provider
    }

    fn flush(&self) -> Vec<ResourceMetrics> {
        self.exporter.reset();
        self.provider.force_flush().expect("flushing metrics");
        self.exporter
            .get_finished_metrics()
            .expect("reading metrics")
    }

    /// Flushes the provider and returns the cumulative data points of the
    /// `f64` histogram named `name`, or an empty [`Vec`] when the metric
    /// was never recorded.
    ///
    /// # Panics
    ///
    /// Panics when flushing or reading fails, or when the metric named
    /// `name` is not an `f64` histogram.
    #[must_use]
    pub fn f64_histogram(&self, name: &str) -> Vec<HistogramPoint> {
        let finished = self.flush();
        match find_metric(&finished, name).map(Metric::data) {
            None => Vec::new(),
            Some(AggregatedMetrics::F64(MetricData::Histogram(histogram))) => histogram
                .data_points()
                .map(|point| HistogramPoint {
                    attributes: point.attributes().cloned().collect(),
                    count: point.count(),
                    bounds: point.bounds().collect(),
                })
                .collect(),
            Some(other) => panic!("{name} is not an f64 histogram: {other:?}"),
        }
    }

    /// Flushes the provider and returns the cumulative data points of the
    /// `i64` sum named `name`, covering up-down counters and observable
    /// up-down counters, or an empty [`Vec`] when the metric was never
    /// recorded.
    ///
    /// # Panics
    ///
    /// Panics when flushing or reading fails, or when the metric named
    /// `name` is not an `i64` sum.
    #[must_use]
    pub fn i64_sum(&self, name: &str) -> Vec<SumPoint> {
        let finished = self.flush();
        match find_metric(&finished, name).map(Metric::data) {
            None => Vec::new(),
            Some(AggregatedMetrics::I64(MetricData::Sum(sum))) => sum
                .data_points()
                .map(|point| SumPoint {
                    attributes: point.attributes().cloned().collect(),
                    value: point.value(),
                })
                .collect(),
            Some(other) => panic!("{name} is not an i64 sum: {other:?}"),
        }
    }

    /// Flushes the provider and returns the cumulative data points of the
    /// `u64` sum named `name`, covering counters, or an empty [`Vec`] when
    /// the metric was never recorded.
    ///
    /// # Panics
    ///
    /// Panics when flushing or reading fails, or when the metric named
    /// `name` is not a `u64` sum.
    #[must_use]
    pub fn u64_sum(&self, name: &str) -> Vec<U64SumPoint> {
        let finished = self.flush();
        match find_metric(&finished, name).map(Metric::data) {
            None => Vec::new(),
            Some(AggregatedMetrics::U64(MetricData::Sum(sum))) => sum
                .data_points()
                .map(|point| U64SumPoint {
                    attributes: point.attributes().cloned().collect(),
                    value: point.value(),
                })
                .collect(),
            Some(other) => panic!("{name} is not a u64 sum: {other:?}"),
        }
    }

    /// Flushes the provider and returns the data points of the `i64` gauge
    /// named `name`, or an empty [`Vec`] when nothing was observed. For an
    /// observable gauge, the points are only the attribute sets observed
    /// during this flush.
    ///
    /// # Panics
    ///
    /// Panics when flushing or reading fails, or when the metric named
    /// `name` is not an `i64` gauge.
    #[must_use]
    pub fn i64_gauge(&self, name: &str) -> Vec<GaugePoint> {
        let finished = self.flush();
        match find_metric(&finished, name).map(Metric::data) {
            None => Vec::new(),
            Some(AggregatedMetrics::I64(MetricData::Gauge(gauge))) => gauge
                .data_points()
                .map(|point| GaugePoint {
                    attributes: point.attributes().cloned().collect(),
                    value: point.value(),
                })
                .collect(),
            Some(other) => panic!("{name} is not an i64 gauge: {other:?}"),
        }
    }
}

/// One cumulative `f64` histogram data point flushed from a
/// [`MetricsHarness`].
#[must_use]
pub struct HistogramPoint {
    /// The attributes recorded with the point.
    pub attributes: Vec<KeyValue>,
    /// The number of recordings accumulated into the point.
    pub count: u64,
    /// The explicit bucket boundaries of the point, in seconds.
    pub bounds: Vec<f64>,
}

impl HistogramPoint {
    /// The value of the attribute named `key`, when it was recorded.
    #[must_use]
    pub fn attribute(&self, key: &str) -> Option<&Value> {
        find_attribute(&self.attributes, key)
    }
}

/// One cumulative `i64` sum data point flushed from a [`MetricsHarness`].
#[must_use]
pub struct SumPoint {
    /// The attributes recorded with the point.
    pub attributes: Vec<KeyValue>,
    /// The accumulated value of the point.
    pub value: i64,
}

impl SumPoint {
    /// The value of the attribute named `key`, when it was recorded.
    #[must_use]
    pub fn attribute(&self, key: &str) -> Option<&Value> {
        find_attribute(&self.attributes, key)
    }
}

/// One cumulative `u64` sum data point flushed from a [`MetricsHarness`].
#[must_use]
pub struct U64SumPoint {
    /// The attributes recorded with the point.
    pub attributes: Vec<KeyValue>,
    /// The accumulated value of the point.
    pub value: u64,
}

impl U64SumPoint {
    /// The value of the attribute named `key`, when it was recorded.
    #[must_use]
    pub fn attribute(&self, key: &str) -> Option<&Value> {
        find_attribute(&self.attributes, key)
    }
}

/// One `i64` gauge data point flushed from a [`MetricsHarness`].
#[must_use]
pub struct GaugePoint {
    /// The attributes observed with the point.
    pub attributes: Vec<KeyValue>,
    /// The last observed value of the point.
    pub value: i64,
}

impl GaugePoint {
    /// The value of the attribute named `key`, when it was observed.
    #[must_use]
    pub fn attribute(&self, key: &str) -> Option<&Value> {
        find_attribute(&self.attributes, key)
    }
}

fn find_attribute<'a>(attributes: &'a [KeyValue], key: &str) -> Option<&'a Value> {
    attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| &kv.value)
}

fn find_metric<'a>(finished: &'a [ResourceMetrics], name: &str) -> Option<&'a Metric> {
    finished
        .last()?
        .scope_metrics()
        .flat_map(ScopeMetrics::metrics)
        .find(|metric| metric.name() == name)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use opentelemetry::metrics::MeterProvider as _;

    use super::*;

    #[test]
    fn unrecorded_metrics_have_no_points() {
        let harness = MetricsHarness::default();
        assert!(harness.f64_histogram("nope").is_empty());
        assert!(harness.i64_sum("nope").is_empty());
        assert!(harness.u64_sum("nope").is_empty());
        assert!(harness.i64_gauge("nope").is_empty());
    }

    #[test]
    fn u64_counters_accumulate() {
        let harness = MetricsHarness::default();
        let meter = harness.provider().meter("test");
        let counter = meter.u64_counter("test.count").build();
        counter.add(2, &[KeyValue::new("k", "v")]);
        counter.add(3, &[KeyValue::new("k", "v")]);

        let points = harness.u64_sum("test.count");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].value, 5);
        assert_eq!(points[0].attribute("k"), Some(&Value::from("v")));
    }

    #[test]
    fn observable_gauges_report_only_current_observations() {
        let harness = MetricsHarness::default();
        let meter = harness.provider().meter("test");
        let observing = Arc::new(AtomicBool::new(true));
        let observed = Arc::clone(&observing);
        let _gauge = meter
            .i64_observable_gauge("test.gauge")
            .with_callback(move |observer| {
                if observed.load(Ordering::SeqCst) {
                    observer.observe(7, &[KeyValue::new("p", "0")]);
                }
            })
            .build();

        let points = harness.i64_gauge("test.gauge");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].value, 7);
        assert_eq!(points[0].attribute("p"), Some(&Value::from("0")));

        observing.store(false, Ordering::SeqCst);
        assert!(harness.i64_gauge("test.gauge").is_empty());
    }

    #[test]
    #[should_panic(expected = "is not an i64 gauge")]
    fn reading_a_sum_as_a_gauge_panics() {
        let harness = MetricsHarness::default();
        let meter = harness.provider().meter("test");
        let counter = meter.i64_up_down_counter("test.sum").build();
        counter.add(1, &[]);

        let _ = harness.i64_gauge("test.sum");
    }

    #[test]
    #[should_panic(expected = "is not an i64 sum")]
    fn reading_a_histogram_as_a_sum_panics() {
        let harness = MetricsHarness::default();
        let meter = harness.provider().meter("test");
        let histogram = meter.f64_histogram("test.duration").build();
        histogram.record(0.2, &[]);

        let _ = harness.i64_sum("test.duration");
    }
}
