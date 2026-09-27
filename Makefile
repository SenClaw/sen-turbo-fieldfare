# sen-turbo-fieldfare — TurboFieldfare LLM runtime for SenClaw.
#
# Running does not need the Swift compiler. TurboFieldfareServer is a normal
# Mach-O binary; it links the Swift runtime that ships with macOS
# (`/usr/lib/swift`), the same way an app links Foundation.
#
# `package`, `install-local`, and `run-dev` copy that binary from ENGINE_DIR
# (or reuse dist/). `make engine` is the only target that calls `swift`, and
# only when you are rebuilding the engine from source.

CARGO_TARGET_DIR ?= target
PROFILE ?= release
CARGO_PROFILE_FLAG := $(if $(filter release,$(PROFILE)),--release,)
OUT_DIR := $(if $(filter release,$(PROFILE)),release,debug)

ID := sen-turbo-fieldfare
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
PLATFORM := darwin-arm64

TURBO_ROOT ?= $(abspath ../../turbo-fieldfare-senclaw)
ENGINE_CONFIG := $(if $(filter release,$(PROFILE)),release,debug)
ENGINE_DIR := $(TURBO_ROOT)/.build/$(ENGINE_CONFIG)
ENGINE_BIN := $(ENGINE_DIR)/TurboFieldfareServer

DIST := dist
PKG_NAME := $(ID)-$(VERSION)-$(PLATFORM)
PKG_DIR := $(DIST)/$(PKG_NAME)
ARCHIVE := $(DIST)/$(PKG_NAME).tar.gz

export CARGO_TARGET_DIR

.PHONY: build engine test package install-local run-dev clean

build:
	cargo build $(CARGO_PROFILE_FLAG)

# Optional. Needs the Swift toolchain. Not used by package / install / run.
engine:
	swift build -c $(ENGINE_CONFIG) --product TurboFieldfareServer --package-path "$(TURBO_ROOT)"
	swift build -c $(ENGINE_CONFIG) --product TurboFieldfareRepack --package-path "$(TURBO_ROOT)"

test:
	cargo test

package: build
	@if [ ! -x "$(ENGINE_BIN)" ]; then \
		echo "error: $(ENGINE_BIN) is missing." >&2; \
		echo "The runtime runs that binary; it does not compile Swift." >&2; \
		echo "Drop a prebuilt TurboFieldfareServer there, or run 'make engine' once on a machine that has Swift." >&2; \
		exit 1; \
	fi
	rm -rf "$(PKG_DIR)"
	mkdir -p "$(PKG_DIR)/bin"
	cp "$(CARGO_TARGET_DIR)/$(OUT_DIR)/sen-turbo-fieldfare" "$(PKG_DIR)/bin/sen-turbo-fieldfare"
	cp "$(ENGINE_BIN)" "$(PKG_DIR)/bin/TurboFieldfareServer"
	@if [ -x "$(ENGINE_DIR)/TurboFieldfareRepack" ]; then \
		cp "$(ENGINE_DIR)/TurboFieldfareRepack" "$(PKG_DIR)/bin/TurboFieldfareRepack"; \
	fi
	cp -R "$(ENGINE_DIR)"/*.bundle "$(PKG_DIR)/bin/"
	cp senclaw-runtime.json "$(PKG_DIR)/senclaw-runtime.json"
	mkdir -p $(DIST)
	tar -C $(DIST) -czf "$(ARCHIVE)" "$(PKG_NAME)"
	cd $(DIST) && shasum -a 256 "$(notdir $(ARCHIVE))" > "$(notdir $(ARCHIVE)).sha256"
	@echo "packaged $(ARCHIVE)"

install-local: package
	@if command -v senclaw >/dev/null 2>&1; then \
		senclaw runtime install-local "$(ARCHIVE)"; \
	else \
		echo "senclaw not on PATH — installing into ~/.senclaw/runtimes/$(ID)/$(VERSION)/ by hand"; \
		dest="$$HOME/.senclaw/runtimes/$(ID)/$(VERSION)"; \
		rm -rf "$$dest" && mkdir -p "$$dest"; \
		cp -R "$(PKG_DIR)/bin" "$$dest/bin"; \
		cp "$(PKG_DIR)/senclaw-runtime.json" "$$dest/senclaw-runtime.json"; \
		echo "installed to $$dest"; \
	fi

# Standalone serve. Does not call swift. Requires a completed .gturbo directory:
#   make run-dev MODEL=~/.senclaw/local-models/gemma4.gturbo
run-dev: build
	@engine="$(ENGINE_BIN)"; \
	if [ ! -x "$$engine" ]; then engine="$(PKG_DIR)/bin/TurboFieldfareServer"; fi; \
	if [ ! -x "$$engine" ]; then echo "error: TurboFieldfareServer not found (looked in $(ENGINE_DIR) and $(PKG_DIR)/bin)" >&2; exit 1; fi; \
	SEN_TURBO_FIELDARE_ENGINE="$$engine" cargo run $(CARGO_PROFILE_FLAG) -- serve --host 127.0.0.1 --port 4980 $(if $(MODEL),--model "$(MODEL)",)

clean:
	rm -rf $(DIST)
