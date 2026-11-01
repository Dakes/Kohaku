# The Kohaku image (design §13, change foundation D25): the static binary CI built and an
# empty /data on distroless, COPY only. CI and release both build from this file.
FROM gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329

ARG TARGETARCH

# CI artifacts lose the executable bit.
COPY --chmod=0755 dist/${TARGETARCH}/kohaku /usr/local/bin/kohaku
# A new named volume at /data inherits this owner and mode.
COPY --chown=65532:65532 --chmod=0700 docker/data/ /data/

CMD ["kohaku", "serve"]
