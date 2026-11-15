# The image of `liberarr serve`, built by the release workflow around the released Linux binary:
#   docker build -f packaging/Dockerfile <folder holding the liberarr binary>
# Debian 13 (glibc 2.41) runs the binary built for glibc 2.35. Renovate updates the digest.
FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

# libdbus: linked by the Bluetooth stack. ca-certificates: HTTPS to the AI providers.
# /data is created owned by the user, so a new named volume gets that owner.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libdbus-1-3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --user-group --home-dir /data --no-create-home --shell /usr/sbin/nologin liberarr \
    && install -d -m 700 -o liberarr -g liberarr /data

COPY --chmod=755 liberarr /usr/local/bin/liberarr

ENV LIBERARR_HOME=/data
VOLUME /data
USER liberarr
EXPOSE 8484
ENTRYPOINT ["liberarr"]
# All interfaces of the container: the port is reached through Docker's port publishing.
CMD ["serve", "--host", "0.0.0.0"]
