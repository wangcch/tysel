//! Bounded, stateless MCP 2026-07-28 tool protocol core.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};

use serde_json::{Map, Value, json};

pub const MCP_PROTOCOL_VERSION: &str = "2026-07-28";
pub const MAX_MCP_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_MCP_TOOLS: usize = 256;
pub const MAX_MCP_NAME_BYTES: usize = 128;
pub const MAX_MCP_DESCRIPTION_BYTES: usize = 4 * 1024;
pub const MAX_MCP_ARGUMENT_BYTES: usize = 32 * 1024;
const PROTOCOL_VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const SERVER_INFO_META: &str = "io.modelcontextprotocol/serverInfo";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    /// A deliberately small SDK surface emitted as JSON Schema 2020-12. Every
    /// declared property is required in v0.3.
    pub input: BTreeMap<String, McpValueType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpValueType {
    String,
    Number,
    Integer,
    Boolean,
    Object,
    Array,
}

impl McpValueType {
    pub fn parse(source: &str) -> Result<Self, McpError> {
        match source {
            "string" => Ok(Self::String),
            "number" => Ok(Self::Number),
            "integer" => Ok(Self::Integer),
            "boolean" => Ok(Self::Boolean),
            "object" => Ok(Self::Object),
            "array" => Ok(Self::Array),
            _ => Err(McpError::InvalidTool(format!("unsupported MCP input type '{source}'"))),
        }
    }

