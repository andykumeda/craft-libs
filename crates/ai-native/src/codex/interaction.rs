//! Native app-server requests are bounded and answered only by the interactive client.
use craft_ai_core::AiError;
use serde_json::{Value, json};

pub(super) fn supported(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
            | "item/tool/requestUserInput"
            | "mcpServer/elicitation/request"
    )
}

/// Validate an explicit response against the pending native request; never broaden a grant.
pub fn validate_interaction_response(method: &str, params: &Value, response: &Value) -> Result<(), AiError> {
    let invalid = || AiError::invalid("The response does not match the pending Codex request.");
    let fields = response.as_object().ok_or_else(invalid)?;
    let valid = match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let Some(decision) = fields.get("decision") else { return Err(invalid()) };
            if params.get("availableDecisions").and_then(Value::as_array).is_some_and(|offered| !offered.contains(decision)) {
                return Err(invalid());
            }
            fields.len() == 1
                && if let Some(decision) = decision.as_str() {
                    matches!(decision, "accept" | "acceptForSession" | "decline" | "cancel")
                        && params
                            .get("availableDecisions")
                            .and_then(Value::as_array)
                            .is_none_or(|offered| offered.iter().any(|v| v.as_str() == Some(decision)))
                } else if method == "item/commandExecution/requestApproval" {
                    params
                        .get("proposedExecpolicyAmendment")
                        .is_some_and(|amendment| decision == &json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":amendment}}))
                        || params.get("proposedNetworkPolicyAmendments").and_then(Value::as_array).is_some_and(|offered| {
                            offered.iter().any(|amendment| decision == &json!({"applyNetworkPolicyAmendment":{"network_policy_amendment":amendment}}))
                        })
                } else {
                    false
                }
        }
        "item/permissions/requestApproval" => {
            fields.keys().all(|key| matches!(key.as_str(), "permissions" | "scope"))
                && fields
                    .get("permissions")
                    .is_some_and(|granted| granted.as_object().is_some_and(|v| v.is_empty()) || params.get("permissions") == Some(granted))
                && match fields.get("scope").map_or(Some("turn"), Value::as_str) {
                    Some("turn") => true,
                    Some("session") => params
                        .get("availableScopes")
                        .and_then(Value::as_array)
                        .is_some_and(|scopes| scopes.iter().any(|scope| scope.as_str() == Some("session"))),
                    _ => false,
                }
        }
        "item/tool/requestUserInput" => {
            let questions = params.get("questions").and_then(Value::as_array).ok_or_else(invalid)?;
            fields.len() == 1
                && fields.get("answers").and_then(Value::as_object).is_some_and(|answers| {
                    answers.len() == questions.len()
                        && questions.iter().all(|question| {
                            question
                                .get("id")
                                .and_then(Value::as_str)
                                .and_then(|id| answers.get(id))
                                .and_then(|answer| answer.get("answers"))
                                .and_then(Value::as_array)
                                .is_some_and(|values| !values.is_empty() && values.iter().all(Value::is_string))
                        })
                })
        }
        "mcpServer/elicitation/request" => {
            fields.keys().all(|key| matches!(key.as_str(), "action" | "content"))
                && match fields.get("action").and_then(Value::as_str) {
                    Some("decline" | "cancel") => fields.get("content").is_none_or(Value::is_null),
                    Some("accept") => {
                        params.get("mode").and_then(Value::as_str) == Some("url") || fields.get("content").is_some_and(Value::is_object)
                    }
                    _ => false,
                }
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(invalid()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn decisions_and_permission_grants_match_exact_pending_request() {
        let method = "item/commandExecution/requestApproval";
        assert!(validate_interaction_response(method, &json!({}), &json!({"decision":"decline"})).is_ok());
        assert!(validate_interaction_response(method, &json!({"availableDecisions":["decline"]}), &json!({"decision":"accept"})).is_err());
        assert!(
            validate_interaction_response(method, &json!({}), &json!({"decision":{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["sh"]}}}))
                .is_err()
        );
        let method = "item/permissions/requestApproval";
        let request = json!({"permissions":{"network":{"enabled":true}}});
        assert!(validate_interaction_response(method, &request, &json!({"permissions":{},"scope":"turn"})).is_ok());
        assert!(validate_interaction_response(method, &request, &json!({"permissions":{"network":{"enabled":true}},"scope":"turn"})).is_ok());
        assert!(validate_interaction_response(method, &request, &json!({"permissions":{"filesystem":{"write":["/"]}}})).is_err());
        assert!(validate_interaction_response(method, &request, &json!({"permissions":{},"scope":"session"})).is_err());
    }
    #[test]
    fn input_and_elicitation_responses_are_typed_and_complete() {
        let method = "item/tool/requestUserInput";
        let request = json!({"questions":[{"id":"choice"}]});
        assert!(validate_interaction_response(method, &request, &json!({"answers":{"choice":{"answers":["yes"]}}})).is_ok());
        assert!(validate_interaction_response(method, &request, &json!({"answers":{"different":{"answers":["yes"]}}})).is_err());
        let method = "mcpServer/elicitation/request";
        assert!(validate_interaction_response(method, &json!({"mode":"form"}), &json!({"action":"accept","content":{"name":"test"}})).is_ok());
        assert!(validate_interaction_response(method, &json!({"mode":"url"}), &json!({"action":"accept"})).is_ok());
        assert!(validate_interaction_response(method, &json!({}), &json!({"action":"cancel"})).is_ok());
        assert!(validate_interaction_response(method, &json!({}), &json!({"action":"accept","content":"not an object"})).is_err());
    }
}
