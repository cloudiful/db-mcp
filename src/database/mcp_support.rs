use anyhow::Result;
use rmcp::model::{
    AnnotateAble, CallToolResult, Content, GetPromptResult, InitializeResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, Prompt, PromptArgument,
    PromptMessage, RawContent, RawResource, RawResourceTemplate, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate, ServerCapabilities, ServerInfo, Tool,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::Arc;

pub fn build_server_info(name: &str, version: &str) -> ServerInfo {
    InitializeResult::new(
        ServerCapabilities::builder()
            .enable_prompts()
            .enable_resources()
            .enable_tools()
            .enable_prompts_list_changed()
            .enable_resources_list_changed()
            .enable_tool_list_changed()
            .build(),
    )
    .with_server_info(
        rmcp::model::Implementation::new(name, version)
            .with_title("Database MCP")
            .with_description("Safe database discovery and read-only query access"),
    )
    .with_instructions(
        "Start with list_connections or db://connections, choose a connection name, and pass that connection to every database tool before running ad hoc SQL.".to_string(),
    )
}

pub fn text_content(text: impl Into<String>) -> Content {
    RawContent::text(text).no_annotation()
}

pub fn call_tool_text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![text_content(text)])
}

pub fn call_tool_json<T: Serialize>(text: impl Into<String>, value: &T) -> Result<CallToolResult> {
    let mut result = CallToolResult::success(vec![text_content(text)]);
    result.structured_content = Some(serde_json::to_value(value)?);
    Ok(result)
}

pub fn to_resource(
    uri: impl Into<String>,
    name: impl Into<String>,
    title: impl Into<String>,
    description: impl Into<String>,
    mime_type: impl Into<String>,
) -> Resource {
    RawResource::new(uri, name)
        .with_title(title)
        .with_description(description)
        .with_mime_type(mime_type)
        .no_annotation()
}

pub fn to_resource_template(
    uri_template: impl Into<String>,
    name: impl Into<String>,
    title: impl Into<String>,
    description: impl Into<String>,
    mime_type: impl Into<String>,
) -> ResourceTemplate {
    RawResourceTemplate::new(uri_template, name)
        .with_title(title)
        .with_description(description)
        .with_mime_type(mime_type)
        .no_annotation()
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

pub fn to_json_object<T: JsonSchema>() -> Arc<Map<String, Value>> {
    let schema = schemars::schema_for!(T);
    let value = serde_json::to_value(schema).expect("schema serializes");
    Arc::new(value.as_object().cloned().expect("schema is object"))
}
