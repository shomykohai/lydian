FROM docker.io/rust:1.96.0-alpine3.24 AS builder
RUN apk add --no-cache alpine-sdk
WORKDIR /usr/src/lydian
COPY . .
RUN cargo build --release --workspace

FROM alpine:3.24.1
RUN apk add --no-cache sqlite ffmpeg
COPY --from=builder /usr/src/lydian/target/release/lydian /usr/local/bin/lydian
RUN mkdir -p /app/music /app/downloads /app/data

EXPOSE 5555

USER 1000

ENV LYDIAN_SERVER_HOST=0.0.0.0
ENV LYDIAN_SERVER_PORT=5555
ENV LYDIAN_SERVER_DATABASE_URL=sqlite:///app/data/lydian.db
ENV LYDIAN_SUBSONIC_MUSIC_FOLDER=/app/music
ENV LYDIAN_SUBSONIC_DOWNLOAD_FOLDER=/app/downloads
ENV LYDIAN_CONFIG=/app/data/config.toml

VOLUME ["/app/data", "/app/music", "/app/downloads"]

CMD ["lydian"]
