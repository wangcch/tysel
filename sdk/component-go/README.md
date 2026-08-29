# Tysel Component SDK for Go

Guest-side JSON dispatch for the experimental
`tysel:component/task@0.4.0` Wasm Component interface. This module is vendored in
the version-matched Tysel Component starter bundle; generated Component Model
bindings remain the guest application's responsibility.

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

`Dispatch` validates JSON and enforces the host's 1 MiB input/output and 4 KiB
error limits. Use the [Go Component guide](https://tysel.dev/docs/guides/wasm-component-go/)
for the complete generated export and build commands.
