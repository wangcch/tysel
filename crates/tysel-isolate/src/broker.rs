use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use tysel_engine::Value;
use tysel_ipc::WireValue;

use crate::supervisor::IsolateError;

pub struct Broker {
    secrets: HashMap<String, String>,
}

impl Broker {
    pub fn new(secrets: HashMap<String, String>) -> Self {
        Self { secrets }
    }

    pub fn call(&self, op: &str, args: &[WireValue]) -> Result<Value, IsolateError> {
        match op {
            "sleep" => {
                let millis = match args.first() {
                    Some(WireValue::Number { v }) => *v as u64,
                    _ => 0,
                };
                thread::sleep(Duration::from_millis(millis));
                Ok(Value::Null)
            }
            "echo" => match args.first() {
                Some(WireValue::String { v }) => Ok(Value::String(v.clone())),
                _ => Err(IsolateError::Broker("echo requires a string".into())),
            },
            "secret.ref" => {
                let name = match args.first() {
                    Some(WireValue::String { v }) => v.as_str(),
                    _ => return Err(IsolateError::Broker("secret.ref requires a name".into())),
                };
                if self.secrets.contains_key(name) {
                    Ok(Value::String(format!("secret:{name}")))
                } else {
                    Err(IsolateError::Broker(format!("unknown secret {name}")))
                }
            }
            "secret.read" => {
                Err(IsolateError::Broker("raw secrets cannot leave the broker".into()))
            }
            other => Err(IsolateError::Broker(format!("unknown capability {other}"))),
        }
    }
}
