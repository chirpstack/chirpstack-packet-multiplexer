# Copy binary stage
FROM --platform=$BUILDPLATFORM alpine:3.22.2 as binary

ARG TARGETPLATFORM

COPY target/x86_64-unknown-linux-musl/release/chirpstack-packet-multiplexer /usr/bin/chirpstack-packet-multiplexer-x86_64
COPY target/armv7-unknown-linux-musleabihf/release/chirpstack-packet-multiplexer /usr/bin/chirpstack-packet-multiplexer-armv7hf
COPY target/aarch64-unknown-linux-musl/release/chirpstack-packet-multiplexer /usr/bin/chirpstack-packet-multiplexer-aarch64

RUN case "$TARGETPLATFORM" in \
	"linux/amd64") \
		cp /usr/bin/chirpstack-packet-multiplexer-x86_64 /usr/bin/chirpstack-packet-multiplexer; \
		;; \
	"linux/arm/v7") \
		cp /usr/bin/chirpstack-packet-multiplexer-armv7hf /usr/bin/chirpstack-packet-multiplexer; \
		;; \
	"linux/arm64") \
		cp /usr/bin/chirpstack-packet-multiplexer-aarch64 /usr/bin/chirpstack-packet-multiplexer; \
		;; \
	esac;

# Final stage
FROM alpine:3.22.2

COPY --from=binary /usr/bin/chirpstack-packet-multiplexer /usr/bin/chirpstack-packet-multiplexer
USER nobody:nogroup
ENTRYPOINT ["/usr/bin/chirpstack-packet-multiplexer"]