    fn schema_type(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Object => "object",
            Self::Array => "array",
        }
    }

    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Number => value.is_number(),
            Self::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
            Self::Boolean => value.is_boolean(),
            Self::Object => value.is_object(),
            Self::Array => value.is_array(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpDispatch {
    Response(Value),
    ToolCall(McpToolCall),
    Notification,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpToolCall {
    pub id: Value,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone)]
pub struct McpServer {
    name: String,
    version: String,
    tools: BTreeMap<String, McpTool>,
}

impl McpServer {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        tools: impl IntoIterator<Item = McpTool>,
    ) -> Result<Self, McpError> {
        let name = name.into();
        let version = version.into();
        validate_text("server name", &name, MAX_MCP_NAME_BYTES)?;
        validate_text("server version", &version, MAX_MCP_NAME_BYTES)?;
        let mut registered = BTreeMap::new();
        for tool in tools {
            validate_tool(&tool)?;
            let tool_name = tool.name.clone();
            if registered.insert(tool_name.clone(), tool).is_some() {
                return Err(McpError::InvalidTool(format!("duplicate MCP tool '{tool_name}'")));
            }
            if registered.len() > MAX_MCP_TOOLS {
                return Err(McpError::TooManyTools);
            }
        }
        Ok(Self { name, version, tools: registered })
    }

    pub fn tools(&self) -> impl Iterator<Item = &McpTool> {
        self.tools.values()
    }

    pub fn handle_bytes(&self, bytes: &[u8]) -> Result<McpDispatch, McpError> {
        if bytes.len() > MAX_MCP_MESSAGE_BYTES {
            return Err(McpError::MessageTooLarge(bytes.len()));
        }
        let request: Value = match serde_json::from_slice(bytes) {
            Ok(request) => request,
            Err(_) => {
                return Ok(McpDispatch::Response(self.error(
                    Value::Null,
                    -32700,
                    "Parse error",
                    None,
                )));
            }
        };
        Ok(self.handle(request))
    }

    /// Well-formed protocol failures become JSON-RPC responses; only framing
    /// and JSON decoding failures are returned as Rust errors.
    pub fn handle(&self, request: Value) -> McpDispatch {
        let Some(object) = request.as_object() else {
            return McpDispatch::Response(self.error(Value::Null, -32600, "Invalid Request", None));
        };
        let id = object.get("id").cloned();
        let response_id = id.clone().unwrap_or(Value::Null);
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || id.as_ref().is_some_and(|id| !valid_request_id(id))
        {
            return McpDispatch::Response(self.error(response_id, -32600, "Invalid Request", None));
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return McpDispatch::Response(self.error(response_id, -32600, "Invalid Request", None));
        };
        if id.is_none() {
            return McpDispatch::Notification;
        }
        let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
        if !params.is_object() {
            return McpDispatch::Response(self.error(response_id, -32602, "Invalid params", None));
        }
        if let Err(response) = self.validate_protocol(&params, &response_id) {
            return McpDispatch::Response(response);
        }
        match method {
            "server/discover" => McpDispatch::Response(self.complete(
                response_id,
                json!({
                    "supportedVersions": [MCP_PROTOCOL_VERSION],
                    "capabilities": { "tools": {} }
                }),
            )),
            "ping" => McpDispatch::Response(self.complete(response_id, json!({}))),
            "tools/list" => self.list_tools(response_id, &params),
            "tools/call" => self.call_tool(response_id, &params),
            _ => McpDispatch::Response(self.error(response_id, -32601, "Method not found", None)),
        }
    }

    fn list_tools(&self, id: Value, params: &Value) -> McpDispatch {
        if params.get("cursor").is_some_and(|cursor| !cursor.is_null()) {
            return McpDispatch::Response(self.error(id, -32602, "Invalid cursor", None));
        }
        let tools: Vec<_> = self.tools.values().map(tool_json).collect();
        McpDispatch::Response(
            self.complete(id, json!({ "tools": tools, "ttlMs": 60_000, "cacheScope": "global" })),
        )
    }

    fn validate_protocol(&self, params: &Value, id: &Value) -> Result<(), Value> {
        let version = params
            .get("_meta")
            .and_then(Value::as_object)
            .and_then(|meta| meta.get(PROTOCOL_VERSION_META))
            .and_then(Value::as_str);
        if version == Some(MCP_PROTOCOL_VERSION) {
            return Ok(());
        }
        Err(self.error(
            id.clone(),
            -32022,
            "Unsupported protocol version",
            Some(json!({ "supported": [MCP_PROTOCOL_VERSION] })),
        ))
    }

    fn call_tool(&self, id: Value, params: &Value) -> McpDispatch {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return McpDispatch::Response(self.error(id, -32602, "Missing tool name", None));
        };
        let Some(tool) = self.tools.get(name) else {
            return McpDispatch::Response(self.error(
                id,
                -32602,
                &format!("Unknown tool: {name}"),
                None,
            ));
        };
        let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let argument_bytes = serde_json::to_vec(&arguments).map_or(usize::MAX, |bytes| bytes.len());
        if argument_bytes > MAX_MCP_ARGUMENT_BYTES {
            return McpDispatch::Response(self.tool_result(
                id,
                Value::String(format!("tool arguments exceed {MAX_MCP_ARGUMENT_BYTES} bytes")),
                true,
            ));
        }
        if let Err(message) = validate_arguments(tool, &arguments) {
            return McpDispatch::Response(self.tool_result(id, Value::String(message), true));
        }
        McpDispatch::ToolCall(McpToolCall { id, name: name.into(), arguments })
    }

    pub fn complete_tool_call(&self, call: McpToolCall, output: Value) -> Value {
        self.tool_result(call.id, output, false)
    }

    pub fn fail_tool_call(&self, call: McpToolCall, message: &str) -> Value {
        self.tool_result(call.id, Value::String(message.into()), true)
    }

    fn tool_result(&self, id: Value, output: Value, is_error: bool) -> Value {
        let text = match &output {
            Value::String(text) => text.clone(),
            value => serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
        };
        self.complete(
            id,
            json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": output,
                "isError": is_error
            }),
        )
    }

    fn complete(&self, id: Value, mut result: Value) -> Value {
        let result = result.as_object_mut().expect("MCP result object");
        result.insert("resultType".into(), Value::String("complete".into()));
        result.insert("_meta".into(), self.response_meta());
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    }

    fn error(&self, id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
        let mut error = json!({ "code": code, "message": message });
        if let Some(data) = data {
            error.as_object_mut().expect("error object").insert("data".into(), data);
        }
        json!({ "jsonrpc": "2.0", "id": id, "error": error })
    }

    fn response_meta(&self) -> Value {
        json!({ SERVER_INFO_META: { "name": self.name, "version": self.version } })
    }
}

