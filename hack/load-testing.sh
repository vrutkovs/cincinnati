#!/usr/bin/env bash
#
# This script is used to load test Cincinnati instance.
# It uses vegeta - `go get -u github.com/tsenart/vegeta`

# PE_URL=$(oc -n cincinnati-e2e get route cincinnati-policy-engine -o jsonpath='{.spec.host}')
# export GRAPH_URL="http://${PE_URL}/api/upgrades_info/v1/graph"


TMP_DIR=$(mktemp -d)

# duration has to be larger than Prometheus collection time to ensure metrics are collected
duration=30s

# for workers in 10 50 100; do
#   for rate in 10 100 500 1000; do
    workers="10"
    rate="500"
    file="${TMP_DIR}/rate-${rate}-workers-${workers}.bin"
    echo "Testing workers ${workers}, rate ${rate} -> ${file}"
    sed "s,GRAPH_URL,${GRAPH_URL},g" vegeta.targets | \
      vegeta attack -format http -workers=${workers} -rate=${rate} -duration ${duration} > ${file}
    vegeta report -type=text ${file}
#     # Sleep here to clear up connections cache in cincinnati
#     sleep 30
#   done
# done

vegeta report -type='hist[0,50ms,100ms,500ms,1s,5s,10s]' ${TMP_DIR}/*.bin
