ARG TARGETARCH

FROM scratch

ARG TARGETARCH
ARG VERSION=dev
ARG REVISION=unknown

LABEL org.opencontainers.image.title="Scorch" \
      org.opencontainers.image.description="Lean Scorch web search, scraping, mapping, and crawling service" \
      org.opencontainers.image.source="https://github.com/Fractal-Tess/scorch" \
      org.opencontainers.image.url="https://github.com/Fractal-Tess/scorch" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

COPY docker/${TARGETARCH}/rootfs /
COPY docker/${TARGETARCH}/scorch /usr/local/bin/scorch
COPY docker/${TARGETARCH}/scorchd /usr/local/bin/scorchd

ENV LD_LIBRARY_PATH=/lib \
    SCORCH_API_URL=http://127.0.0.1:33000 \
    SCORCH_BIND=0.0.0.0:33000 \
    SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt

EXPOSE 33000
USER 65532:65532
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/scorchd"]
