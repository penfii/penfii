# Quality gate for the server. `make check` is what to run before every commit; it does what CI
# does (fmt, clippy, cargo-deny, unused dependencies, notices, and `cargo test --locked` on PostgreSQL 17 with
# OpenSearch + nori). `make tools` installs the pinned cargo-deny and cargo-shear.
# Every integration test creates and drops a database of its own (`#[sqlx::test]`) on the server in
# DATABASE_URL, so they only ever run against throwaway containers (`integration`) or a server you
# point at on purpose (`test`).

SHELL := bash
.SHELLFLAGS := -euo pipefail -c
.ONESHELL:
.DEFAULT_GOAL := help

PG_IMAGE ?= postgres:17.11-bookworm
OS_IMAGE ?= penfii-opensearch:2.19.2
OS_NAME ?= penfii-os
# The test server is thrown away after the run, so it skips durability: most of a test's time is
# its database's migration, which otherwise waits on fsync.
PG_TEST_FLAGS ?= -c fsync=off -c synchronous_commit=off -c full_page_writes=off

need = @command -v $(1) >/dev/null || { echo "$(1) is not installed: make tools" >&2; exit 1; }

.PHONY: help tools fmt fmt-check lint licenses deny unused-deps notices notices-check quick test integration check opensearch run

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | sed -E 's/:.*## /\t/' | column -t -s $$'\t'

fmt: ## Format the code (rustfmt)
	cargo fmt

fmt-check: ## Fail if the code is not formatted
	cargo fmt --check

lint: ## clippy on every target, warnings are errors
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

tools: ## Install the pinned cargo-deny and cargo-shear (scripts/install-tools.sh)
	scripts/install-tools.sh

licenses: ## cargo-deny licence check: every resolved crate is in the GPL-3.0-compatible set of deny.toml
	$(call need,cargo-deny)
	cargo deny --locked check licenses

deny: ## cargo-deny: RustSec advisories, licences, banned and unknown sources (needs network for advisories)
	$(call need,cargo-deny)
	cargo deny --locked check

unused-deps: ## cargo-shear: fail on a dependency no crate uses
	$(call need,cargo-shear)
	cargo shear

notices: ## Regenerate THIRD-PARTY-NOTICES.md from Cargo.lock
	python3 -I scripts/notices.py > THIRD-PARTY-NOTICES.md

notices-check: ## Fail when THIRD-PARTY-NOTICES.md is out of date with Cargo.lock (fix: make notices)
	python3 -I scripts/notices.py | diff -u THIRD-PARTY-NOTICES.md - || { echo "THIRD-PARTY-NOTICES.md is out of date: run make notices" >&2; exit 1; }

quick: fmt-check lint licenses unused-deps notices-check ## Fast checks without containers

