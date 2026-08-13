package main

import _ "embed"

// p9HandlerSource is kept as a standalone JavaScript expression so its queue
// and failure behavior can be exercised directly by the Node test suite.
//
//go:embed p9-handler.js
var p9HandlerSource string
