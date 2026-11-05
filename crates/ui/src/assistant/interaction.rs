//! Explicit app-server approval and input requests. No response is chosen or sent by rendering.

use super::Action;
use egui::Ui;
use serde_json::{Map, Value, json};

/// Unmodified server request, correlated by the adapter's opaque local id.
#[derive(Clone)]
pub struct Interaction {
    pub id: u64,
    pub method: String,
    pub params: Value,
}

/// Unsaved user input; never serialize or log this state because answers can contain secrets.
#[derive(Default)]
pub struct InteractionForm {
    request_id: Option<u64>,
    answers: Map<String, Value>,
    raw: String,
    error: Option<String>,
}

pub(super) fn show(ui: &mut Ui, request: &Interaction, form: &mut InteractionForm, tr: &impl Fn(&str) -> String) -> Option<Action> {
    if form.request_id != Some(request.id) {
        *form = InteractionForm { request_id: Some(request.id), ..Default::default() };
    }
    if !bounded_value(&request.params) {
        ui.label(tr("This request is too large or deeply nested to review safely. Stop the turn and ask for a smaller request."));
        return ui.button(tr("Stop turn")).clicked().then_some(Action::Stop);
    }
    let mut result = None;
    egui::ScrollArea::vertical().id_salt(("assistant_interaction", request.id)).max_height((ui.available_height() - 36.0).max(1.0)).show(ui, |ui| {
        ui.push_id(request.id, |ui| {
            ui.strong(tr("Your response is needed"));
            match request.method.as_str() {
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                    result = approval(ui, request, tr);
                }
                "item/permissions/requestApproval" => result = permissions(ui, request, tr),
                "item/tool/requestUserInput" => result = questions(ui, request, form, tr),
                "mcpServer/elicitation/request" => result = elicitation(ui, request, form, tr),
                _ => {
                    ui.label(tr("This request type is not supported. Stop the turn to cancel it."));
                }
            }
            if let Some(error) = &form.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        });
    });
    if ui.button(tr("Stop turn")).clicked() {
        result = Some(Action::Stop);
    }
    result
}

// A request must fit a finite review surface before any details are formatted or grants offered.
fn bounded_value(value: &Value) -> bool {
    let mut pending = vec![(value, 0usize)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes = nodes.saturating_add(1);
        bytes = bytes.saturating_add(32);
        if nodes > 4096 || depth > 32 || bytes > 65_536 {
            return false;
        }
        match value {
            Value::String(text) => bytes = bytes.saturating_add(text.len().saturating_mul(6)),
            Value::Array(values) => {
                if values.len().saturating_add(pending.len()) > 4096 {
                    return false;
                }
                pending.extend(values.iter().map(|v| (v, depth.saturating_add(1))));
            }
            Value::Object(values) => {
                if values.len().saturating_add(pending.len()) > 4096 {
                    return false;
                }
                for (key, value) in values {
                    bytes = bytes.saturating_add(key.len().saturating_mul(6));
                    pending.push((value, depth.saturating_add(1)));
                }
            }
            _ => {}
        }
        if bytes > 65_536 {
            return false;
        }
    }
    true
}

fn respond(request: &Interaction, response: Value) -> Action {
    Action::Respond { id: request.id, response }
}

fn detail(ui: &mut Ui, label: &str, value: Option<&Value>, tr: &impl Fn(&str) -> String) {
    if let Some(value) = value.filter(|v| !v.is_null()) {
        ui.strong(tr(label));
        ui.add(egui::Label::new(value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())).wrap());
    }
}

