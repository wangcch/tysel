package export_wit_world

import (
	"encoding/json"

	component "github.com/wangcch/tysel/sdk/component-go"
	witTypes "go.bytecodealliance.org/pkg/wit/types"
)

// Run implements tysel:component/task@0.4.0 and echoes valid JSON through the
// bounded guest SDK dispatcher.
func Run(input string) witTypes.Result[string, string] {
	output, err := component.Dispatch(input, func(input json.RawMessage) (any, error) {
		return input, nil
	})
	if err != nil {
		return witTypes.Err[string, string](err.Error())
	}
	return witTypes.Ok[string, string](output)
}
