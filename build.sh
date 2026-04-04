#!/bin/bash

cd "$(dirname "$0")"
cargo build --release && cp target/release/cos-cli ~/.cargo/bin/cos-cli