fn approval(ui: &mut Ui, request: &Interaction, tr: &impl Fn(&str) -> String) -> Option<Action> {
    let p = &request.params;
    let command = request.method == "item/commandExecution/requestApproval";
    ui.heading(tr(if command { "Allow command execution?" } else { "Allow file changes?" }));
    for (key, label) in [
        ("reason", "Reason"),
        ("command", "Command"),
        ("cwd", "Working directory"),
        ("commandActions", "Requested actions"),
        ("changes", "File changes"),
        ("grantRoot", "Write access"),
        ("networkApprovalContext", "Network access"),
        ("additionalPermissions", "Additional permissions"),
    ] {
        detail(ui, label, p.get(key), tr);
    }
    let mut result = None;
    ui.horizontal_wrapped(|ui| {
        for (decision, label) in approval_choices(p) {
            if ui.button(tr(label)).clicked() {
                result = Some(respond(request, json!({"decision":decision})));
            }
        }
    });
    if command {
        if let Some(amendment) = p.get("proposedExecpolicyAmendment").filter(|v| v.is_array() && decision_offered(p, "acceptWithExecpolicyAmendment"))
        {
            ui.collapsing(tr("Allow matching commands in future"), |ui| {
                detail(ui, "Command prefix", Some(amendment), tr);
                if ui.button(tr("Allow and save command rule")).clicked() {
                    result = Some(respond(request, json!({"decision":{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":amendment}}})));
                }
            });
        }
        if let Some(amendments) =
            p.get("proposedNetworkPolicyAmendments").filter(|_| decision_offered(p, "applyNetworkPolicyAmendment")).and_then(Value::as_array)
        {
            for (index, amendment) in amendments.iter().take(64).enumerate() {
                ui.push_id(("network_rule", index), |ui| {
                    ui.collapsing(tr("Save network rule"), |ui| {
                        detail(ui, "Network policy", Some(amendment), tr);
                        if ui.button(tr("Apply this network rule")).clicked() {
                            result =
                                Some(respond(request, json!({"decision":{"applyNetworkPolicyAmendment":{"network_policy_amendment":amendment}}})));
                        }
                    });
                });
            }
        }
    }
    result
}

fn decision_offered(params: &Value, name: &str) -> bool {
    params
        .get("availableDecisions")
        .and_then(Value::as_array)
        .is_none_or(|choices| choices.iter().any(|v| v.get(name).is_some() || v.as_str() == Some(name)))
}

fn approval_choices(params: &Value) -> Vec<(&'static str, &'static str)> {
    let offered = params.get("availableDecisions").and_then(Value::as_array);
    [("accept", "Allow once"), ("acceptForSession", "Allow for session"), ("decline", "Deny"), ("cancel", "Cancel turn")]
        .into_iter()
        .filter(|(decision, _)| {
            offered.map_or_else(
                || *decision != "acceptForSession" || params.get("grantRoot").is_some_and(Value::is_string),
                |values| values.iter().any(|v| v.as_str() == Some(decision)),
            )
        })
        .collect()
}

fn permissions(ui: &mut Ui, request: &Interaction, tr: &impl Fn(&str) -> String) -> Option<Action> {
    ui.heading(tr("Allow additional permissions?"));
    detail(ui, "Reason", request.params.get("reason"), tr);
    detail(ui, "Working directory", request.params.get("cwd"), tr);
    let requested = request.params.get("permissions").cloned().unwrap_or_else(|| json!({}));
    detail(ui, "Requested permissions", Some(&requested), tr);
    let mut result = None;
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Allow for this turn")).clicked() {
            result = Some(respond(request, json!({"permissions":requested,"scope":"turn"})));
        }
        if request.params.get("availableScopes").and_then(Value::as_array).is_some_and(|values| values.iter().any(|v| v == "session"))
            && ui.button(tr("Allow for session")).clicked()
        {
            result = Some(respond(request, json!({"permissions":requested,"scope":"session"})));
        }
        if ui.button(tr("Deny")).clicked() {
            result = Some(respond(request, json!({"permissions":{},"scope":"turn"})));
        }
        if ui.button(tr("Cancel turn")).clicked() {
            result = Some(Action::Stop);
        }
    });
    result
}

fn questions(ui: &mut Ui, request: &Interaction, form: &mut InteractionForm, tr: &impl Fn(&str) -> String) -> Option<Action> {
    let Some(questions) = request.params.get("questions").and_then(Value::as_array).filter(|q| !q.is_empty() && q.len() <= 64) else {
        ui.label(tr("The questions are missing or too numerous to display. Cancel this turn."));
        return ui.button(tr("Cancel turn")).clicked().then_some(Action::Stop);
    };
    let mut complete = true;
    for (index, question) in questions.iter().enumerate() {
        let Some(id) = question.get("id").and_then(Value::as_str) else {
            complete = false;
            continue;
        };
        ui.push_id(index, |ui| {
            detail(ui, "Question", question.get("question"), tr);
            let mut answer = form.answers.get(id).and_then(Value::as_str).unwrap_or_default().to_owned();
            let options = question.get("options").and_then(Value::as_array);
            if let Some(options) = options {
                for option in options.iter().take(64) {
                    if let Some(label) = option.get("label").and_then(Value::as_str)
                        && ui
                            .radio(answer == label, label)
                            .on_hover_text(option.get("description").and_then(Value::as_str).unwrap_or_default())
                            .clicked()
                    {
                        answer = label.into();
                    }
                }
            }
            if options.is_none_or(Vec::is_empty) || question.get("isOther").and_then(Value::as_bool) == Some(true) {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut answer)
                        .char_limit(16_384)
                        .password(question.get("isSecret").and_then(Value::as_bool) == Some(true))
                        .desired_width(f32::INFINITY)
                        .hint_text(tr("Your answer")),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::TextInput, true, question.get("question").and_then(Value::as_str).unwrap_or("Your answer"))
                });
            }
            complete &= !answer.trim().is_empty();
            form.answers.insert(id.into(), Value::String(answer));
        });
        ui.separator();
    }
    let mut result = None;
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(complete, egui::Button::new(tr("Submit answers"))).clicked() {
            let answers: Map<String, Value> = form.answers.iter().map(|(id, answer)| (id.clone(), json!({"answers":[answer]}))).collect();
            result = Some(respond(request, json!({"answers":answers})));
        }
        if ui.button(tr("Cancel turn")).clicked() {
            result = Some(Action::Stop);
        }
    });
    result
}

