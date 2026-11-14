#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

mkdir -p .sandbox
if [[ -x "$HOME/.cargo/bin/cargo" ]]; then export PATH="$HOME/.cargo/bin:$PATH"; fi
if ! command -v rustup >/dev/null; then
  curl -fsSL --retry 2 https://sh.rustup.rs -o .sandbox/rustup-init.sh
  sh .sandbox/rustup-init.sh -y --default-toolchain 1.97.1 --profile minimal
  export PATH="$HOME/.cargo/bin:$PATH"
fi
rustup toolchain install 1.97.1 --profile minimal >/dev/null
rustup target add wasm32-unknown-unknown --toolchain 1.97.1 >/dev/null

if [[ ! -x .sandbox/Linux-x86_64/near-sandbox ]]; then
  curl -fsSL --retry 2 'https://s3-us-west-1.amazonaws.com/build.nearprotocol.com/nearcore/Linux-x86_64/2.13.4/near-sandbox.tar.gz' -o .sandbox/near-sandbox.tar.gz
  printf '%s  %s\n' '5690a4635172263cf026683f75965cac09dd916912584f12709974fed620163f' '.sandbox/near-sandbox.tar.gz' | sha256sum -c -
  tar -xzf .sandbox/near-sandbox.tar.gz -C .sandbox
fi
export NEAR_SANDBOX_BIN_PATH="$PWD/.sandbox/Linux-x86_64/near-sandbox"

# Some containers cap open files below the node's requested 65,535.
# The shim clamps its request to the existing hard limit; it never raises that limit.
hard_limit="$(ulimit -H -n)"
if [[ "$hard_limit" != unlimited && "$hard_limit" -lt 65535 ]]; then
  cc -shared -fPIC -O2 -o .sandbox/nofile.so scripts/nofile.c -ldl
  export LD_PRELOAD="$PWD/.sandbox/nofile.so${LD_PRELOAD:+:$LD_PRELOAD}"
fi

cargo +1.97.1 build --locked --release --target wasm32-unknown-unknown
cargo +1.97.1 build --locked --example agent
cargo +1.97.1 test --locked --lib --quiet
cargo +1.97.1 test --locked --test sandbox -- --nocapture 2>&1 | tee evidence/last-run.log
python3 scripts/parse-evidence.py evidence/last-run.log evidence/last-run.js
