//go:build js && wasm

package main

import (
	"flag"
	"fmt"
	"os"
	"regexp"
	"strconv"
	"strings"
)

type config struct {
	memoryMB int
	cmdline  string
}

func parseFlags(args []string) (config, error) {
	var mem, appendArgs, export string
	f := flag.NewFlagSet("rv64", flag.ContinueOnError)
	f.StringVar(&mem, "m", "512M", "memory size")
	f.StringVar(&mem, "mem", "512M", "memory size")
	f.StringVar(&appendArgs, "append", "", "kernel command line additions")
	f.StringVar(&export, "export", "", "guest device used for host export")
	if err := f.Parse(args); err != nil {
		return config{}, err
	}
	if value := os.Getenv("VM_APPEND"); value != "" {
		appendArgs = value
	}
	bytes, err := parseMemorySize(mem)
	if err != nil {
		return config{}, err
	}
	cmdline := "console=ttyS0 loglevel=3 init=/bin/init rw root=host9p rootfstype=9p rootflags=trans=virtio,version=9p2000.L,aname=,cache=none,msize=131072 rv64.network=fetch"
	if appendArgs != "" {
		cmdline += " " + appendArgs
	}
	if export != "" {
		// rv64 reserves ttyS0 for the interactive UART and exposes the WANIX
		// host-export stream on the secondary virtio console.
		cmdline += " export=hvc0"
	}
	return config{memoryMB: bytes / (1024 * 1024), cmdline: cmdline}, nil
}

func parseMemorySize(value string) (int, error) {
	m := regexp.MustCompile(`^(\d+)([KMGT]?)$`).FindStringSubmatch(strings.ToUpper(value))
	if m == nil {
		return 0, fmt.Errorf("invalid memory size %q", value)
	}
	n, err := strconv.Atoi(m[1])
	if err != nil {
		return 0, err
	}
	multipliers := map[string]int{"": 1024 * 1024, "K": 1024, "M": 1024 * 1024, "G": 1024 * 1024 * 1024}
	return n * multipliers[m[2]], nil
}
