use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use opentelemetry::{KeyValue, metrics::AsyncInstrument};
use serde::Deserialize;

use super::partition_id;

/// The lag snapshots of the live consumers built from one
/// [`crate::metrics::ConsumerMetrics`]; dropped consumers are pruned on
/// observation.
#[derive(Default)]
pub(crate) struct LagRegistry {
    snapshots: Mutex<Vec<Weak<LagSnapshot>>>,
}

impl LagRegistry {
    /// Adds `snapshot` to the consumers observed by the lag gauge.
    pub(crate) fn register(&self, snapshot: &Arc<LagSnapshot>) {
        self.snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::downgrade(snapshot));
    }

    /// Observes the lag of every live consumer and prunes dropped ones.
    pub(crate) fn observe(&self, observer: &dyn AsyncInstrument<i64>) {
        let mut snapshots = self
            .snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        snapshots.retain(|snapshot| snapshot.strong_count() > 0);
        for snapshot in snapshots.iter().filter_map(Weak::upgrade) {
            snapshot.observe(observer);
        }
    }
}

/// The latest per-partition lag of one consumer on one topic, replaced
/// by every librdkafka statistics report.
pub(crate) struct LagSnapshot {
    topic: String,
    attributes: Vec<KeyValue>,
    partitions: Mutex<Vec<(i32, i64)>>,
}

impl LagSnapshot {
    /// An empty snapshot for a consumer on `topic`, observed with the
    /// consumer's shared `attributes` plus each partition id.
    pub(crate) fn new(topic: &str, attributes: Vec<KeyValue>) -> Self {
        Self {
            topic: topic.to_owned(),
            attributes,
            partitions: Mutex::new(Vec::new()),
        }
    }

    /// Replaces the snapshot from a librdkafka statistics JSON report; an
    /// unparsable report clears it.
    pub(crate) fn update(&self, statistics: &[u8]) {
        let lags = match serde_json::from_slice::<Statistics>(statistics) {
            Ok(parsed) => lags(&parsed, &self.topic),
            Err(error) => {
                tracing::warn!(
                    %error,
                    topic = %self.topic,
                    "could not parse kafka consumer statistics"
                );
                Vec::new()
            }
        };
        *self
            .partitions
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = lags;
    }

    /// Observes the lag of every partition in the snapshot.
    fn observe(&self, observer: &dyn AsyncInstrument<i64>) {
        let partitions = self
            .partitions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for &(partition, lag) in partitions.iter() {
            let attributes: Vec<KeyValue> = self
                .attributes
                .iter()
                .cloned()
                .chain([partition_id(partition)])
                .collect();
            observer.observe(lag, &attributes);
        }
    }

