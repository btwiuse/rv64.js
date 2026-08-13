RV64_DIR := ../..
ADAPTER := adapter
DIST := dist
RV64_RUST_SOURCES := $(shell find $(RV64_DIR)/crates/rv64-core $(RV64_DIR)/crates/rv64-system $(RV64_DIR)/crates/rv64-dbt $(RV64_DIR)/crates/rv64-wasm -type f -name '*.rs')
LEGACY_TAG := v0.1.0
LEGACY_RELEASE := https://github.com/ibuildthecloud/rv64.js/releases/download/$(LEGACY_TAG)
LEGACY_JS_SHA256 := 54df79c8b35cf50bcee34c4af02d7eb02b09e0439b717ee75bb830e733595b12
LEGACY_WASM_SHA256 := 001e5158b8f47f981371eae201079e6ec3eb632ab39ba1d0caf9a1ca5412b049
LEGACY_STAGE := $(DIST)/rv64-legacy
JIT_STAGE := $(DIST)/rv64-jit

build: $(DIST)/rv64.tgz
.PHONY: build

adapter: $(DIST)/rv64-vm.wasm
.PHONY: adapter

bundle: $(DIST)/rv64.tgz
.PHONY: bundle

comparison: $(DIST)/rv64-legacy.tgz $(DIST)/rv64-jit.tgz $(DIST)/wanix-linux-rv64.tgz $(DIST)/wanix-linux-x86.tgz
.PHONY: comparison

$(DIST)/rv64.tgz: $(DIST)/rv64-vm.wasm $(DIST)/rv64_wasm.wasm $(RV64_DIR)/web/rv64.js
	tar -czf $@ -C $(DIST) rv64-vm.wasm rv64_wasm.wasm -C $(abspath $(RV64_DIR)/web) rv64.js

$(DIST)/rv64-vm.wasm: $(wildcard $(ADAPTER)/*.go) $(ADAPTER)/p9-handler.js
	mkdir -p $(DIST)
	cd $(ADAPTER) && GOWORK=off GOOS=js GOARCH=wasm go build -o ../$@ .

$(DIST)/rv64_wasm.wasm: $(RV64_RUST_SOURCES) $(RV64_DIR)/Cargo.toml $(RV64_DIR)/Cargo.lock
	mkdir -p $(DIST)
	cargo build --manifest-path $(RV64_DIR)/Cargo.toml -p rv64-wasm --target wasm32-unknown-unknown --release
	cp $(RV64_DIR)/target/wasm32-unknown-unknown/release/rv64_wasm.wasm $@

$(JIT_STAGE)/rv64-jit-vm.wasm: $(DIST)/rv64-vm.wasm
	mkdir -p $(JIT_STAGE)
	cp $< $@

$(JIT_STAGE)/rv64_wasm.wasm: $(DIST)/rv64_wasm.wasm
	mkdir -p $(JIT_STAGE)
	cp $< $@

$(JIT_STAGE)/rv64.js: $(RV64_DIR)/web/rv64.js
	mkdir -p $(JIT_STAGE)
	cp $< $@

$(DIST)/rv64-jit.tgz: $(JIT_STAGE)/rv64-jit-vm.wasm $(JIT_STAGE)/rv64_wasm.wasm $(JIT_STAGE)/rv64.js
	tar -czf $(abspath $@) -C $(JIT_STAGE) rv64-jit-vm.wasm rv64_wasm.wasm rv64.js

$(LEGACY_STAGE)/rv64-legacy-vm.wasm: $(DIST)/rv64-vm.wasm
	mkdir -p $(LEGACY_STAGE)
	cp $< $@

$(LEGACY_STAGE)/rv64.js: Makefile
	mkdir -p $(LEGACY_STAGE)
	curl -fsSL $(LEGACY_RELEASE)/rv64.js -o $@.download
	printf '%s  %s\n' '$(LEGACY_JS_SHA256)' '$@.download' | sha256sum -c -
	mv $@.download $@

$(LEGACY_STAGE)/rv64_wasm.wasm: Makefile
	mkdir -p $(LEGACY_STAGE)
	curl -fsSL $(LEGACY_RELEASE)/rv64_wasm.wasm -o $@.download
	printf '%s  %s\n' '$(LEGACY_WASM_SHA256)' '$@.download' | sha256sum -c -
	mv $@.download $@

$(DIST)/rv64-legacy.tgz: $(LEGACY_STAGE)/rv64-legacy-vm.wasm $(LEGACY_STAGE)/rv64_wasm.wasm $(LEGACY_STAGE)/rv64.js
	tar -czf $(abspath $@) -C $(LEGACY_STAGE) rv64-legacy-vm.wasm rv64_wasm.wasm rv64.js

LINUX_COMMON_DEPS := build-linux-bundle.sh wanix-wexec-js.patch wanix-wexec-poll.patch wanix-wexec-signal.patch wanix-wexec-live-read.patch guest/init

$(DIST)/wanix-linux-rv64.tgz: $(LINUX_COMMON_DEPS) wanix-riscv64.patch $(RV64_DIR)/flake.nix $(RV64_DIR)/kernel/rv64-config.nix
	WANIX_GUEST_ARCH=riscv64 ./build-linux-bundle.sh $@

$(DIST)/wanix-linux-x86.tgz: $(LINUX_COMMON_DEPS) $(RV64_DIR)/flake.nix $(RV64_DIR)/kernel/x86-v86-config.nix
	WANIX_GUEST_ARCH=x86 ./build-linux-bundle.sh $@

linux-rv64: $(DIST)/wanix-linux-rv64.tgz
.PHONY: linux-rv64

linux-x86: $(DIST)/wanix-linux-x86.tgz
.PHONY: linux-x86

linux: linux-rv64 linux-x86
.PHONY: linux

clean:
	rm -rf $(DIST)
.PHONY: clean
