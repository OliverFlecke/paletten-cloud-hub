FROM lukemathwalker/cargo-chef:latest-rust-1.74.1-alpine as chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
# Everything until now should be cached as long as dependency tree is the same.
COPY . .
RUN cargo build --release


FROM scratch
COPY --from=builder /app/target/release/paletten-cloud-hub app
ENTRYPOINT ["./app"]