fn valid_request_id(id: &Value) -> bool {
    id.is_string() || id.as_i64().is_some() || id.as_u64().is_some()
}

fn validate_tool(tool: &McpTool) -> Result<(), McpError> {
    validate_text("tool name", &tool.name, MAX_MCP_NAME_BYTES)?;
    validate_text("tool description", &tool.description, MAX_MCP_DESCRIPTION_BYTES)?;
    for name in tool.input.keys() {
        validate_text("tool input name", name, MAX_MCP_NAME_BYTES)?;
    }
    Ok(())
}

fn validate_text(label: &str, value: &str, maximum: usize) -> Result<(), McpError> {
    if value.is_empty() || value.len() > maximum || value.contains(['\n', '\r']) {
        return Err(McpError::InvalidTool(format!("{label} is invalid")));
    }
    Ok(())
}

fn tool_json(tool: &McpTool) -> Value {
    let properties: Map<_, _> = tool
        .input
        .iter()
        .map(|(name, kind)| (name.clone(), json!({ "type": kind.schema_type() })))
        .collect();
    let required: Vec<_> = tool.input.keys().cloned().collect();
    json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

fn validate_arguments(tool: &McpTool, arguments: &Value) -> Result<(), String> {
    let Some(arguments) = arguments.as_object() else {
        return Err("tool arguments must be an object".into());
    };
    let expected: BTreeSet<_> = tool.input.keys().map(String::as_str).collect();
    for name in &expected {
        let Some(value) = arguments.get(*name) else {
            return Err(format!("missing required argument '{name}'"));
        };
        if !tool.input[*name].accepts(value) {
            return Err(format!("argument '{name}' must be {}", tool.input[*name].schema_type()));
        }
    }
    if let Some(name) = arguments.keys().find(|name| !expected.contains(name.as_str())) {
        return Err(format!("unknown argument '{name}'"));
    }
    Ok(())
}

/// Read one newline-delimited stdio message without an unbounded allocation.
pub fn read_stdio_message(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, McpError> {
    let mut bytes = Vec::new();
    let mut limited = std::io::Read::take(reader, (MAX_MCP_MESSAGE_BYTES + 2) as u64);
    let read = limited.read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.last() != Some(&b'\n') && bytes.len() > MAX_MCP_MESSAGE_BYTES {
        return Err(McpError::MessageTooLarge(bytes.len()));
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    if bytes.len() > MAX_MCP_MESSAGE_BYTES {
        return Err(McpError::MessageTooLarge(bytes.len()));
    }
    Ok(Some(bytes))
}

pub fn write_stdio_message(writer: &mut impl Write, response: &Value) -> Result<(), McpError> {
    let bytes = serde_json::to_vec(response)?;
    if bytes.len() > MAX_MCP_MESSAGE_BYTES {
        return Err(McpError::MessageTooLarge(bytes.len()));
    }
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("MCP JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("MCP I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("MCP message is {0} bytes; maximum is {MAX_MCP_MESSAGE_BYTES}")]
    MessageTooLarge(usize),
    #[error("MCP server defines more than {MAX_MCP_TOOLS} tools")]
    TooManyTools,
    #[error("invalid MCP tool: {0}")]
    InvalidTool(String),
}

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    fn server() -> McpServer {
        McpServer::new(
            "tysel-test",
            "0.3.0",
            [McpTool {
                name: "analyzeCustomer".into(),
                description: "Analyze a customer".into(),
                input: BTreeMap::from([("customerId".into(), McpValueType::String)]),
            }],
        )
        .unwrap()
    }

    fn request(id: Value, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    fn modern_params(mut value: Value) -> Value {
        value
            .as_object_mut()
            .unwrap()
            .insert("_meta".into(), json!({ PROTOCOL_VERSION_META: MCP_PROTOCOL_VERSION }));
        value
    }

    #[test]
    fn discovers_stateless_server_and_lists_schema() {
        let server = server();
        let McpDispatch::Response(discover) =
            server.handle(request(json!(1), "server/discover", modern_params(json!({}))))
        else {
            panic!("response");
        };
        assert_eq!(discover["result"]["supportedVersions"][0], MCP_PROTOCOL_VERSION);
        assert_eq!(discover["result"]["resultType"], "complete");

        let McpDispatch::Response(list) =
            server.handle(request(json!(2), "tools/list", modern_params(json!({}))))
        else {
            panic!("response");
        };
        assert_eq!(list["result"]["tools"][0]["name"], "analyzeCustomer");
        assert_eq!(
            list["result"]["tools"][0]["inputSchema"]["properties"]["customerId"]["type"],
            "string"
        );
    }

    #[test]
    fn dispatches_valid_tool_call_and_completes_structured_result() {
        let server = server();
        let McpDispatch::ToolCall(call) = server.handle(request(
            json!("call-1"),
            "tools/call",
            modern_params(json!({
                "name": "analyzeCustomer",
                "arguments": { "customerId": "customer-7" }
            })),
        )) else {
            panic!("tool call");
        };
        assert_eq!(call.arguments["customerId"], "customer-7");
        let response = server.complete_tool_call(call, json!({ "risk": "low" }));
        assert_eq!(response["result"]["structuredContent"]["risk"], "low");
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn rejects_versions_unknown_tools_and_bad_arguments() {
        let server = server();
        let McpDispatch::Response(version) =
            server.handle(request(json!(1), "tools/list", json!({})))
        else {
            panic!("response");
        };
        assert_eq!(version["error"]["code"], -32022);
        let McpDispatch::Response(unknown) = server.handle(request(
            json!(2),
            "tools/call",
            modern_params(json!({ "name": "missing", "arguments": {} })),
        )) else {
            panic!("response");
        };
        assert_eq!(unknown["error"]["code"], -32602);
        let McpDispatch::Response(invalid) = server.handle(request(
            json!(3),
            "tools/call",
            modern_params(json!({
                "name": "analyzeCustomer",
                "arguments": { "customerId": 7 }
            })),
        )) else {
            panic!("response");
        };
        assert_eq!(invalid["result"]["isError"], true);
    }

    #[test]
    fn stdio_frames_are_newline_delimited_and_bounded() {
        let McpDispatch::Response(parse_error) = server().handle_bytes(b"{").unwrap() else {
            panic!("parse response");
        };
        assert_eq!(parse_error["error"]["code"], -32700);

        let mut reader = BufReader::new(Cursor::new(b"{\"jsonrpc\":\"2.0\"}\nnext\n"));
        assert_eq!(read_stdio_message(&mut reader).unwrap().unwrap(), b"{\"jsonrpc\":\"2.0\"}");
        assert_eq!(read_stdio_message(&mut reader).unwrap().unwrap(), b"next");
        assert!(read_stdio_message(&mut reader).unwrap().is_none());
        let mut output = Vec::new();
        write_stdio_message(&mut output, &json!({ "ok": true })).unwrap();
        assert_eq!(output, b"{\"ok\":true}\n");
        let oversized = vec![b'x'; MAX_MCP_MESSAGE_BYTES + 1];
        let mut reader = BufReader::new(Cursor::new(oversized));
        assert!(matches!(read_stdio_message(&mut reader), Err(McpError::MessageTooLarge(_))));
    }

    #[test]
    fn validates_registry_and_ignores_notifications() {
        assert!(McpServer::new("", "1", []).is_err());
        let duplicate =
            McpTool { name: "same".into(), description: "same".into(), input: BTreeMap::new() };
        assert!(McpServer::new("server", "1", [duplicate.clone(), duplicate]).is_err());
        assert_eq!(
            server().handle(json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": {}
            })),
            McpDispatch::Notification
        );
    }

    #[test]
    fn crate_is_named() {
        assert!(!crate_name().is_empty());
    }
}