fn elicitation(ui: &mut Ui, request: &Interaction, form: &mut InteractionForm, tr: &impl Fn(&str) -> String) -> Option<Action> {
    detail(ui, "MCP server", request.params.get("serverName"), tr);
    detail(ui, "Request", request.params.get("message"), tr);
    let url = request.params.get("mode").and_then(Value::as_str) == Some("url");
    let mut content = Some(Value::Object(form.answers.clone()));
    if url {
        if let Some(url) = request.params.get("url").and_then(Value::as_str) {
            ui.add(egui::Label::new(url).wrap());
            if url.starts_with("https://") || url.starts_with("http://") {
                ui.hyperlink_to(tr("Open requested page"), url);
            }
        }
        if !request.params.get("url").and_then(Value::as_str).is_some_and(|url| url.starts_with("https://") || url.starts_with("http://")) {
            content = None;
            ui.colored_label(ui.visuals().error_fg_color, tr("The requested page must use HTTP or HTTPS."));
        }
        ui.label(tr("Complete the request in your browser, then choose Done. Opening the page does not approve this request."));
    } else if let Some(schema) = request.params.get("requestedSchema") {
        content = schema_form(ui, schema, form, tr);
    } else {
        ui.label(tr("This request has no form schema."));
        content = None;
    }
    let mut result = None;
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(content.is_some(), egui::Button::new(tr(if url { "Done" } else { "Submit" }))).clicked() {
            result = Some(respond(request, if url { json!({"action":"accept"}) } else { json!({"action":"accept","content":content}) }));
        }
        if ui.button(tr("Deny")).clicked() {
            result = Some(respond(request, json!({"action":"decline"})));
        }
        if ui.button(tr("Cancel")).clicked() {
            result = Some(respond(request, json!({"action":"cancel"})));
        }
    });
    result
}

