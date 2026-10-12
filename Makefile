# Development entry points shared by local runs and CI (`make ci`).
# See docs/development.md.

# The toolchain line in go.mod is only a minimum; exporting it as GOTOOLCHAIN
# makes every go invocation here use exactly that version (downloaded by the
# go command if missing).
GOTOOLCHAIN := $(shell sed -n 's/^toolchain //p' go.mod)
export GOTOOLCHAIN

GO          ?= go
STATICCHECK := $(GO) tool -modfile=tools/go.mod staticcheck
# gofmt from the selected toolchain, not whichever gofmt is on PATH.
GOFMT        = $(shell $(GO) env GOROOT)/bin/gofmt
GO_DIRS      = $(shell $(GO) list -f '{{.Dir}}' ./...)

.PHONY: setup update-go update-staticcheck ci fmt fmt-check lint test test-race build load

# Fetches the pinned toolchain, modules and tools so later targets work
# offline.
setup:
	$(GO) version
	$(GO) mod download
	$(GO) mod download -modfile=tools/go.mod
	$(STATICCHECK) -version

# make update-go VERSION=1.27.2
update-go:
	@test -n "$(VERSION)" || { echo "usage: make update-go VERSION=X.Y.Z"; exit 1; }
	GOTOOLCHAIN=local $(GO) mod edit -toolchain=go$(VERSION)
	GOTOOLCHAIN=local $(GO) mod edit -toolchain=go$(VERSION) tools/go.mod

# make update-staticcheck VERSION=v0.8.1 (staticcheck release 2026.2.1)
update-staticcheck:
	@test -n "$(VERSION)" || { echo "usage: make update-staticcheck VERSION=vX.Y.Z"; exit 1; }
	$(GO) get -tool -modfile=tools/go.mod honnef.co/go/tools/cmd/staticcheck@$(VERSION)

ci: fmt-check lint test-race build

fmt:
	$(GOFMT) -w $(GO_DIRS)

fmt-check:
	@out="$$($(GOFMT) -l $(GO_DIRS))"; \
	if [ -n "$$out" ]; then echo "gofmt needed:"; echo "$$out"; exit 1; fi

lint:
	$(GO) vet ./...
	$(STATICCHECK) ./...

test:
	$(GO) test ./...

test-race:
	$(GO) test -race ./...

build:
	CGO_ENABLED=0 $(GO) build ./...

# Load measurements recorded in docs/adr/0001-p0-technical-spike.md (3.5).
load:
	REPLOR_LOAD=1 $(GO) test ./internal/... -run Load -v -timeout 20m
