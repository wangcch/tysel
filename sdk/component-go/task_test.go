package component

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func TestDispatch(t *testing.T) {
	output, err := Dispatch(`{"value":21}`, func(input json.RawMessage) (any, error) {
		var value struct {
			Value int `json:"value"`
		}
		if err := json.Unmarshal(input, &value); err != nil {
			return nil, err
		}
		return map[string]int{"doubled": value.Value * 2}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if output != `{"doubled":42}` {
		t.Fatalf("unexpected output: %s", output)
	}
}

func TestDispatchRejectsInvalidAndOversizedInput(t *testing.T) {
	noop := func(json.RawMessage) (any, error) { return nil, nil }
	if _, err := Dispatch("invalid", noop); err == nil {
		t.Fatal("invalid JSON was accepted")
	}
	if _, err := Dispatch(strings.Repeat(" ", MaxInputBytes+1), noop); err == nil {
		t.Fatal("oversized input was accepted")
	}
}

func TestDispatchBoundsErrorsAtUTF8Boundary(t *testing.T) {
	_, err := Dispatch("null", func(json.RawMessage) (any, error) {
		return nil, errors.New(strings.Repeat("好", MaxErrorBytes))
	})
	if err == nil || len(err.Error()) > MaxErrorBytes {
		t.Fatalf("unexpected bounded error: %v", err)
	}
}
