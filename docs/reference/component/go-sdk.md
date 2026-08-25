# Go Component SDK

Status: **experimental and versioned**. Use the guest SDK source and WIT
package vendored in the Component starter bundle for the installed Tysel
release. The starter module already uses a local replacement:

```text title="go.mod"
require github.com/wangcch/tysel/sdk/component-go v0.0.0

replace github.com/wangcch/tysel/sdk/component-go => ./sdk/component-go
```

The SDK is not currently published as an independent Go module release.

## Guest interface

```go
type Handler func(json.RawMessage) (any, error)

func Dispatch(input string, handler Handler) (string, error)
```

`Dispatch` checks input JSON, invokes the handler, encodes its result, and
enforces 1 MiB input, 1 MiB output, and 4 KiB UTF-8-safe error bounds.

## Generated binding adapter

Component Model bindings remain the guest application's responsibility. The
release starter uses Bytecode Alliance `componentize-go` and adapts its
generated result type:

```go
package export_wit_world

import (
    "encoding/json"

    component "github.com/wangcch/tysel/sdk/component-go"
    witTypes "go.bytecodealliance.org/pkg/wit/types"
)

func Run(input string) witTypes.Result[string, string] {
    output, err := component.Dispatch(input, func(input json.RawMessage) (any, error) {
        return input, nil
    })
    if err != nil {
        return witTypes.Err[string, string](err.Error())
    }
    return witTypes.Ok[string, string](output)
}
```

The starter was generated with `componentize-go` `0.4.1`; generated
`wit_exports.go` is included so ordinary builds do not regenerate bindings.

Follow [Build a Go Component](../../guides/wasm-component-go.md) for the tested
workflow and [Component capabilities](capabilities.md) before adding imports.
