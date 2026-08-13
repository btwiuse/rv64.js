//go:build js && wasm

package main

import (
	"encoding/binary"
	"fmt"
	"io"
	"log"
	"os"
	"path"
	"syscall/js"
)

const defaultRelayURL = "wss://rv64-http-relay.darren-e4d.workers.dev/relay"

func main() {
	cfg, err := parseFlags(os.Args[1:])
	if err != nil {
		log.Fatal(err)
	}
	sys := js.Global().Get("sys")
	writeHost(sys, "[rv64 adapter] loading runtime assets\n")
	assetRoot := path.Dir(os.Args[0])
	loader, err := await(sys.Call("readFile", path.Join(assetRoot, "rv64.js")))
	if err != nil {
		log.Fatal(err)
	}
	wasm, err := await(sys.Call("readFile", path.Join(assetRoot, "rv64_wasm.wasm")))
	if err != nil {
		log.Fatal(err)
	}
	kernel, err := await(sys.Call("readFile", "boot/Image"))
	if err != nil {
		log.Fatal(err)
	}

	module := importLibrary(loader)
	p9 := newP9Handler()
	exportPort, exportOutput := newExportChannel()
	relayURL := os.Getenv("RV64_RELAY_URL")
	if relayURL == "" {
		relayURL = defaultRelayURL
	}
	consoleOutput := js.Global().Call(
		"eval",
		"(sys) => (bytes) => { void sys.write(1, bytes); }",
	).Invoke(sys)

	events := map[string]any{
		// A pure JavaScript callback avoids re-entering the Go Wasm runtime
		// from the emulator's host_write import while generated code is live.
		"console": consoleOutput,
		"export": js.FuncOf(func(_ js.Value, args []js.Value) any {
			exportOutput(args[0])
			return nil
		}),
		"error": js.FuncOf(func(_ js.Value, args []js.Value) any {
			writeHost(sys, fmt.Sprintf("[rv64 adapter] VM error: %s\n", args[0].Call("toString").String()))
			return nil
		}),
		"stop": js.FuncOf(func(_ js.Value, args []js.Value) any {
			writeHost(sys, fmt.Sprintf("[rv64 adapter] VM stopped: %s\n", args[0].Get("reason").String()))
			return nil
		}),
	}
	opts := map[string]any{
		"wasm":      wasm,
		"memoryMB":  cfg.memoryMB,
		"execution": map[string]any{"mode": "local"},
		"network":   map[string]any{"mode": "fetch", "relayURL": relayURL},
		"boot": map[string]any{
			"mode": "linux-direct", "kernel": kernel, "cmdline": cfg.cmdline,
			"p9":            map[string]any{"tag": "host9p", "handle": p9},
			"virtioConsole": true,
		},
		"events": events,
	}
	vm, err := await(module.Get("RV64").Call("create", opts))
	if err != nil {
		log.Fatal(err)
	}
	if cfg.jitDisabled {
		if vm.Get("setJitEnabled").Type() != js.TypeFunction {
			log.Fatal("rv64.jit=off requested, but this runtime cannot disable its JIT")
		}
		vm.Call("setJitEnabled", false)
		writeHost(sys, "[rv64 adapter] JIT disabled; running interpreter only\n")
	}
	// The comparison harness attaches to this dedicated Worker through CDP.
	// Keep the VM private in ordinary embeddings; benchmark=1 explicitly opts
	// into read-only counter observation without periodic logging or timer noise.
	if cfg.benchmark {
		js.Global().Set("__rv64BenchmarkVM", vm)
	}
	writeHost(sys, "[rv64 adapter] emulator created; starting scheduler\n")
	exportPort.Set("onmessage", js.FuncOf(func(_ js.Value, args []js.Value) any {
		vm.Get("export").Call("send", args[0].Get("data"))
		return nil
	}))
	if _, err := await(vm.Call("start")); err != nil {
		log.Fatal(err)
	}
	writeHost(sys, "[rv64 adapter] scheduler running; guest console is ttyS0\n")
	js.Global().Get("self").Call("postMessage", map[string]any{
		"vm": os.Getenv("vm"), "export": exportPort,
	}, []any{exportPort})

	go copyStdin(vm)
	select {}
}

func writeHost(sys js.Value, text string) {
	buf := js.Global().Get("Uint8Array").New(len(text))
	js.CopyBytesToJS(buf, []byte(text))
	sys.Call("write", 1, buf)
}

func importLibrary(source js.Value) js.Value {
	blob := js.Global().Get("Blob").New([]any{source}, map[string]any{"type": "application/javascript"})
	url := js.Global().Get("URL").Call("createObjectURL", blob)
	module, err := await(js.Global().Call("eval", "(url)=>import(url)").Invoke(url))
	if err != nil {
		log.Fatal(err)
	}
	return module
}

func await(promise js.Value) (js.Value, error) {
	done := make(chan struct{})
	var value js.Value
	var awaitErr error
	resolve := js.FuncOf(func(_ js.Value, args []js.Value) any {
		value = args[0]
		close(done)
		return nil
	})
	reject := js.FuncOf(func(_ js.Value, args []js.Value) any {
		awaitErr = fmt.Errorf("%s", args[0].Call("toString").String())
		close(done)
		return nil
	})
	promise.Call("then", resolve).Call("catch", reject)
	<-done
	resolve.Release()
	reject.Release()
	return value, awaitErr
}

func newP9Handler() js.Value {
	// Keep the hot request/reply bridge in JavaScript. The previous js.FuncOf
	// implementation entered Go-Wasm for the request, again for the Promise
	// executor, and once more for the reply. A cache=none 9P read/write sends
	// roughly 2,000 four-KiB messages in the comparison workload, making those
	// crossings material even though the actual filesystem server is unchanged.
	// WANIX adapts those messages back into a stream-oriented Go 9P server.
	// Keep that endpoint single-flight: its filesystem/path-lock layer can
	// deadlock when a metadata mutation overlaps another request. The emulator's
	// generic external-9P API remains capable of multiplexing other handlers.
	return js.Global().Call("eval", p9HandlerSource).
		Invoke(js.Global().Get("worker").Get("p9"))
}

func newExportChannel() (js.Value, func(js.Value)) {
	channel := js.Global().Get("MessageChannel").New()
	port1, port2 := channel.Get("port1"), channel.Get("port2")
	buf := make([]byte, 0, 4096)
	signaled := false
	return port2, func(chunk js.Value) {
		bytes := make([]byte, chunk.Get("byteLength").Int())
		js.CopyBytesToGo(bytes, chunk)
		if !signaled && len(bytes) > 0 {
			signaled = true
			port1.Call("postMessage", bytes[0])
			bytes = bytes[1:]
		}
		buf = append(buf, bytes...)
		for len(buf) >= 4 {
			n := int(binary.LittleEndian.Uint32(buf[:4]))
			if n < 4 || len(buf) < n {
				break
			}
			out := js.Global().Get("Uint8Array").New(n)
			js.CopyBytesToJS(out, buf[:n])
			port1.Call("postMessage", out)
			buf = buf[n:]
		}
	}
}

func copyStdin(vm js.Value) {
	buf := make([]byte, 4096)
	for {
		n, err := os.Stdin.Read(buf)
		if n > 0 {
			out := js.Global().Get("Uint8Array").New(n)
			js.CopyBytesToJS(out, buf[:n])
			vm.Get("console").Call("send", out)
		}
		if err != nil {
			if err != io.EOF {
				log.Println(err)
			}
			return
		}
	}
}
