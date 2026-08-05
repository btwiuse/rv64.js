//go:build js && wasm

package main

import (
	"embed"
	"encoding/binary"
	"fmt"
	"io"
	"log"
	"os"
	"sync"
	"syscall/js"
)

//go:embed lib.js
var assets embed.FS

const defaultRelayURL = "wss://rv64-http-relay.darren-e4d.workers.dev/relay"

func main() {
	cfg, err := parseFlags(os.Args[1:])
	if err != nil {
		log.Fatal(err)
	}
	sys := js.Global().Get("sys")
	wasm, err := await(sys.Call("readFile", "#vm/rv64/rv64_wasm.wasm"))
	if err != nil {
		log.Fatal(err)
	}
	kernel, err := await(sys.Call("readFile", "boot/Image"))
	if err != nil {
		log.Fatal(err)
	}

	module := importLibrary()
	p9 := newP9Handler()
	exportPort, exportOutput := newExportChannel()
	relayURL := os.Getenv("RV64_RELAY_URL")
	if relayURL == "" {
		relayURL = defaultRelayURL
	}

	events := map[string]any{
		"console": js.FuncOf(func(_ js.Value, args []js.Value) any {
			bytes := make([]byte, args[0].Get("byteLength").Int())
			js.CopyBytesToGo(bytes, args[0])
			go fmt.Fprint(os.Stdout, string(bytes))
			return nil
		}),
		"export": js.FuncOf(func(_ js.Value, args []js.Value) any {
			exportOutput(args[0])
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
	exportPort.Set("onmessage", js.FuncOf(func(_ js.Value, args []js.Value) any {
		vm.Get("export").Call("send", args[0].Get("data"))
		return nil
	}))
	if _, err := await(vm.Call("start")); err != nil {
		log.Fatal(err)
	}
	js.Global().Get("self").Call("postMessage", map[string]any{
		"vm": os.Getenv("vm"), "export": exportPort,
	}, []any{exportPort})

	go copyStdin(vm)
	select {}
}

func importLibrary() js.Value {
	b, err := assets.ReadFile("lib.js")
	if err != nil {
		log.Fatal(err)
	}
	buf := js.Global().Get("Uint8Array").New(len(b))
	js.CopyBytesToJS(buf, b)
	blob := js.Global().Get("Blob").New([]any{buf}, map[string]any{"type": "application/javascript"})
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

func newP9Handler() js.Func {
	var mu sync.Mutex
	resolvers := map[uint16]js.Value{}
	js.Global().Get("worker").Get("p9").Set("onmessage", js.FuncOf(func(_ js.Value, args []js.Value) any {
		data := args[0].Get("data")
		tag := uint16(data.Index(5).Int()) | uint16(data.Index(6).Int())<<8
		mu.Lock()
		resolve := resolvers[tag]
		delete(resolvers, tag)
		mu.Unlock()
		if resolve.Truthy() {
			resolve.Invoke(data)
		}
		return nil
	}))
	return js.FuncOf(func(_ js.Value, args []js.Value) any {
		req := args[0]
		tag := uint16(req.Index(5).Int()) | uint16(req.Index(6).Int())<<8
		executor := js.FuncOf(func(_ js.Value, promiseArgs []js.Value) any {
			mu.Lock()
			resolvers[tag] = promiseArgs[0]
			mu.Unlock()
			js.Global().Get("worker").Get("p9").Call("postMessage", req)
			return nil
		})
		return js.Global().Get("Promise").New(executor)
	})
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
