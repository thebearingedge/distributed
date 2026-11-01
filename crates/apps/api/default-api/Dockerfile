ARG APPLICATION=default-api

FROM --platform=$BUILDPLATFORM ghcr.io/thebearingedge/rust-builder:default AS rust-builder

ARG TARGETARCH
ARG APPLICATION

COPY --chown=65534:65534 . .

RUN \
  T="$(rust-target "$TARGETARCH")" \
  && cargo build --release --locked --target "$T" -p "$APPLICATION" \
  && cp "target/$T/release/$APPLICATION" "/app/$APPLICATION"


FROM gcr.io/distroless/cc-debian12

ARG APPLICATION

COPY --from=rust-builder "/app/$APPLICATION" "/$APPLICATION"

USER 65534:65534

ENTRYPOINT ["/default-api"]
