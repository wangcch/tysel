# Go echo component

This fixture implements `tysel:component/task@0.4.0` with the bounded Tysel Go
guest SDK. The generated `wit_exports.go` is committed so normal builds do not
depend on regenerating bindings.

Generate bindings and build with the pinned `componentize-go` release:

```sh
go install github.com/bytecodealliance/componentize-go/cmd/componentize-go@v0.4.1
componentize-go -d ../../../wit/component -w task bindings
componentize-go -d ../../../wit/component -w task build -o echo.component.wasm
```
