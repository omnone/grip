.PHONY: build test \
        test-integration-apt test-integration-dnf \
        test-integration-github test-integration-url \
        test-integration \
        release

build:
	cargo build --release

# Bump the version and push master. The release pipeline is triggered manually
# from GitHub Actions (workflow_dispatch) and creates the git tag itself.
# Usage: make release VERSION=0.2.0
release:
	@test -n "$(VERSION)" || (echo "error: VERSION is required  (e.g. make release VERSION=0.2.0)" && exit 1)
	@echo "$(VERSION)" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$$' || (echo "error: VERSION must be X.Y.Z" && exit 1)
	sed -i 's/^version = ".*"/version = "$(VERSION)"/' Cargo.toml
	cargo build --release 2>/dev/null  # updates Cargo.lock
	git add Cargo.toml Cargo.lock
	git diff --cached --quiet || git commit -m "chore: release v$(VERSION)"
	git push origin master

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

# Run all integration test suites sequentially.
test-integration: test-integration-apt test-integration-dnf \
                  test-integration-github test-integration-url
