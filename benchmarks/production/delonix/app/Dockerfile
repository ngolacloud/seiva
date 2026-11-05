# Runtime image for a Catalog admin-data-e2e replica. The release binary is
# built on the host (it is a DORM-coupled excluded crate with path deps that are
# awkward to reproduce in a build context) and copied in by ../run.sh before
# `delonix compose up`. The binary is glibc-dynamic (see `ldd`), so the base is
# debian-slim, not alpine/musl.
FROM debian:stable-slim
RUN useradd --system --uid 10001 seiva
COPY admin-data-e2e /usr/local/bin/admin-data-e2e
USER seiva
EXPOSE 8098
ENTRYPOINT ["/usr/local/bin/admin-data-e2e"]
