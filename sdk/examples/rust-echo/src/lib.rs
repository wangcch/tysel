use serde::{Deserialize, Serialize};
use tysel_component_sdk::{Task, dispatch};

wit_bindgen::generate!({
    path: "../../../wit/component",
    world: "task",
});

struct EchoComponent;

#[derive(Deserialize)]
struct Input {
    value: serde_json::Value,
}

#[derive(Serialize)]
struct Output {
    value: serde_json::Value,
}

impl Task for EchoComponent {
    type Input = Input;
    type Output = Output;

    fn run(input: Self::Input) -> Result<Self::Output, String> {
        Ok(Output { value: input.value })
    }
}

struct Component;

impl Guest for Component {
    fn run(input: String) -> Result<String, String> {
        dispatch::<EchoComponent>(&input)
    }
}

export!(Component);
