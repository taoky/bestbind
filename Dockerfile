FROM rust:1-bullseye AS builder
# Use the oldest glibc for supported Ubuntu/Debian versions.

COPY . /workspace
WORKDIR /workspace
RUN cd libbinder && cargo build --release
RUN cargo build --release

FROM scratch
COPY --from=builder /workspace/target/release/bestbind /bestbind
COPY --from=builder /workspace/libbinder/target/release/libbinder.so /libbinder.so