fn schema_form(ui: &mut Ui, schema: &Value, form: &mut InteractionForm, tr: &impl Fn(&str) -> String) -> Option<Value> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object).filter(|p| p.len() <= 64) else {
        return raw_form(ui, schema, form, tr);
    };
    if properties.values().any(|p| {
        !matches!(p.get("type").and_then(Value::as_str), Some("string" | "boolean" | "number" | "integer" | "array"))
            || (p.get("type").and_then(Value::as_str) == Some("array") && enum_options(p.get("items").unwrap_or(&Value::Null)).is_empty())
    }) {
        return raw_form(ui, schema, form, tr);
    }
    let required = schema.get("required").and_then(Value::as_array);
    let mut complete = true;
    for (key, property) in properties {
        ui.push_id(key, |ui| {
            let title = property.get("title").and_then(Value::as_str).unwrap_or(key);
            ui.label(title);
            if let Some(description) = property.get("description").and_then(Value::as_str) {
                ui.add(egui::Label::new(description).wrap());
            }
            let value = form.answers.entry(key.clone()).or_insert(Value::Null);
            let kind = property.get("type").and_then(Value::as_str).unwrap_or_default();
            let options = enum_options(if kind == "array" { property.get("items").unwrap_or(&Value::Null) } else { property });
            if !options.is_empty() {
                for (choice, label) in options {
                    if kind == "array" {
                        let mut selected = value.as_array().is_some_and(|values| values.contains(&Value::String(choice.clone())));
                        if ui.checkbox(&mut selected, label).changed() {
                            let mut values = value.as_array().cloned().unwrap_or_default();
                            values.retain(|v| v.as_str() != Some(choice.as_str()));
                            if selected {
                                values.push(Value::String(choice));
                            }
                            *value = Value::Array(values);
                        }
                    } else if ui.radio(value.as_str() == Some(choice.as_str()), label).clicked() {
                        *value = Value::String(choice);
                    }
                }
            } else if kind == "boolean" {
                ui.horizontal_wrapped(|ui| {
                    for (choice, label) in [(true, "Yes"), (false, "No")] {
                        if ui.radio(value.as_bool() == Some(choice), tr(label)).clicked() {
                            *value = Value::Bool(choice);
                        }
                    }
                });
            } else {
                let mut text = value.as_str().unwrap_or_default().to_owned();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .char_limit(16_384)
                        .desired_width(f32::INFINITY)
                        .password(property.get("format").and_then(Value::as_str) == Some("password")),
                );
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, title));
                if response.changed() {
                    *value = Value::String(text);
                }
            }
            let is_required = required.is_some_and(|values| values.iter().any(|v| v.as_str() == Some(key)));
            let present = !value.is_null() && value.as_str().is_none_or(|s| !s.is_empty());
            if is_required && !present {
                complete = false;
            }
            if present && !valid_value(value, property) {
                complete = false;
                ui.colored_label(ui.visuals().error_fg_color, tr("Enter a value allowed by this field."));
            }
        });
    }
    if !complete {
        return None;
    }
    Some(Value::Object(
        form.answers
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| {
                let property = properties.get(k);
                let numeric = property.and_then(|p| p.get("type")).and_then(Value::as_str).is_some_and(|kind| kind == "number" || kind == "integer");
                (
                    k.clone(),
                    if numeric { v.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap_or_else(|| v.clone()) } else { v.clone() },
                )
            })
            .collect(),
    ))
}

fn enum_options(schema: &Value) -> Vec<(String, String)> {
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        let names = schema.get("enumNames").and_then(Value::as_array);
        return options
            .iter()
            .take(64)
            .enumerate()
            .filter_map(|(i, v)| v.as_str().map(|value| (value.into(), names.and_then(|n| n.get(i)).and_then(Value::as_str).unwrap_or(value).into())))
            .collect();
    }
    schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(64)
        .filter_map(|option| {
            option
                .get("const")
                .and_then(Value::as_str)
                .map(|value| (value.into(), option.get("title").and_then(Value::as_str).unwrap_or(value).into()))
        })
        .collect()
}

fn valid_value(value: &Value, property: &Value) -> bool {
    match property.get("type").and_then(Value::as_str) {
        Some("number" | "integer") => {
            let Some(number) = value.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()).filter(Value::is_number) else { return false };
            number.as_f64().is_some_and(|v| {
                v.is_finite()
                    && property.get("minimum").and_then(Value::as_f64).is_none_or(|min| v >= min)
                    && property.get("maximum").and_then(Value::as_f64).is_none_or(|max| v <= max)
            }) && (property.get("type").and_then(Value::as_str) != Some("integer") || number.is_i64() || number.is_u64())
        }
        Some("string") => value.as_str().is_some_and(|s| {
            let len = s.chars().count() as u64;
            property.get("minLength").and_then(Value::as_u64).is_none_or(|min| len >= min)
                && property.get("maxLength").and_then(Value::as_u64).is_none_or(|max| len <= max)
        }),
        Some("array") => value.as_array().is_some_and(|a| {
            property.get("minItems").and_then(Value::as_u64).is_none_or(|min| a.len() as u64 >= min)
                && property.get("maxItems").and_then(Value::as_u64).is_none_or(|max| a.len() as u64 <= max)
        }),
        _ => true,
    }
}

