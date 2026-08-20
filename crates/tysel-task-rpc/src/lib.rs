//! Versioned, bounded TaskRPC messages between schedulers and task workers.
//!
//! This wire format is intentionally separate from isolated-worker IPC. Frames
//! are rejected by length before allocation, then decoded and semantically
//! validated before they are returned to a caller.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use tysel_task::TaskId;

pub const TASK_RPC_VERSION: u16 = 1;
pub const MAX_TASK_RPC_FRAME: usize = 64 * 1024;
pub const MAX_CLAIM_BATCH: u16 = 128;
pub const MAX_WORKER_ID_BYTES: usize = 128;
pub const MAX_ERROR_BYTES: usize = 4 * 1024;
pub const MAX_RESULT_BYTES: usize = 32 * 1024;
pub const MAX_LEASE_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u16,
    pub message: Message,
}

impl Envelope {
    pub fn new(message: Message) -> Self {
        Self { version: TASK_RPC_VERSION, message }
    }

    pub fn validate(&self) -> Result<(), TaskRpcError> {
        if self.version != TASK_RPC_VERSION {
            return Err(TaskRpcError::UnsupportedVersion {
                found: self.version,
                supported: TASK_RPC_VERSION,
            });
        }
        self.message.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    Hello { worker_id: String },
    Ready {},
    Claim { request_id: u64, worker_id: String, lease_ms: u64, limit: u16 },
    Claimed { request_id: u64, leases: Vec<TaskLease> },
    Renew { request_id: u64, lease: LeaseToken, lease_ms: u64 },
    Renewed { request_id: u64, lease: Option<TaskLease> },
    Release { request_id: u64, lease: LeaseToken },
    Released { request_id: u64, released: bool },
    Cancel { request_id: u64, task_id: WireTaskId },
    Commit { request_id: u64, lease: LeaseToken, outcome: TaskOutcome },
    Committed { request_id: u64, accepted: bool },
    Error { request_id: Option<u64>, code: ErrorCode, message: String },
}

impl Message {
    fn validate(&self) -> Result<(), TaskRpcError> {
        match self {
            Self::Hello { worker_id } => validate_worker_id(worker_id),
            Self::Ready {} | Self::Released { .. } | Self::Committed { .. } => Ok(()),
            Self::Claim { worker_id, lease_ms, limit, .. } => {
                validate_worker_id(worker_id)?;
                validate_lease_ms(*lease_ms)?;
                if *limit == 0 || *limit > MAX_CLAIM_BATCH {
                    return Err(TaskRpcError::InvalidClaimLimit(*limit));
                }
                Ok(())
            }
            Self::Claimed { leases, .. } => {
                if leases.len() > usize::from(MAX_CLAIM_BATCH) {
                    return Err(TaskRpcError::TooManyLeases(leases.len()));
                }
                leases.iter().try_for_each(TaskLease::validate)
            }
            Self::Renew { lease, lease_ms, .. } => {
                lease.validate()?;
                validate_lease_ms(*lease_ms)
            }
            Self::Renewed { lease, .. } => {
                if let Some(lease) = lease {
                    lease.validate()?;
                }
                Ok(())
            }
            Self::Release { lease, .. } | Self::Commit { lease, .. } => lease.validate(),
            Self::Cancel { task_id, .. } => task_id.validate(),
            Self::Error { message, .. } => validate_error(message),
        }?;
        if let Self::Commit { outcome, .. } = self {
            outcome.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WireTaskId(String);

impl WireTaskId {
    pub fn new(task_id: TaskId) -> Self {
        Self(task_id.to_string())
    }

    pub fn parse(&self) -> Result<TaskId, TaskRpcError> {
        self.validate()?;
        let value = u128::from_str_radix(&self.0, 16)
            .map_err(|_| TaskRpcError::InvalidTaskId(self.0.clone()))?;
        Ok(TaskId(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), TaskRpcError> {
        if self.0.len() != 32
            || !self.0.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(TaskRpcError::InvalidTaskId(self.0.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseToken {
    pub task_id: WireTaskId,
    pub generation: u64,
    pub lease_owner: String,
}

impl LeaseToken {
    fn validate(&self) -> Result<(), TaskRpcError> {
        self.task_id.validate()?;
        if self.generation == 0 {
            return Err(TaskRpcError::InvalidLeaseGeneration);
        }
        validate_worker_id(&self.lease_owner)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLease {
    pub token: LeaseToken,
    pub lease_until_ms: u64,
}

impl TaskLease {
    fn validate(&self) -> Result<(), TaskRpcError> {
        self.token.validate()?;
        if self.lease_until_ms == 0 {
            return Err(TaskRpcError::InvalidLeaseDeadline);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskOutcome {
    Completed { result: serde_json::Value },
    Suspended {},
    Failed { error: String, retryable: bool },
    Canceled {},
    TimedOut {},
}

impl TaskOutcome {
    fn validate(&self) -> Result<(), TaskRpcError> {
        match self {
            Self::Completed { result } => {
                let bytes = serde_json::to_vec(result)?;
                if bytes.len() > MAX_RESULT_BYTES {
                    return Err(TaskRpcError::ResultTooLarge(bytes.len()));
                }
                Ok(())
            }
            Self::Failed { error, .. } => validate_error(error),
            Self::Suspended {} | Self::Canceled {} | Self::TimedOut {} => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    VersionMismatch,
    LeaseLost,
    TaskNotFound,
    Internal,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskRpcError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("TaskRPC frame length {0} exceeds {MAX_TASK_RPC_FRAME} byte limit")]
    FrameTooLarge(usize),
    #[error("TaskRPC JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TaskRPC version {found} is unsupported; this runtime supports {supported}")]
    UnsupportedVersion { found: u16, supported: u16 },
    #[error(
        "worker id must be 1..={MAX_WORKER_ID_BYTES} ASCII letters, digits, '.', '_', ':', '@', or '-'"
    )]
    InvalidWorkerId,
    #[error("claim limit {0} must be 1..={MAX_CLAIM_BATCH}")]
    InvalidClaimLimit(u16),
    #[error("claim response contains {0} leases; maximum is {MAX_CLAIM_BATCH}")]
    TooManyLeases(usize),
    #[error("lease duration must be 1..={MAX_LEASE_MS} milliseconds")]
    InvalidLeaseDuration,
    #[error("lease deadline must be non-zero")]
    InvalidLeaseDeadline,
    #[error("lease generation must be non-zero")]
    InvalidLeaseGeneration,
    #[error("task id {0:?} is not canonical 32-character lowercase hex")]
    InvalidTaskId(String),
    #[error("task error must not exceed {MAX_ERROR_BYTES} bytes")]
    ErrorTooLarge,
    #[error("task result is {0} bytes; maximum is {MAX_RESULT_BYTES}")]
    ResultTooLarge(usize),
}

fn validate_worker_id(worker_id: &str) -> Result<(), TaskRpcError> {
    if worker_id.is_empty()
        || worker_id.len() > MAX_WORKER_ID_BYTES
        || !worker_id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'@' | b'-')
        })
    {
        return Err(TaskRpcError::InvalidWorkerId);
    }
    Ok(())
}

fn validate_lease_ms(lease_ms: u64) -> Result<(), TaskRpcError> {
    if lease_ms == 0 || lease_ms > MAX_LEASE_MS {
        return Err(TaskRpcError::InvalidLeaseDuration);
    }
    Ok(())
}

fn validate_error(error: &str) -> Result<(), TaskRpcError> {
    if error.len() > MAX_ERROR_BYTES {
        return Err(TaskRpcError::ErrorTooLarge);
    }
    Ok(())
}

pub fn write_message(writer: &mut impl Write, envelope: &Envelope) -> Result<(), TaskRpcError> {
    envelope.validate()?;
    let bytes = serde_json::to_vec(envelope)?;
    if bytes.len() > MAX_TASK_RPC_FRAME {
        return Err(TaskRpcError::FrameTooLarge(bytes.len()));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

pub fn read_message(reader: &mut impl Read) -> Result<Envelope, TaskRpcError> {
    let mut length = [0; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_TASK_RPC_FRAME {
        return Err(TaskRpcError::FrameTooLarge(length));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    let envelope: Envelope = serde_json::from_slice(&bytes)?;
    envelope.validate()?;
    Ok(envelope)
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn token() -> LeaseToken {
        LeaseToken {
            task_id: WireTaskId::new(TaskId(42)),
            generation: 7,
            lease_owner: "worker-a".into(),
        }
    }

    #[test]
    fn claim_roundtrips_with_version() {
        let expected = Envelope::new(Message::Claim {
            request_id: 9,
            worker_id: "worker-a".into(),
            lease_ms: 5_000,
            limit: 16,
        });
        let mut bytes = Vec::new();
        write_message(&mut bytes, &expected).unwrap();
        assert_eq!(read_message(&mut Cursor::new(bytes)).unwrap(), expected);
    }

    #[test]
    fn commit_roundtrips_with_generation_fence() {
        let expected = Envelope::new(Message::Commit {
            request_id: 10,
            lease: token(),
            outcome: TaskOutcome::Completed { result: serde_json::json!({"ok": true}) },
        });
        let mut bytes = Vec::new();
        write_message(&mut bytes, &expected).unwrap();
        assert_eq!(read_message(&mut Cursor::new(bytes)).unwrap(), expected);
    }

    #[test]
    fn rejects_oversize_frame_before_payload() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&((MAX_TASK_RPC_FRAME as u32) + 1).to_le_bytes());
        bytes.extend_from_slice(&[0; 8]);
        assert!(matches!(
            read_message(&mut Cursor::new(bytes)),
            Err(TaskRpcError::FrameTooLarge(length)) if length == MAX_TASK_RPC_FRAME + 1
        ));
    }

    #[test]
    fn rejects_unknown_version_and_invalid_claim() {
        let version = Envelope { version: TASK_RPC_VERSION + 1, message: Message::Ready {} };
        assert!(matches!(version.validate(), Err(TaskRpcError::UnsupportedVersion { .. })));
        let invalid = Envelope::new(Message::Claim {
            request_id: 1,
            worker_id: String::new(),
            lease_ms: 0,
            limit: 0,
        });
        assert!(matches!(invalid.validate(), Err(TaskRpcError::InvalidWorkerId)));
        let invalid = Envelope::new(Message::Hello { worker_id: "worker\nspoofed".into() });
        assert!(matches!(invalid.validate(), Err(TaskRpcError::InvalidWorkerId)));
        let invalid = Envelope::new(Message::Release {
            request_id: 2,
            lease: LeaseToken { generation: 0, ..token() },
        });
        assert!(matches!(invalid.validate(), Err(TaskRpcError::InvalidLeaseGeneration)));
    }

    #[test]
    fn rejects_unknown_message_fields() {
        let payload = br#"{"version":1,"message":{"type":"ready","unexpected":true}}"#;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        assert!(matches!(read_message(&mut Cursor::new(bytes)), Err(TaskRpcError::Json(_))));
    }

    #[test]
    fn task_ids_are_canonical_and_parse_without_precision_loss() {
        let id = TaskId(u128::MAX - 1);
        let wire = WireTaskId::new(id);
        assert_eq!(wire.as_str(), "fffffffffffffffffffffffffffffffe");
        assert_eq!(wire.parse().unwrap(), id);
        let invalid: WireTaskId =
            serde_json::from_str(r#""FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFE""#).unwrap();
        assert!(matches!(invalid.parse(), Err(TaskRpcError::InvalidTaskId(_))));
    }

    #[test]
    fn rejects_oversize_result_during_encode_and_decode() {
        let envelope = Envelope::new(Message::Commit {
            request_id: 11,
            lease: token(),
            outcome: TaskOutcome::Completed {
                result: serde_json::Value::String("x".repeat(MAX_RESULT_BYTES)),
            },
        });
        assert!(matches!(envelope.validate(), Err(TaskRpcError::ResultTooLarge(_))));
    }

    #[test]
    fn maximum_claim_batch_fits_one_frame() {
        let leases = (0..MAX_CLAIM_BATCH)
            .map(|generation| TaskLease {
                token: LeaseToken {
                    task_id: WireTaskId::new(TaskId(u128::from(generation))),
                    generation: u64::from(generation) + 1,
                    lease_owner: "w".repeat(MAX_WORKER_ID_BYTES),
                },
                lease_until_ms: u64::MAX,
            })
            .collect();
        let envelope = Envelope::new(Message::Claimed { request_id: 12, leases });
        let mut bytes = Vec::new();
        write_message(&mut bytes, &envelope).unwrap();
        assert!(bytes.len() <= MAX_TASK_RPC_FRAME + 4);
    }
}