    /// The current `(partition, lag)` pairs.
    #[cfg(test)]
    fn partitions(&self) -> Vec<(i32, i64)> {
        self.partitions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The librdkafka statistics fields the lag gauge reads.
#[derive(Deserialize)]
struct Statistics {
    #[serde(default)]
    topics: HashMap<String, TopicStatistics>,
}

/// The librdkafka statistics fields the lag gauge reads.
#[derive(Deserialize)]
struct TopicStatistics {
    #[serde(default)]
    partitions: HashMap<String, PartitionStatistics>,
}

/// The librdkafka statistics fields the lag gauge reads.
#[derive(Deserialize)]
struct PartitionStatistics {
    partition: i32,
    desired: bool,
    consumer_lag: i64,
    consumer_lag_stored: i64,
}

/// The `(partition, lag)` pairs of `topic`'s assigned partitions with a
/// known lag, sorted by partition.
fn lags(statistics: &Statistics, topic: &str) -> Vec<(i32, i64)> {
    let mut lags: Vec<(i32, i64)> = statistics
        .topics
        .get(topic)
        .into_iter()
        .flat_map(|topic| topic.partitions.values())
        .filter_map(|partition| partition_lag(partition).map(|lag| (partition.partition, lag)))
        .collect();
    lags.sort_unstable();
    lags
}

/// The lag of an assigned partition: the stored-offset lag, else the
/// committed-offset lag; `None` when neither is known.
fn partition_lag(partition: &PartitionStatistics) -> Option<i64> {
    if partition.partition < 0 || !partition.desired {
        return None;
    }
    [partition.consumer_lag_stored, partition.consumer_lag]
        .into_iter()
        .find(|lag| *lag >= 0)
}

#[cfg(test)]
mod tests {
    use super::LagSnapshot;

    const FIXTURE: &str = r#"{
  "name": "greetings-consumer#consumer-1",
  "client_id": "greetings-consumer",
  "type": "consumer",
  "ts": 5016483227792,
  "time": 1764158400,
  "age": 15000123,
  "replyq": 0,
  "msg_cnt": 0,
  "msg_size": 0,
  "msg_max": 100000,
  "msg_size_max": 1073741824,
  "simple_cnt": 0,
  "metadata_cache_cnt": 1,
  "brokers": {},
  "topics": {
    "greetings": {
      "topic": "greetings",
      "age": 14000,
      "metadata_age": 9000,
      "batchsize": { "min": 0, "max": 0, "avg": 0, "sum": 0, "stddev": 0, "p50": 0, "p75": 0, "p90": 0, "p95": 0, "p99": 0, "p99_99": 0, "outofrange": 0, "hdrsize": 0, "cnt": 0 },
      "batchcnt": { "min": 0, "max": 0, "avg": 0, "sum": 0, "stddev": 0, "p50": 0, "p75": 0, "p90": 0, "p95": 0, "p99": 0, "p99_99": 0, "outofrange": 0, "hdrsize": 0, "cnt": 0 },
      "partitions": {
        "0": {
          "partition": 0, "broker": 1, "leader": 1, "desired": true, "unknown": false,
          "msgq_cnt": 0, "msgq_bytes": 0, "xmit_msgq_cnt": 0, "xmit_msgq_bytes": 0,
          "fetchq_cnt": 3, "fetchq_size": 300, "fetch_state": "active",
          "query_offset": -1001, "next_offset": 45, "app_offset": 42,
          "stored_offset": 40, "stored_leader_epoch": -1,
          "commited_offset": 38, "committed_offset": 38, "committed_leader_epoch": 0,
          "eof_offset": -1001, "lo_offset": 0, "hi_offset": 45, "ls_offset": 45,
          "consumer_lag": 7, "consumer_lag_stored": 5, "leader_epoch": 0,
          "txmsgs": 0, "txbytes": 0, "rxmsgs": 45, "rxbytes": 4500, "msgs": 45,
          "rx_ver_drops": 0, "msgs_inflight": 0, "next_ack_seq": 0, "next_err_seq": 0, "acked_msgid": 0
        },
        "1": {
          "partition": 1, "broker": 1, "leader": 1, "desired": true, "unknown": false,
          "msgq_cnt": 0, "msgq_bytes": 0, "xmit_msgq_cnt": 0, "xmit_msgq_bytes": 0,
          "fetchq_cnt": 0, "fetchq_size": 0, "fetch_state": "active",
          "query_offset": -1001, "next_offset": 12, "app_offset": -1001,
          "stored_offset": -1001, "stored_leader_epoch": -1,
          "commited_offset": 12, "committed_offset": 12, "committed_leader_epoch": 0,
          "eof_offset": 12, "lo_offset": 0, "hi_offset": 12, "ls_offset": 12,
          "consumer_lag": 0, "consumer_lag_stored": -1, "leader_epoch": 0,
          "txmsgs": 0, "txbytes": 0, "rxmsgs": 0, "rxbytes": 0, "msgs": 0,
          "rx_ver_drops": 0, "msgs_inflight": 0, "next_ack_seq": 0, "next_err_seq": 0, "acked_msgid": 0
        },
        "-1": {
          "partition": -1, "broker": -1, "leader": -1, "desired": false, "unknown": false,
          "msgq_cnt": 0, "msgq_bytes": 0, "xmit_msgq_cnt": 0, "xmit_msgq_bytes": 0,
          "fetchq_cnt": 0, "fetchq_size": 0, "fetch_state": "none",
          "query_offset": -1001, "next_offset": -1001, "app_offset": -1001,
          "stored_offset": -1001, "stored_leader_epoch": -1,
          "commited_offset": -1001, "committed_offset": -1001, "committed_leader_epoch": -1,
          "eof_offset": -1001, "lo_offset": -1001, "hi_offset": -1001, "ls_offset": -1001,
          "consumer_lag": -1, "consumer_lag_stored": -1, "leader_epoch": -1,
          "txmsgs": 0, "txbytes": 0, "rxmsgs": 0, "rxbytes": 0, "msgs": 0,
          "rx_ver_drops": 0, "msgs_inflight": 0, "next_ack_seq": 0, "next_err_seq": 0, "acked_msgid": 0
        }
      }
    }
  },
  "cgrp": {
    "state": "up", "stateage": 14000, "join_state": "steady",
    "rebalance_age": 13000, "rebalance_cnt": 1, "rebalance_reason": "", "assignment_size": 2
  },
  "tx": 40, "tx_bytes": 4800, "rx": 39, "rx_bytes": 9100,
  "txmsgs": 0, "txmsg_bytes": 0, "rxmsgs": 45, "rxmsg_bytes": 4500
}"#;

    fn stats(partitions: &str) -> Vec<u8> {
        format!(r#"{{"topics":{{"orders":{{"partitions":{{{partitions}}}}}}}}}"#).into_bytes()
    }

    fn snapshot_of(statistics: &[u8]) -> Vec<(i32, i64)> {
        let snapshot = LagSnapshot::new("orders", vec![]);
        snapshot.update(statistics);
        snapshot.partitions()
    }

    #[test]
    fn stored_lag_is_preferred() {
        let partitions = snapshot_of(&stats(
            r#""0":{"partition":0,"desired":true,"consumer_lag":5,"consumer_lag_stored":3}"#,
        ));
        assert_eq!(partitions, [(0, 3)]);
    }

    #[test]
    fn committed_lag_is_the_fallback_when_nothing_is_stored() {
        let partitions = snapshot_of(&stats(
            r#""0":{"partition":0,"desired":true,"consumer_lag":4,"consumer_lag_stored":-1}"#,
        ));
        assert_eq!(partitions, [(0, 4)]);
    }

    #[test]
    fn unknown_lag_reports_nothing() {
        let partitions = snapshot_of(&stats(
            r#""0":{"partition":0,"desired":true,"consumer_lag":-1,"consumer_lag_stored":-1}"#,
        ));
        assert_eq!(partitions, []);
    }

    #[test]
    fn unassigned_and_internal_partitions_are_ignored() {
        let partitions = snapshot_of(&stats(concat!(
            r#""0":{"partition":0,"desired":false,"consumer_lag":5,"consumer_lag_stored":3},"#,
            r#""-1":{"partition":-1,"desired":false,"consumer_lag":5,"consumer_lag_stored":3}"#,
        )));
        assert_eq!(partitions, []);
    }

    #[test]
    fn other_topics_are_ignored() {
        let partitions = snapshot_of(
            br#"{"topics":{"payments":{"partitions":{
                "0":{"partition":0,"desired":true,"consumer_lag":5,"consumer_lag_stored":3}
            }}}}"#,
        );
        assert_eq!(partitions, []);
    }

    #[test]
    fn partitions_are_sorted() {
        let partitions = snapshot_of(&stats(concat!(
            r#""2":{"partition":2,"desired":true,"consumer_lag":9,"consumer_lag_stored":8},"#,
            r#""0":{"partition":0,"desired":true,"consumer_lag":5,"consumer_lag_stored":3}"#,
        )));
        assert_eq!(partitions, [(0, 3), (2, 8)]);
    }

    #[test]
    fn a_full_librdkafka_report_parses() {
        let snapshot = LagSnapshot::new("greetings", vec![]);
        snapshot.update(FIXTURE.as_bytes());
        assert_eq!(snapshot.partitions(), [(0, 5), (1, 0)]);
    }

    #[test]
    fn an_unparsable_report_clears_the_snapshot() {
        let snapshot = LagSnapshot::new("orders", vec![]);
        snapshot.update(&stats(
            r#""0":{"partition":0,"desired":true,"consumer_lag":5,"consumer_lag_stored":3}"#,
        ));
        assert_eq!(snapshot.partitions(), [(0, 3)]);
        snapshot.update(b"not json");
        assert_eq!(snapshot.partitions(), []);
    }
}
