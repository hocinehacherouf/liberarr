# The image of `liberarr serve`, built by the release workflow around the released Linux binary:
#   docker build -f packaging/Dockerfile <folder holding the liberarr binary>
# Wolfi base image (glibc). Renovate updates the digest.
FROM cgr.dev/chainguard/wolfi-base:latest@sha256:24f549a7fcace88e69b9557941168c74645688ecc4acf7fb6d734a6b74e9019d

# libdbus: linked by the Bluetooth stack. libstdc++: whisper.cpp (local transcription). tzdata: local times with `TZ`. CA certificates come with the base.
# /data is created owned by `nonroot`, so a new named volume gets that owner.
RUN apk add --no-cache dbus-libs libstdc++ tzdata \
    && install -d -m 700 -o nonroot -g nonroot /data

COPY --chmod=755 liberarr /usr/local/bin/liberarr

ENV LIBERARR_HOME=/data
VOLUME /data
# The `nonroot` user of the base image, by number so Kubernetes `runAsNonRoot` can check it.
USER 65532:65532
EXPOSE 8484
ENTRYPOINT ["/usr/local/bin/liberarr"]
# All interfaces of the container: the port is reached through Docker's port publishing.
CMD ["serve", "--host", "0.0.0.0"]
