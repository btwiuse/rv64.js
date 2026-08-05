RV64_DIR := ../..
ADAPTER := adapter
DIST := dist
RV64_RUST_SOURCES := $(shell find $(RV64_DIR)/crates/rv64-core $(RV64_DIR)/crates/rv64-system $(RV64_DIR)/crates/rv64-jit $(RV64_DIR)/crates/rv64-wasm -type f -name '*.rs')

build: $(DIST)/rv64.tgz
.PHONY: build

adapter: $(DIST)/rv64-vm.wasm
.PHONY: wasm

bundle: $(DIST)/rv64.tgz
.PHONY: bundle

$(DIST)/rv64.tgz: $(DIST)/rv64-vm.wasm $(DIST)/rv64_wasm.wasm
	tar -C $(DIST) -czf $@ rv64-vm.wasm rv64_wasm.wasm

$(ADAPTER)/lib.js: $(ADAPTER)/lib/mod.js $(RV64_DIR)/web/rv64.js
	mkdir -p $(ADAPTER)/.build
	cp $(RV64_DIR)/web/rv64.js $(ADAPTER)/.build/rv64.js
	npx --yes esbuild $(ADAPTER)/lib/mod.js --bundle --format=esm --alias:rv64.js=./$(ADAPTER)/.build/rv64.js --outfile=$@

$(DIST)/rv64-vm.wasm: $(ADAPTER)/main.go $(ADAPTER)/config.go $(ADAPTER)/lib.js
	mkdir -p $(DIST)
	cd $(ADAPTER) && GOWORK=off GOOS=js GOARCH=wasm go build -o ../$@ .

$(DIST)/rv64_wasm.wasm: $(RV64_RUST_SOURCES) $(RV64_DIR)/Cargo.toml $(RV64_DIR)/Cargo.lock
	mkdir -p $(DIST)
	cargo build --manifest-path $(RV64_DIR)/Cargo.toml -p rv64-wasm --target wasm32-unknown-unknown --release
	cp $(RV64_DIR)/target/wasm32-unknown-unknown/release/rv64_wasm.wasm $@

$(DIST)/wanix-linux-rv64.tgz: build-linux-bundle.sh wanix-riscv64.patch wanix-wexec-js.patch wanix-wexec-poll.patch wanix-wexec-signal.patch wanix-wexec-live-read.patch guest/init
	./build-linux-bundle.sh $@

linux: $(DIST)/wanix-linux-rv64.tgz
.PHONY: linux

clean:
	rm -rf $(DIST) $(ADAPTER)/.build $(ADAPTER)/lib.js
.PHONY: clean
