# syntax=docker/dockerfile:1

FROM rust:1.89 AS builder
WORKDIR /usr/src/httplive_dvr

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN apt update && apt install -y libavcodec-dev libavdevice-dev libavfilter-dev libavformat-dev libavutil-dev libclang-dev
RUN cargo fetch

# Build application
COPY src ./src
RUN cargo build --release

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y ffmpeg

COPY --from=builder /usr/src/httplive_dvr/target/release/httplive_dvr /usr/local/bin/httplive_dvr

ENV HTTPLIVE_BASE_DIR=/data
VOLUME ["/data"]
EXPOSE 3000

CMD ["httplive_dvr"]

