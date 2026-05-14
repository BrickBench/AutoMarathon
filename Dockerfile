FROM rust:1.95-slim-bookworm as builder
RUN apt-get update && apt-get install -y build-essential libssl-dev pkg-config libasound2-dev cmake npm && rm -rf /var/lib/apt/lists/*

# build automarathon
WORKDIR automarathon/
COPY Cargo.lock Cargo.lock
COPY Cargo.toml Cargo.toml
COPY test.db test.db
COPY src/ src/
COPY web/ web/

RUN DATABASE_URL=sqlite://test.db cargo install -j 4 --path .

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates streamlink libasound2 nodejs && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/automarathon /usr/local/bin/automarathon

EXPOSE 28010/tcp
EXPOSE 28010/udp

CMD automarathon /data
