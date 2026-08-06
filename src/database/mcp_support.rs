use anyhow::Result;
use rmcp::handler::server::tool::{schema_for_input, schema_for_output};
use rmcp::model::{
    CallToolResult, ContentBlock, GetPromptResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, Prompt, PromptArgument, PromptMessage,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, ServerCapabilities,
    ServerInfo, Tool,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::Arc;

/// Formats that JSON Schema draft 2020-12 defines, plus numeric formats widely
/// recognized by validators (ajv and friends). Anything outside this list is
/// stripped from generated schemas: schemars emits Rust-specific formats such
/// as `uint` for `usize`, which strict clients reject with "unknown format"
/// warnings.
const KNOWN_FORMATS: &[&str] = &[
    "date-time",
    "date",
    "time",
    "duration",
    "email",
    "idn-email",
    "hostname",
    "idn-hostname",
    "ipv4",
    "ipv6",
    "uri",
    "uri-reference",
    "iri",
    "iri-reference",
    "uuid",
    "uri-template",
    "json-pointer",
    "relative-json-pointer",
    "regex",
    "int32",
    "int64",
    "float",
    "double",
    "byte",
    "password",
    "binary",
    "url",
];

/// Recursively remove `format` keywords whose value is not a known format.
/// `format` is only a validation hint, so dropping unknown values loses no
/// usable contract information.
fn strip_unknown_formats(map: &mut Map<String, Value>) {
    let format = map.get("format").and_then(Value::as_str).map(str::to_owned);
    if let Some(format) = format
        && !KNOWN_FORMATS.contains(&format.as_str())
    {
        map.remove("format");
    }
    for value in map.values_mut() {
        match value {
            Value::Object(child) => strip_unknown_formats(child),
            Value::Array(items) => {
                for item in items {
                    if let Value::Object(child) = item {
                        strip_unknown_formats(child);
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn build_server_info(name: &str, version: &str) -> ServerInfo {
    ServerInfo::new(
        ServerCapabilities::builder()
            .enable_prompts()
            .enable_resources()
            .enable_tools()
            .enable_prompts_list_changed()
            .enable_resources_list_changed()
            .enable_tool_list_changed()
            .build(),
    )
    .with_server_info(rmcp::model::Implementation::new(name, version))
    .with_instructions(
        "Start with list_connections or db://connections, choose a connection name, and pass that connection to every database tool before running ad hoc SQL.".to_string(),
    )
}

pub fn text_content(text: impl Into<String>) -> ContentBlock {
    ContentBlock::text(text)
}

pub fn call_tool_text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![text_content(text)])
}

pub fn call_tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![text_content(message)])
}

/// Structured success result. The serialized JSON is emitted as both the
/// visible text content and `structured_content`, so clients with and without
/// structured-content support receive the actual payload.
pub fn call_tool_json<T: Serialize>(value: &T) -> Result<CallToolResult> {
    Ok(CallToolResult::structured(serde_json::to_value(value)?))
}

/// Structured tool-level error result carrying the serialized error payload.
pub fn call_tool_error_json(value: Value) -> CallToolResult {
    CallToolResult::structured_error(value)
}

pub fn to_resource(
    uri: impl Into<String>,
    name: impl Into<String>,
    title: impl Into<String>,
    description: impl Into<String>,
    mime_type: impl Into<String>,
) -> Resource {
    Resource::new(uri, name)
        .with_title(title)
        .with_description(description)
        .with_mime_type(mime_type)
}

pub fn to_resource_template(
    uri_template: impl Into<String>,
    name: impl Into<String>,
    title: impl Into<String>,
    description: impl Into<String>,
    mime_type: impl Into<String>,
) -> ResourceTemplate {
    ResourceTemplate::new(uri_template, name)
        .with_title(title)
        .with_description(description)
        .with_mime_type(mime_type)
}

pub fn prompt(name: &str, description: &str) -> Prompt {
    Prompt::new(name, Some(description), None)
}

pub fn prompt_with_args(name: &str, description: &str, args: Vec<PromptArgument>) -> Prompt {
    Prompt::new(name, Some(description), Some(args))
}

pub fn get_prompt_result(
    description: impl Into<String>,
    messages: Vec<PromptMessage>,
) -> GetPromptResult {
    GetPromptResult::new(messages).with_description(description)
}

pub fn read_resource_result(contents: Vec<ResourceContents>) -> ReadResourceResult {
    ReadResourceResult::new(contents)
}

pub fn list_tools_result(tools: Vec<Tool>) -> ListToolsResult {
    ListToolsResult {
        tools,
        ..Default::default()
    }
}

pub fn list_prompts_result(prompts: Vec<Prompt>) -> ListPromptsResult {
    ListPromptsResult {
        prompts,
        ..Default::default()
    }
}

pub fn list_resources_result(resources: Vec<Resource>) -> ListResourcesResult {
    ListResourcesResult {
        resources,
        ..Default::default()
    }
}

pub fn list_resource_templates_result(
    resource_templates: Vec<ResourceTemplate>,
) -> ListResourceTemplatesResult {
    ListResourceTemplatesResult {
        resource_templates,
        ..Default::default()
    }
}

pub fn to_json_object<T: schemars::JsonSchema + std::any::Any>() -> Arc<Map<String, Value>> {
    let mut schema = schema_for_input::<T>().expect("input schema is a valid object");
    strip_unknown_formats(Arc::make_mut(&mut schema));
    schema
}

/// Build a tool from input and output types. The input schema is validated and
/// the output schema is populated from the `JsonSchema` implementation, so
/// clients receive both contracts. Unknown `format` values are stripped so
/// strict schema validators do not warn about Rust-specific formats such as
/// `uint`.
pub fn tool_schema<I, O>(
    name: &'static str,
    description: &'static str,
    annotations: rmcp::model::ToolAnnotations,
) -> Tool
where
    I: schemars::JsonSchema + std::any::Any,
    O: schemars::JsonSchema + std::any::Any,
{
    let mut input_schema = schema_for_input::<I>().expect("input schema is a valid object");
    strip_unknown_formats(Arc::make_mut(&mut input_schema));
    let mut output_schema = schema_for_output::<O>();
    strip_unknown_formats(Arc::make_mut(&mut output_schema));
    Tool::new(name, description, input_schema)
        .with_raw_output_schema(output_schema)
        .with_annotations(annotations)
}

/// Annotations shared by read-only, idempotent metadata and diagnostics tools.
pub fn read_only_annotations() -> rmcp::model::ToolAnnotations {
    rmcp::model::ToolAnnotations::new()
        .read_only(true)
        .idempotent(true)
        .open_world(false)
}

/// Annotations for query execution tools. Not read-only because writable
/// connections are supported; potentially destructive and not idempotent.
pub fn query_annotations() -> rmcp::model::ToolAnnotations {
    rmcp::model::ToolAnnotations::new()
        .read_only(false)
        .destructive(true)
        .idempotent(false)
        .open_world(false)
}
