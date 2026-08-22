# Tysel Component SDK for Go

Guest-side JSON dispatch helpers for the experimental
`tysel:component/task@0.4.0` Wasm Component interface.

```go
func run(input string) (string, error) {
    return component.Dispatch(input, func(raw json.RawMessage) (any, error) {
        var request struct {
            Value int `json:"value"`
        }
        if err := json.Unmarshal(raw, &request); err != nil {
            return nil, err
        }
        return map[string]int{"doubled": request.Value * 2}, nil
    })
}
```

`Dispatch` validates input JSON, invokes the handler, encodes its output, and
enforces the host's 1 MiB input/output and 4 KiB error limits. Generated
Component Model bindings remain the guest application's responsibility.

See the [Go echo Component](../examples/go-echo/README.md) for binding and build
commands. The module requires the Go version declared in `go.mod`.
