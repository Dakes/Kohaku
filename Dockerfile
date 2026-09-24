# The Kohaku image (design §13, change foundation D25): the static binary CI built and an
# empty /data on distroless, COPY only. CI and release both build from this file.
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3

ARG TARGETARCH

# CI artifacts lose the executable bit.
COPY --chmod=0755 dist/${TARGETARCH}/kohaku /usr/local/bin/kohaku
# A new named volume at /data inherits this owner and mode.
COPY --chown=65532:65532 --chmod=0700 docker/data/ /data/

CMD ["kohaku", "serve"]
