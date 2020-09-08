FROM quay.io/app-sre/cincinnati:builder AS rust_builder
# build e2e tests
COPY . .

# Build and copy test binary
RUN cd e2e && \
    cargo build --tests --features test-e2e-prom-query --message-format=json \
    | jq -r "select(.profile.test == true) | .filenames[]" \
    | xargs -n1 -i cp -rvf {} /opt/cincinnati/bin && \
    for f in /opt/cincinnati/bin/*; do mv -- ${f} ${f%-*}; done

FROM registry.svc.ci.openshift.org/openshift/release:golang-1.13 AS golang_builder
RUN env GOBIN=/usr/local/bin go get -u github.com/tsenart/vegeta

FROM centos:7

ENV HOME="/root"

RUN mkdir -p "${HOME}/cincinnati"
WORKDIR "${HOME}/cincinnati"

# Get oc CLI
RUN mkdir -p ${HOME}/bin && \
    curl https://mirror.openshift.com/pub/openshift-v4/clients/oc/latest/linux/oc.tar.gz 2>/dev/null | tar xzf - -C "${HOME}/bin/" oc
ENV PATH="${PATH}:${HOME}/bin"

COPY --from=rust_builder /opt/cincinnati/bin/cincinnati-e2e-test /usr/bin
COPY --from=rust_builder /opt/cincinnati/bin/cincinnati-prometheus_query-test /usr/bin
COPY --from=rust_builder /opt/cincinnati/bin/cincinnati-e2e-slo /usr/bin
COPY --from=rust_builder hack/e2e.sh hack/
COPY --from=rust_builder dist/openshift/cincinnati.yaml dist/openshift/
COPY --from=rust_builder dist/openshift/observability.yaml dist/openshift/
COPY --from=golang_builder /usr/local/bin/vegeta /usr/bin
COPY --from=rust_builder hack/load-testing.sh /usr/local/bin/load-testing.sh
COPY --from=rust_builder hack/vegeta.targets vegeta.targets
COPY --from=rust_builder e2e/tests/testdata e2e/tests/testdata
ENV E2E_TESTDATA_DIR "e2e/tests/testdata"

ENTRYPOINT ["hack/e2e.sh"]
