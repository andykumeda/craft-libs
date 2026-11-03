//! Lossless discovery for MCP catalogs larger than the configured provider tool budget.
use craft_ai_core::{
    AiError,
    assistant::{ToolCall, ToolDefinition},
};
use serde_json::{Value, json};

pub(crate) const MAX_HOST_TOOLS: usize = 1024;
pub(crate) const DISCOVERY: &str = "craft_mcp";

pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition { mcp_metadata: Default::default(),
        name: DISCOVERY.into(),
        description: "Access the complete application MCP catalog. Use list to discover tools (query and cursor are optional), describe with a tool name to get its exact argument schema, then call with that name and arguments. No application tools are excluded; calls use the same MCP server and permissions as external clients.".into(),
        input_schema: json!({"type":"object","properties":{
            "operation":{"type":"string","enum":["list","describe","call"]},
            "query":{"type":"string"},"cursor":{"type":"integer","minimum":0},
            "name":{"type":"string"},"arguments":{"type":"object"}
        },"required":["operation"],"additionalProperties":false}),
    }
}

pub(crate) enum Operation {
    Result(Value),
    Call(ToolCall),
}

pub(crate) fn resolve(tools: &[ToolDefinition], call: &ToolCall) -> Result<Operation, AiError> {
    let args = call.arguments.as_object().ok_or_else(|| AiError::invalid("craft_mcp needs an argument object"))?;
    if args.keys().any(|key| !["operation", "query", "cursor", "name", "arguments"].contains(&key.as_str())) {
        return Err(AiError::invalid("unknown craft_mcp argument"));
    }
    match args.get("operation").and_then(Value::as_str) {
        Some("list") => {
            let query = match args.get("query") {
                None => "",
                Some(Value::String(query)) => query,
                _ => return Err(AiError::invalid("query must be text")),
            }
            .to_lowercase();
            let cursor = match args.get("cursor") {
                None => 0,
                Some(cursor) => {
                    cursor.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| AiError::invalid("cursor must be a nonnegative integer"))?
                }
            };
            let found: Vec<_> = tools.iter().filter(|tool| format!("{} {}", tool.name, tool.description).to_lowercase().contains(&query)).collect();
            if cursor > found.len() {
                return Err(AiError::invalid("cursor is beyond the matching catalog"));
            }
            // Protocol-sized pages keep discovery within small model context budgets.
            let end = cursor.saturating_add(32).min(found.len());
            let page: Vec<_> = found
                .iter()
                .skip(cursor)
                .take(end.saturating_sub(cursor))
                .map(|tool| {
                    json!({
                        "name":tool.name,"description":tool.description.chars().take(240).collect::<String>()
                    })
                })
                .collect();
            Ok(Operation::Result(json!({"tools":page,"total":found.len(),"next_cursor":(end<found.len()).then_some(end)})))
        }
        Some("describe") => {
            let name = args.get("name").and_then(Value::as_str).ok_or_else(|| AiError::invalid("describe needs a tool name"))?;
            let tool = tools.iter().find(|tool| tool.name == name).ok_or_else(|| AiError::invalid("unknown MCP tool; use list to discover names"))?;
            Ok(Operation::Result(tool.to_mcp()))
        }
        Some("call") => {
            let name =
                args.get("name").and_then(Value::as_str).filter(|name| !name.is_empty()).ok_or_else(|| AiError::invalid("call needs a tool name"))?;
            let arguments = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
            if !arguments.is_object() {
                return Err(AiError::invalid("tool arguments must be an object"));
            }
            // The MCP server remains authoritative for unknown names, argument validation and permissions.
            Ok(Operation::Call(ToolCall { id: call.id.clone(), name: name.into(), arguments }))
        }
        _ => Err(AiError::invalid("craft_mcp operation must be list, describe or call")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tools() -> Vec<ToolDefinition> {
        (0..181)
            .map(|i| ToolDefinition {
                mcp_metadata: Default::default(),
                name: format!("tool_{i}"),
                description: format!("Operation {i}"),
                input_schema: json!({"type":"object","properties":{"doc":{"type":"integer"}}}),
            })
            .collect()
    }
    fn call(args: Value) -> ToolCall {
        ToolCall { id: "request-1".into(), name: DISCOVERY.into(), arguments: args }
    }
    #[test]
    fn discovery_reaches_every_tool_and_returns_exact_schemas_and_arguments() {
        let tools = tools();
        let mut names = Vec::new();
        let mut cursor = 0;
        loop {
            let Operation::Result(page) = resolve(&tools, &call(json!({"operation":"list","cursor":cursor}))).unwrap() else { panic!("list result") };
            names.extend(page["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap().to_owned()));
            match page["next_cursor"].as_u64() {
                Some(next) => cursor = next,
                None => break,
            }
        }
        assert_eq!(names, tools.iter().map(|tool| tool.name.clone()).collect::<Vec<_>>());
        let Operation::Result(schema) = resolve(&tools, &call(json!({"operation":"describe","name":"tool_180"}))).unwrap() else { panic!("schema") };
        assert_eq!(schema["inputSchema"], tools.last().unwrap().input_schema);
        let Operation::Call(actual) = resolve(&tools, &call(json!({"operation":"call","name":"tool_180","arguments":{"doc":42}}))).unwrap() else {
            panic!("call")
        };
        assert_eq!((actual.id.as_str(), actual.name.as_str(), actual.arguments), ("request-1", "tool_180", json!({"doc":42})));
        assert!(resolve(&tools, &call(json!({"operation":"list","cursor":u64::MAX}))).is_err());
    }
}
