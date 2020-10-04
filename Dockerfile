FROM rust:1.47

RUN apt-get update && apt-get install libssl-dev pkg-config -y

RUN mkdir /usr/local/we-rust
WORKDIR /usr/local/we-rust
