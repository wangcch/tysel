//! Bounded, length-prefixed IPC for isolated workers.
//!
//! Frames larger than [`MAX_FRAME`] are rejected before the payload is read.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use tysel_engine::Value;

pub const MAX_FRAME: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame length {0} exceeds {MAX_FRAME} byte limit")]
    FrameTooLarge(usize),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    WorkerReady,
    Start {
        memory_limit_bytes: usize,
        cpu_ms_per_turn: u64,
        request_timeout_ms: u64,
        rlimit_as_bytes: usize,
        #[serde(default)]
        app: String,
        #[serde(default)]
        json_logs: bool,
    },
    Started,
    Eval {
        id: u64,
        source: String,
    },
    EvalOk {
        id: u64,
        value: WireValue,
    },
    EvalErr {
        id: u64,
        error: String,
    },
    Load {
        source: String,
        secret_names: Vec<String>,
    },
    Loaded,
    LoadErr {
        error: String,
    },
    TaskLoad {
        id: u64,
        source: String,
        secret_names: Vec<String>,
    },
    TaskLoaded {
        id: u64,
        definitions_json: String,
    },
    TaskInvoke {
        id: u64,
        task_name: String,
        input_json: String,
        request_id: String,
        deadline_ms: u64,
    },
    TaskOk {
        id: u64,
        value: WireValue,
    },
    TaskErr {
        id: u64,
        error: String,
        kind: TaskErrorKind,
    },
    Http {
        id: u64,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: String,
        #[serde(default)]
        request_id: u64,
    },
    HttpOk {
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
        websocket: bool,
    },
    HttpErr {
        id: u64,
        error: String,
    },
    CapCall {
        id: u64,
        op: String,
        args: Vec<WireValue>,
    },
    CapOk {
        id: u64,
        value: WireValue,
    },
    CapErr {
        id: u64,
        error: String,
    },
    Overalloc,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskErrorKind {
    Failed,
    TimedOut,
    Canceled,
    Suspended,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum WireValue {
    Null,
    Bool { v: bool },
    Number { v: f64 },
    String { v: String },
    Bytes { v: Vec<u8> },
    Array { v: Vec<WireValue> },
    Record { v: Vec<(String, WireValue)> },
}

impl From<Value> for WireValue {
    fn from(value: Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(v) => Self::Bool { v },
            Value::Number(v) => Self::Number { v },
            Value::String(v) => Self::String { v },
            Value::Bytes(v) => Self::Bytes { v },
            Value::Array(v) => Self::Array { v: v.into_iter().map(Self::from).collect() },
            Value::Record(v) => Self::Record {
                v: v.into_iter().map(|(name, value)| (name, Self::from(value))).collect(),
            },
        }
    }
}

impl From<WireValue> for Value {
    fn from(value: WireValue) -> Self {
        match value {
            WireValue::Null => Self::Null,
            WireValue::Bool { v } => Self::Bool(v),
            WireValue::Number { v } => Self::Number(v),
            WireValue::String { v } => Self::String(v),
            WireValue::Bytes { v } => Self::Bytes(v),
            WireValue::Array { v } => Self::Array(v.into_iter().map(Self::from).collect()),
            WireValue::Record { v } => {
                Self::Record(v.into_iter().map(|(name, value)| (name, Self::from(value))).collect())
            }
        }
    }
}

pub fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> Result<(), IpcError> {
    if bytes.len() > MAX_FRAME {
        return Err(IpcError::FrameTooLarge(bytes.len()));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()?;
    Ok(())
}

pub fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, IpcError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(IpcError::FrameTooLarge(len));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn write_message(writer: &mut impl Write, message: &Message) -> Result<(), IpcError> {
    write_frame(writer, &serde_json::to_vec(message)?)
}

pub fn read_message(reader: &mut impl Read) -> Result<Message, IpcError> {
    Ok(serde_json::from_slice(&read_frame(reader)?)?)
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn roundtrip_eval_message() {
        let mut buf = Vec::new();
        write_message(&mut buf, &Message::Eval { id: 7, source: "1+1".into() }).unwrap();
        let decoded = read_message(&mut Cursor::new(buf)).unwrap();
        assert_eq!(decoded, Message::Eval { id: 7, source: "1+1".into() });
    }

    #[test]
    fn roundtrip_http_message() {
        let mut buf = Vec::new();
        write_message(
            &mut buf,
            &Message::Http {
                id: 3,
                method: "GET".into(),
                url: "http://tysel.local/".into(),
                headers: vec![("accept".into(), "*/*".into())],
                body: String::new(),
                request_id: 9,
            },
        )
        .unwrap();
        let decoded = read_message(&mut Cursor::new(buf)).unwrap();
        assert_eq!(
            decoded,
            Message::Http {
                id: 3,
                method: "GET".into(),
                url: "http://tysel.local/".into(),
                headers: vec![("accept".into(), "*/*".into())],
                body: String::new(),
                request_id: 9,
            }
        );
    }

    #[test]
    fn roundtrip_task_message_and_structured_value() {
        let message = Message::TaskOk {
            id: 11,
            value: WireValue::Record {
                v: vec![(
                    "items".into(),
                    WireValue::Array {
                        v: vec![WireValue::Number { v: 1.0 }, WireValue::Bool { v: true }],
                    },
                )],
            },
        };
        let mut buf = Vec::new();
        write_message(&mut buf, &message).unwrap();
        assert_eq!(read_message(&mut Cursor::new(buf)).unwrap(), message);
    }

    #[test]
    fn rejects_oversize_frame_before_payload() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&((MAX_FRAME as u32) + 1).to_le_bytes());
        buf.extend_from_slice(&[0u8; 8]);
        let err = read_frame(&mut Cursor::new(buf)).unwrap_err();
        assert!(matches!(err, IpcError::FrameTooLarge(len) if len == MAX_FRAME + 1));
    }
}
