#!/bin/sh

set -eu

BOOTSTRAP_SERVER="${KAFKA_BOOTSTRAP_SERVER:-kafka:19092}"
READY_FILE=/tmp/topics-created

trap 'exit 0' INT TERM

for TOPIC in $KAFKA_TOPICS; do
  echo "Provisioning topic: $TOPIC"
  /opt/kafka/bin/kafka-topics.sh \
    --bootstrap-server "$BOOTSTRAP_SERVER" \
    --create --if-not-exists \
    --topic "$TOPIC" \
    --partitions "$KAFKA_TOPIC_PARTITIONS" \
    --replication-factor 1
done

touch "$READY_FILE"
echo "Provisioning complete."

sleep infinity &
wait $!
