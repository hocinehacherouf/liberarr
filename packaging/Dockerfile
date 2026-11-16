# The image of `liberarr serve`, built by the release workflow around the released Linux binary:
#   docker build -f packaging/Dockerfile <folder holding the liberarr binary>
# Wolfi packages only, installed into an empty root (the way apko builds Chainguard images):
# glibc, CA certificates, libdbus (linked by the Bluetooth stack) and tzdata (for `TZ`). No shell,
# no package manager.
# Renovate updates the digest.
FROM cgr.dev/chainguard/wolfi-base:latest@sha256:24f549a7fcace88e69b9557941168c74645688ecc4acf7fb6d734a6b74e9019d AS root

# --no-scripts: the empty root has no shell to run them (only an ldconfig cache, not needed).
# The apk database stays, so image scanners can list the packages.
# /data is created owned by the user, so a new named volume gets that owner.
RUN apk add --no-cache --no-scripts --initdb --root /out \
        --keys-dir /etc/apk/keys --repositories-file /etc/apk/repositories \
        wolfi-baselayout ca-certificates-bundle dbus-libs tzdata \
    && echo 'liberarr:x:1000:1000:liberarr:/data:/sbin/nologin' >> /out/etc/passwd \
    && echo 'liberarr:x:1000:' >> /out/etc/group \
    && install -d -m 700 -o 1000 -g 1000 /out/data

FROM scratch
COPY --from=root /out /
COPY --chmod=755 liberarr /usr/local/bin/liberarr

ENV LIBERARR_HOME=/data
VOLUME /data
USER 1000:1000
EXPOSE 8484
ENTRYPOINT ["/usr/local/bin/liberarr"]
# All interfaces of the container: the port is reached through Docker's port publishing.
CMD ["serve", "--host", "0.0.0.0"]
