// Package component provides guest-side helpers for
// tysel:component/task@0.4.0. Generated Component Model bindings call Dispatch
// from their exported run function.
package component

import (
	"encoding/json"
	"errors"
)

const (
	ABIVersion     = "0.4.0"
	MaxInputBytes  = 1024 * 1024
	MaxOutputBytes = 1024 * 1024
	MaxErrorBytes  = 4 * 1024
)

type Handler func(json.RawMessage) (any, error)

func Dispatch(input string, handler Handler) (string, error) {
	if len(input) > MaxInputBytes {
		return "", errors.New("component input exceeds 1048576 bytes")
	}
	if !json.Valid([]byte(input)) {
		return "", errors.New("component input is not valid JSON")
	}
	output, err := handler(json.RawMessage(input))
	if err != nil {
		return "", boundedError(err)
	}
	encoded, err := json.Marshal(output)
	if err != nil {
		return "", boundedError(err)
	}
	if len(encoded) > MaxOutputBytes {
		return "", errors.New("component output exceeds 1048576 bytes")
	}
	return string(encoded), nil
}

func boundedError(err error) error {
	message := err.Error()
	if len(message) <= MaxErrorBytes {
		return errors.New(message)
	}
	end := MaxErrorBytes
	for end > 0 && (message[end]&0xc0) == 0x80 {
		end--
	}
	return errors.New(message[:end])
}
