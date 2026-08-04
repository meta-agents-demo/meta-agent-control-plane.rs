.PHONY: check run test deep-test image

check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo test --workspace --all-targets --locked
	python3 scripts/verify_contract.py
	node --check scripts/dashboard.js

run:
	cargo run -- --auth-token "$${META_AGENT_TOKEN}" --protect-read-api

test:
	cargo test --workspace --all-targets --locked

deep-test:
	cargo test --locked --test replay_pressure_udp -- --nocapture --test-threads=1

image:
	docker build -t meta-agent-control-plane:local .