fn raw_form(ui: &mut Ui, schema: &Value, form: &mut InteractionForm, tr: &impl Fn(&str) -> String) -> Option<Value> {
    ui.label(tr("This form needs structured JSON. Review the schema and enter your response."));
    ui.collapsing(tr("Requested schema"), |ui| {
        ui.add(egui::Label::new(schema.to_string()).wrap());
    });
    let response = ui.add(egui::TextEdit::multiline(&mut form.raw).char_limit(65_536).desired_width(f32::INFINITY).desired_rows(5));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, tr("Requested form data")));
    if form.raw.len() > 65_536 {
        form.error = Some(tr("The response exceeds the 64 KiB form limit."));
        return None;
    }
    match serde_json::from_str::<Value>(&form.raw) {
        Ok(value) if value.is_object() => {
            form.error = None;
            Some(value)
        }
        _ => {
            if !form.raw.is_empty() {
                form.error = Some(tr("Enter a valid JSON object."));
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::{State, show as assistant_show};
    use egui_kittest::kittest::Queryable as _;

    struct View {
        state: State,
        actions: Vec<Action>,
    }

    fn harness(method: &str, params: Value, width: f32) -> egui_kittest::Harness<'static, View> {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(width, 700.0)).build_ui_state(
            |ui, view: &mut View| view.actions.extend(assistant_show(ui, egui::Id::unique("interaction-test"), &mut view.state, str::to_owned)),
            View {
                state: State {
                    busy: true,
                    available: true,
                    settings_open: true,
                    interaction: Some(Interaction { id: 42, method: method.into(), params }),
                    ..Default::default()
                },
                actions: Vec::new(),
            },
        );
        harness.run_steps(3);
        harness
    }

    #[test]
    fn rendering_never_answers_and_only_offered_session_choice_is_shown() {
        let mut h = harness(
            "item/commandExecution/requestApproval",
            json!({"command":"echo synthetic", "cwd":"/tmp", "availableDecisions":["accept","decline","cancel"]}),
            280.0,
        );
        assert!(h.state().actions.is_empty());
        assert!(h.query_by_label("echo synthetic").is_some());
        assert!(h.query_by_label("Allow for session").is_none());
        for label in ["Allow once", "Deny", "Cancel turn"] {
            let rect = h.get_by_label(label).rect();
            assert!(rect.right() <= 280.0 && rect.left() >= 0.0);
        }
        h.get_by_label("Deny").click();
        h.run_steps(2);
        assert!(matches!(h.state().actions.last(), Some(Action::Respond {id:42,response}) if response == &json!({"decision":"decline"})));
        let h = harness("item/commandExecution/requestApproval", json!({"availableDecisions":["acceptForSession","decline"]}), 420.0);
        assert!(h.query_by_label("Allow for session").is_some());
        assert!(h.query_by_label("Allow once").is_none());
    }

    #[test]
    fn input_choices_wait_for_explicit_selection_and_submission() {
        let mut h = harness(
            "item/tool/requestUserInput",
            json!({"questions":[{"id":"location","question":"Choose location","options":[{"label":"Local","description":"Use local files"},{"label":"Remote","description":"Use remote files"}]}]}),
            280.0,
        );
        assert!(h.state().actions.is_empty());
        h.get_by_label("Submit answers").click();
        h.run_steps(2);
        assert!(h.state().actions.is_empty());
        h.get_by_label("Local").click();
        h.run_steps(2);
        assert!(h.state().actions.is_empty());
        h.get_by_label("Submit answers").click();
        h.run_steps(2);
        assert!(
            matches!(h.state().actions.last(), Some(Action::Respond {id:42,response}) if response == &json!({"answers":{"location":{"answers":["Local"]}}}))
        );
    }

    #[test]
    fn permissions_deny_grants_nothing_and_unknown_requests_only_stop() {
        let mut h =
            harness("item/permissions/requestApproval", json!({"permissions":{"network":{"enabled":true}},"reason":"Fetch reference"}), 280.0);
        assert!(h.state().actions.is_empty());
        assert!(h.query_by_label("Allow for session").is_none());
        h.get_by_label("Deny").click();
        h.run_steps(2);
        assert!(matches!(h.state().actions.last(), Some(Action::Respond {response,..}) if response == &json!({"permissions":{},"scope":"turn"})));
        let mut h = harness("unsupported/request", json!({}), 280.0);
        assert!(h.state().actions.is_empty());
        h.get_by_label("Stop turn").click();
        h.run_steps(2);
        assert!(matches!(h.state().actions.last(), Some(Action::Stop)));
    }

    #[test]
    fn mcp_forms_support_explicit_boolean_and_enum_answers_without_defaults() {
        let mut h = harness(
            "mcpServer/elicitation/request",
            json!({"mode":"form","serverName":"synthetic","message":"Choose export settings","requestedSchema":{"type":"object","required":["overwrite","format"],"properties":{"overwrite":{"type":"boolean","default":true},"format":{"type":"string","enum":["PDF","PNG"]}}}}),
            280.0,
        );
        assert!(h.state().actions.is_empty());
        h.get_by_label("Submit").click();
        h.run_steps(2);
        assert!(h.state().actions.is_empty());
        h.get_by_label("No").click();
        h.run_steps(2);
        h.get_by_label("PDF").click();
        h.run_steps(2);
        h.get_by_label("Submit").click();
        h.run_steps(2);
        assert!(
            matches!(h.state().actions.last(),Some(Action::Respond {response,..}) if response == &json!({"action":"accept","content":{"overwrite":false,"format":"PDF"}}))
        );
    }

    #[test]
    fn pending_forms_are_scoped_to_request_and_secret_responses_are_not_debugged() {
        let mut h =
            harness("item/tool/requestUserInput", json!({"questions":[{"id":"secret","question":"Private response","isSecret":true}]}), 280.0);
        h.state_mut().state.interaction_form.answers.insert("secret".into(), json!("private-value"));
        h.state_mut().state.interaction.as_mut().unwrap().id = 43;
        h.run_steps(2);
        assert_eq!(h.state().state.interaction_form.answers.get("secret"), Some(&json!("")));
        assert!(!format!("{:?}", Action::Respond { id: 42, response: json!({"secret":"private-value"}) }).contains("private-value"));
        for (value, schema, valid) in [
            (json!("4"), json!({"type":"integer","minimum":1,"maximum":5}), true),
            (json!("1.2"), json!({"type":"integer"}), false),
            (json!("NaN"), json!({"type":"number"}), false),
            (json!("0"), json!({"type":"number","minimum":1}), false),
        ] {
            assert_eq!(valid_value(&value, &schema), valid);
        }
    }

    #[test]
    fn url_elicitation_requires_explicit_done_and_http_address() {
        let mut h = harness(
            "mcpServer/elicitation/request",
            json!({"mode":"url","url":"https://example.invalid/verify","serverName":"synthetic","message":"Verify in browser"}),
            280.0,
        );
        assert!(h.state().actions.is_empty());
        h.get_by_label("Done").click();
        h.run_steps(2);
        assert!(matches!(h.state().actions.last(),Some(Action::Respond {response,..}) if response == &json!({"action":"accept"})));
        let mut h = harness("mcpServer/elicitation/request", json!({"mode":"url","url":"file:///private-data","message":"Request"}), 280.0);
        h.get_by_label("Done").click();
        h.run_steps(2);
        assert!(h.state().actions.is_empty());
    }
    #[test]
    fn oversized_and_deep_requests_cannot_offer_grants() {
        let oversized = json!({"command":"x".repeat(65_537)});
        assert!(!bounded_value(&oversized));
        let mut deep = Value::Null;
        for _ in 0..40 {
            deep = json!([deep]);
        }
        assert!(!bounded_value(&deep));
        assert!(!bounded_value(&Value::Array(vec![Value::Null; 4097])));
        let mut h = harness("item/commandExecution/requestApproval", oversized, 280.0);
        assert!(h.query_by_label("Allow once").is_none());
        assert!(h.state().actions.is_empty());
        h.get_by_label("Stop turn").click();
        h.run_steps(2);
        assert!(matches!(h.state().actions.last(), Some(Action::Stop)));
    }

    #[test]
    fn structured_form_requires_valid_explicit_json_and_pending_request_blocks_send() {
        let mut h = harness(
            "mcpServer/elicitation/request",
            json!({"mode":"openai/form","message":"Structured response","requestedSchema":{"type":"object","properties":{"nested":{"type":"object"}}}}),
            420.0,
        );
        assert!(h.state().actions.is_empty());
        h.state_mut().state.interaction_form.raw = "{invalid}".into();
        h.run_steps(2);
        h.get_by_label("Submit").click();
        h.run_steps(2);
        assert!(h.state().actions.is_empty());
        h.state_mut().state.interaction_form.raw = "{\"nested\":{\"value\":4}}".into();
        h.run_steps(2);
        h.get_by_label("Submit").click();
        h.run_steps(2);
        assert!(
            matches!(h.state().actions.last(),Some(Action::Respond{response,..}) if response == &json!({"action":"accept","content":{"nested":{"value":4}}}))
        );
        h.state_mut().state.busy = false;
        h.state_mut().state.draft = "Preserve draft".into();
        assert!(h.state_mut().state.submit().is_none());
        assert_eq!(h.state().state.draft, "Preserve draft");
        h.state_mut().state.interaction = None;
        h.run_steps(2);
        assert!(h.state().state.interaction_form.raw.is_empty());
    }
}