test: ## cargo test against DATABASE_URL and PENFII_TEST_OPENSEARCH_URL (a test database, never the dev one)
	: "$${DATABASE_URL:?set DATABASE_URL to a throwaway test database}"
	: "$${PENFII_TEST_OPENSEARCH_URL:?set PENFII_TEST_OPENSEARCH_URL (OpenSearch with analysis-nori)}"
	case "$$DATABASE_URL" in *:5432/*) echo "refusing: port 5432 is the development database" >&2; exit 1;; esac
	cargo test --locked

integration: ## cargo test in throwaway PostgreSQL + OpenSearch containers (same as CI)
	PG="penfii-check-pg-$$$$"
	OS="penfii-check-os-$$$$"
	docker image inspect $(OS_IMAGE) >/dev/null 2>&1 || docker build -q -t $(OS_IMAGE) ../docker/opensearch >/dev/null
	docker run -d --rm --name "$$PG" -e POSTGRES_USER=penfii -e POSTGRES_PASSWORD=penfii -e POSTGRES_DB=penfii -p 127.0.0.1::5432 $(PG_IMAGE) $(PG_TEST_FLAGS) >/dev/null
	docker run -d --rm --name "$$OS" -e discovery.type=single-node -e DISABLE_SECURITY_PLUGIN=true -e DISABLE_INSTALL_DEMO_CONFIG=true -e OPENSEARCH_JAVA_OPTS="-Xms512m -Xmx512m" -p 127.0.0.1::9200 $(OS_IMAGE) >/dev/null
	trap 'docker stop "$$PG" "$$OS" >/dev/null 2>&1 || true' EXIT
	PG_PORT="$$(docker port "$$PG" 5432/tcp | head -1 | sed 's/.*://')"
	OS_PORT="$$(docker port "$$OS" 9200/tcp | head -1 | sed 's/.*://')"
	for _ in $$(seq 1 60); do docker exec "$$PG" pg_isready -U penfii >/dev/null 2>&1 && break; sleep 1; done
	for _ in $$(seq 1 90); do curl -fs "http://127.0.0.1:$$OS_PORT/_cluster/health" >/dev/null 2>&1 && break; sleep 2; done
	sleep 2
	DATABASE_URL="postgres://penfii:penfii@127.0.0.1:$$PG_PORT/penfii" PENFII_TEST_OPENSEARCH_URL="http://127.0.0.1:$$OS_PORT" cargo test --locked

opensearch: ## Start (or create once) the persistent local OpenSearch + nori on 127.0.0.1:9200
	docker image inspect $(OS_IMAGE) >/dev/null 2>&1 || docker build -q -t $(OS_IMAGE) ../docker/opensearch >/dev/null
	if docker ps -a --format '{{.Names}}' | grep -qx "$(OS_NAME)"; then
	  docker start "$(OS_NAME)" >/dev/null
	else
	  docker run -d --name "$(OS_NAME)" --restart unless-stopped -v "$(OS_NAME)":/usr/share/opensearch/data \
	    -e discovery.type=single-node -e DISABLE_SECURITY_PLUGIN=true -e DISABLE_INSTALL_DEMO_CONFIG=true \
	    -e OPENSEARCH_JAVA_OPTS="-Xms512m -Xmx512m" -p 127.0.0.1:9200:9200 $(OS_IMAGE) >/dev/null
	fi
	for _ in $$(seq 1 90); do curl -fs http://127.0.0.1:9200/_cluster/health >/dev/null 2>&1 && break; sleep 2; done
	curl -fs http://127.0.0.1:9200/_cat/plugins | grep -q analysis-nori
	echo "opensearch: $(OS_NAME) on 127.0.0.1:9200 with analysis-nori"

run: ## core-api on 127.0.0.1:8080 against the local PostgreSQL, MinIO and OpenSearch containers
	for c in penfii-pg penfii-minio $(OS_NAME); do
	  docker ps --format '{{.Names}}' | grep -qx "$$c" || { echo "$$c is not running (start it first, see docs/development.md, Run the server; OpenSearch: make opensearch)" >&2; exit 1; }
	done
	S3_ENV="$$(docker inspect penfii-minio --format '{{range .Config.Env}}{{println .}}{{end}}')"
	[ ! -f .env.local ] || { set -a; . ./.env.local; set +a; }
	export PENFII_S3_ACCESS_KEY="$$(printf '%s\n' "$$S3_ENV" | sed -n 's/^MINIO_ROOT_USER=//p')"
	export PENFII_S3_SECRET_KEY="$$(printf '%s\n' "$$S3_ENV" | sed -n 's/^MINIO_ROOT_PASSWORD=//p')"
	export PENFII_S3_ENDPOINT="$${PENFII_S3_ENDPOINT:-http://127.0.0.1:9000}" PENFII_S3_BUCKET="$${PENFII_S3_BUCKET:-penfii}"
	export PENFII_OPENSEARCH_URL="$${PENFII_OPENSEARCH_URL:-http://127.0.0.1:9200}"
	export DATABASE_URL="$${DATABASE_URL:-postgres://penfii:penfii@127.0.0.1:5432/penfii}"
	export PENFII_BIND="$${PENFII_BIND:-127.0.0.1:8080}" PENFII_COOKIE_SECURE=0 PENFII_DEV_RETURN_INVITE_TOKEN=1
	export PENFII_DEV_SEED="$${PENFII_DEV_SEED:-1}"
	export PENFII_SESSION_TTL_SECONDS=43200 PENFII_MESSAGE_BODY_MAX_BYTES=65536 PENFII_INVITE_TTL_SECONDS=604800
	export PENFII_FILE_MAX_BYTES=104857600 PENFII_MESSAGE_FILE_MAX_COUNT=10 PENFII_MESSAGE_FILE_MAX_BYTES=209715200
	exec cargo run -p core-api

check: quick deny integration ## Final gate before a commit
	@echo "server check: OK"
