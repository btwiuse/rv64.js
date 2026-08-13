//go:build js && wasm

package main

import (
	"strings"
	"testing"
)

func TestRemovedStaticT0AdapterOptionIsRejected(t *testing.T) {
	if _, err := parseFlags([]string{
		"-append", "rv64.benchmark=1 rv64.static-t0=sampled-backoff",
	}); err == nil || !strings.Contains(err.Error(), "experiment was rejected") {
		t.Fatalf("parseFlags did not reject removed static-T0 option: %v", err)
	}
}

func TestBenchmarkAdapterOptionIsPreserved(t *testing.T) {
	cfg, err := parseFlags([]string{"-append", "rv64.benchmark=1 loglevel=6"})
	if err != nil {
		t.Fatal(err)
	}
	if !cfg.benchmark || !strings.Contains(cfg.cmdline, "rv64.benchmark=1 loglevel=6") {
		t.Fatalf("benchmark command line changed unexpectedly: %s", cfg.cmdline)
	}
}
