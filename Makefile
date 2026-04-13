.PHONY: build test \
        test-integration-apt test-integration-dnf \
        test-integration-github test-integration-url test-integration-shell \
        test-integration

build:
	cargo build --release

# Run all unit tests (no Docker required).
test:
	cargo test

# Run APT integration tests inside an Ubuntu/Debian container.
# Requires Docker. Runs as root so apt-get works without sudo.
test-integration-apt:
	docker build \
		-f tests/docker/Dockerfile.test-apt \
		-t grip-test-apt \
		.
	docker run --rm grip-test-apt

# Run DNF integration tests inside a Fedora container.
# Requires Docker. Runs as root so dnf works without sudo.
test-integration-dnf:
	docker build \
		-f tests/docker/Dockerfile.test-dnf \
		-t grip-test-dnf \
		.
	docker run --rm grip-test-dnf

# Run GitHub release adapter integration tests (requires outbound HTTPS).
test-integration-github:
	docker build \
		-f tests/docker/Dockerfile.test-github \
		-t grip-test-github \
		.
	docker run --rm grip-test-github

# Run URL adapter integration tests (requires outbound HTTPS).
test-integration-url:
	docker build \
		-f tests/docker/Dockerfile.test-url \
		-t grip-test-url \
		.
	docker run --rm grip-test-url

# Run Shell adapter integration tests (no network required).
test-integration-shell:
	docker build \
		-f tests/docker/Dockerfile.test-shell \
		-t grip-test-shell \
		.
	docker run --rm grip-test-shell

# Run all integration test suites sequentially.
test-integration: test-integration-apt test-integration-dnf \
                  test-integration-github test-integration-url test-integration-shell
