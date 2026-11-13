# Quality gate for the client. `make check` is what to run before every commit: it runs what CI
# runs (npm ci, licences, typecheck, lint, knip, test) plus the production build, the bundle size budget and `npm audit --omit=dev`, in the CI Node
# image on a clean copy of the working tree. A test can pass on a newer local Node and fail in CI, so `check` never uses the
# local Node. There is no formatter; ESLint holds only a few bug-class rules (eslint.config.js).

SHELL := bash
.SHELLFLAGS := -euo pipefail -c
.ONESHELL:
.DEFAULT_GOAL := help

NODE_IMAGE ?= node:22.23.3-bookworm-slim

.PHONY: help install typecheck lint knip test build budget licenses audit types contract notices e2e-seed e2e quick check

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | sed -E 's/:.*## /\t/' | column -t -s $$'\t'

install: ## npm ci with the local Node
	npm ci --no-audit --no-fund

typecheck: ## tsc in every workspace (local Node)
	npm run typecheck

lint: ## ESLint: hooks, floating promises, TanStack Query rules (local Node)
	npm run lint

knip: ## Unused files, exports and dependencies, also code only tests use (local Node)
	npm run knip

test: ## vitest (local Node)
	npm test

build: ## Production build of the web app (local Node)
	npm run build

budget: ## Fail when the first-load JavaScript is over its size budget or holds a one-screen library (run after build)
	node scripts/check-bundle-size.mjs

licenses: ## Fail on a resolved npm package whose licence is outside the GPL-3.0-compatible set
	node scripts/check-licenses.mjs

audit: ## Known vulnerabilities in the packages that ship to the browser (needs the network)
	npm audit --omit=dev

types: ## Fail when packages/contracts/src/generated.ts is not what openapi.json produces (`npm run contract:types` rewrites it)
	npm run contract:types:check

contract: ## Fail when packages/contracts/openapi.json differs from the server's api/openapi.json (../server, or PENFII_OPENAPI)
	node scripts/sync-openapi.mjs --check

e2e-seed: ## Make the end-to-end test accounts in the running Compose stack (see docs/development.md)
	apps/e2e/seed.sh

e2e: ## Playwright against the running Compose stack, http://127.0.0.1:8080 or $$E2E_BASE_URL (not part of `make check`)
	npm run e2e

notices: ## Regenerate THIRD-PARTY-NOTICES.md from package-lock.json
	node scripts/check-licenses.mjs --notices > THIRD-PARTY-NOTICES.md

quick: typecheck lint knip test licenses types ## Fast checks with the local Node (needs `make install` once)

check: ## Final gate before a commit: CI Node image, clean copy, npm ci, typecheck, lint, knip, test, build, bundle budget
	WORK="$$(mktemp -d)"
	trap 'rm -rf "$$WORK"' EXIT
	git ls-files -co --exclude-standard -z | xargs -0 -I{} cp --parents {} "$$WORK" 2>/dev/null || true
	docker run --rm --user "$$(id -u):$$(id -g)" -e HOME=/tmp -v "$$WORK":/w -w /w $(NODE_IMAGE) sh -c 'npm ci --no-audit --no-fund >/dev/null && node scripts/check-licenses.mjs >/dev/null && npm run contract:types:check && npm run typecheck && npm run lint && npm run knip && npm test && npm run build && npm run budget && npm audit --omit=dev'
	echo "client check: OK"
