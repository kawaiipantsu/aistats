//! Turns one JSONL record from a Claude Code or Codex log into [`Event`]s.
//!
//! Most "user" records in both formats are not a person typing: they are tool
//! results, injected context, slash-command plumbing, or another AI. All of
//! that is filtered here so praise and abuse are attributed to humans only.

use std::borrow::Cow;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::lexicon::{model_family, tool_label};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event<'a> {
    /// The working directory of the session.
    Cwd(&'a str),
    Prompt(Cow<'a, str>),
    AiText(Cow<'a, str>),
    Tool {
        label: &'static str,
        command: Option<Cow<'a, str>>,
    },
    ToolFailed,
    ToolDenied,
    Interrupted,
    Compaction,
    Refusal,
    ApiError,
    Usage {
        model: &'static str,
        input: u64,
        cached: u64,
        output: u64,
    },
}

/// Wrapper tags that mark a user-role message as harness-generated. Matching
/// is on the start of the text, since a person may well mention these tags
/// in the middle of a sentence.
const INJECTED_PREFIXES: &[&str] = &[
    "<command-name>",
    "<command-message>",
    "<command-args>",
    "<local-command-",
    "<task-notification>",
    "<system-reminder>",
    "<bash-input>",
    "<bash-stdout>",
    "<bash-stderr>",
    "<user-prompt-submit-hook>",
    "<environment_context>",
    "<user_instructions>",
    "<user_shell_command>",
    "<turn_aborted>",
    "<recommended_plugins>",
    "<send_user_message_question_reply>",
    "<INSTRUCTIONS>",
];

fn is_injected(text: &str) -> bool {
    let text = text.trim_start();
    text.is_empty() || INJECTED_PREFIXES.iter().any(|tag| text.starts_with(tag))
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn u64_field(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn bool_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    let raw = str_field(value, "timestamp")?;
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// A record id, used to drop records that a resumed or forked session copied
/// from an earlier file.
pub fn record_id(value: &Value) -> Option<&str> {
    str_field(value, "uuid").or_else(|| {
        value
            .get("payload")?
            .get("item")?
            .get("id")?
            .as_str()
            .filter(|_| str_field(value, "type") == Some("event_msg"))
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct ClaudeUsage {
    model: &'static str,
    input: u64,
    cached: u64,
    output: u64,
}

/// Per-file Claude Code parser. Stateful because one API message spans several
/// records, each repeating the same usage block: summing them naively roughly
/// doubles every token count.
#[derive(Debug, Default)]
pub struct ClaudeParser {
    usage: HashMap<String, ClaudeUsage>,
}

impl ClaudeParser {
    pub fn line<'a>(&mut self, record: &'a Value, out: &mut Vec<Event<'a>>) {
        if let Some(cwd) = str_field(record, "cwd") {
            out.push(Event::Cwd(cwd));
        }
        match str_field(record, "type") {
            Some("user") => Self::user(record, out),
            Some("assistant") => self.assistant(record, out),
            Some("system") if str_field(record, "subtype") == Some("compact_boundary") => {
                out.push(Event::Compaction);
            }
            _ => {}
        }
    }

    /// Emits token usage, deduplicated per API message.
    pub fn finish(self, out: &mut Vec<Event<'_>>) {
        out.extend(self.usage.into_values().map(|usage| Event::Usage {
            model: usage.model,
            input: usage.input,
            cached: usage.cached,
            output: usage.output,
        }));
    }

    fn is_human(record: &Value) -> bool {
        if bool_field(record, "isSidechain")
            || bool_field(record, "isMeta")
            || bool_field(record, "isCompactSummary")
            || bool_field(record, "isVisibleInTranscriptOnly")
            || str_field(record, "promptSource") == Some("system")
        {
            return false;
        }
        match record.get("origin") {
            None | Some(Value::Null) => true,
            Some(origin) => origin.as_str().or_else(|| str_field(origin, "kind")) == Some("human"),
        }
    }

    fn user<'a>(record: &'a Value, out: &mut Vec<Event<'a>>) {
        let Some(content) = record.get("message").and_then(|m| m.get("content")) else {
            return;
        };
        let human = Self::is_human(record);

        if let Some(text) = content.as_str() {
            Self::user_text(text, human, out);
            return;
        }

        let denied = record.get("toolDenialKind").is_some();
        for block in content.as_array().into_iter().flatten() {
            match str_field(block, "type") {
                Some("tool_result") => {
                    let is_error = bool_field(block, "is_error");
                    if denied || (is_error && tool_was_denied(block)) {
                        out.push(Event::ToolDenied);
                    } else if is_error {
                        out.push(Event::ToolFailed);
                    }
                }
                Some("text") => {
                    if let Some(text) = str_field(block, "text") {
                        Self::user_text(text, human, out);
                    }
                }
                _ => {}
            }
        }
    }

    fn user_text<'a>(text: &'a str, human: bool, out: &mut Vec<Event<'a>>) {
        if text.starts_with("[Request interrupted") {
            out.push(Event::Interrupted);
        } else if human && !is_injected(text) {
            out.push(Event::Prompt(Cow::Borrowed(text)));
        }
    }

    fn assistant<'a>(&mut self, record: &'a Value, out: &mut Vec<Event<'a>>) {
        let Some(message) = record.get("message") else {
            return;
        };
        // Harness-synthesized error notices are logged as assistant messages,
        // but the AI said none of it.
        if bool_field(record, "isApiErrorMessage") {
            out.push(Event::ApiError);
            return;
        }
        if str_field(message, "stop_reason") == Some("refusal") {
            out.push(Event::Refusal);
        }

        for block in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match str_field(block, "type") {
                Some("text") => {
                    if let Some(text) = str_field(block, "text") {
                        out.push(Event::AiText(Cow::Borrowed(text)));
                    }
                }
                Some("tool_use") => {
                    let name = str_field(block, "name").unwrap_or_default();
                    let command = block
                        .get("input")
                        .and_then(|input| str_field(input, "command"))
                        .filter(|_| name == "Bash")
                        .map(Cow::Borrowed);
                    out.push(Event::Tool {
                        label: tool_label(name),
                        command,
                    });
                }
                _ => {}
            }
        }

        if let (Some(id), Some(usage)) = (str_field(message, "id"), message.get("usage")) {
            let model = str_field(message, "model").unwrap_or_default();
            let entry = self.usage.entry(id.to_string()).or_default();
            entry.model = model_family(model);
            entry.input = entry.input.max(
                u64_field(usage, "input_tokens") + u64_field(usage, "cache_creation_input_tokens"),
            );
            entry.cached = entry
                .cached
                .max(u64_field(usage, "cache_read_input_tokens"));
            entry.output = entry.output.max(u64_field(usage, "output_tokens"));
        }
    }
}

/// Current Claude Code marks a declined call with `toolDenialKind` on the
/// record. Older versions only log an errored result whose text gives it away.
fn tool_was_denied(block: &Value) -> bool {
    str_field(block, "content").is_some_and(|text| text.contains("doesn't want to proceed"))
}

/// Per-file Codex parser. Current logs record everything as `item_completed`
/// events; older ones use `user_message`, `agent_message` and raw function
/// calls. A file that has items also repeats some of those older shapes, so
/// they are only read until the first item is seen.
#[derive(Debug, Default)]
pub struct CodexParser {
    subagent: bool,
    saw_items: bool,
    model: &'static str,
    totals: (u64, u64, u64),
    pub session_id: Option<String>,
}

impl CodexParser {
    pub fn line<'a>(&mut self, record: &'a Value, out: &mut Vec<Event<'a>>) {
        let Some(payload) = record.get("payload") else {
            return;
        };
        let payload_type = str_field(payload, "type");

        match str_field(record, "type") {
            Some("session_meta") => {
                self.subagent = payload
                    .get("source")
                    .is_some_and(|source| source.get("subagent").is_some());
                self.session_id = str_field(payload, "id").map(str::to_string);
                out.extend(str_field(payload, "cwd").map(Event::Cwd));
            }
            Some("turn_context") => {
                out.extend(str_field(payload, "cwd").map(Event::Cwd));
                if let Some(model) = str_field(payload, "model") {
                    self.model = model_family(model);
                }
            }
            Some("event_msg") => match payload_type {
                Some("item_completed") => {
                    self.saw_items = true;
                    if let Some(item) = payload.get("item") {
                        self.item(item, out);
                    }
                }
                Some("user_message") if !self.saw_items && !self.subagent => {
                    if let Some(text) = str_field(payload, "message").filter(|t| !is_injected(t)) {
                        out.push(Event::Prompt(Cow::Borrowed(text)));
                    }
                }
                Some("agent_message") if !self.saw_items => {
                    if let Some(text) = str_field(payload, "message") {
                        out.push(Event::AiText(Cow::Borrowed(text)));
                    }
                }
                Some("token_count") => self.tokens(payload, out),
                Some("turn_aborted") if str_field(payload, "reason") == Some("interrupted") => {
                    out.push(Event::Interrupted);
                }
                Some("error") => out.push(Event::ApiError),
                _ => {}
            },
            Some("response_item")
                if !self.saw_items
                    && matches!(payload_type, Some("function_call" | "custom_tool_call")) =>
            {
                out.push(legacy_tool_call(payload));
            }
            Some("compacted") if !self.saw_items => out.push(Event::Compaction),
            _ => {}
        }
    }

    fn item<'a>(&self, item: &'a Value, out: &mut Vec<Event<'a>>) {
        let failed = str_field(item, "status") == Some("failed");
        match str_field(item, "type") {
            Some("UserMessage") if !self.subagent => {
                let text = joined_text(item);
                if !is_injected(&text) {
                    out.push(Event::Prompt(text));
                }
            }
            Some("AgentMessage") => out.push(Event::AiText(joined_text(item))),
            Some("CommandExecution") => {
                out.push(Event::Tool {
                    label: "Shell",
                    command: item.get("command").and_then(shell_command),
                });
                let exit_code = item.get("exit_code").and_then(Value::as_i64);
                if failed || exit_code.is_some_and(|code| code != 0) {
                    out.push(Event::ToolFailed);
                }
            }
            Some("FileChange") => {
                out.push(Event::Tool {
                    label: "Edit",
                    command: None,
                });
                if failed {
                    out.push(Event::ToolFailed);
                }
            }
            Some("McpToolCall") => out.push(Event::Tool {
                label: "MCP",
                command: None,
            }),
            Some("ImageView") => out.push(Event::Tool {
                label: "Read",
                command: None,
            }),
            Some("SubAgentActivity") => out.push(Event::Tool {
                label: "Subagent",
                command: None,
            }),
            Some("Extension") => out.push(Event::Tool {
                label: "Web",
                command: None,
            }),
            Some("ContextCompaction") => out.push(Event::Compaction),
            _ => {}
        }
    }

    /// `total_token_usage` is a running total for the session and is logged
    /// repeatedly, so only the growth since the previous record is new usage.
    /// Codex includes cached tokens in `input_tokens`; they are separated here
    /// to match how Claude reports them.
    fn tokens<'a>(&mut self, payload: &'a Value, out: &mut Vec<Event<'a>>) {
        let Some(total) = payload
            .get("info")
            .and_then(|info| info.get("total_token_usage"))
        else {
            return;
        };
        let cached = u64_field(total, "cached_input_tokens");
        let input = u64_field(total, "input_tokens").saturating_sub(cached);
        let output = u64_field(total, "output_tokens");

        let (last_input, last_cached, last_output) = self.totals;
        let new_tokens = (
            input.saturating_sub(last_input),
            cached.saturating_sub(last_cached),
            output.saturating_sub(last_output),
        );
        self.totals = (
            input.max(last_input),
            cached.max(last_cached),
            output.max(last_output),
        );
        if new_tokens != (0, 0, 0) {
            out.push(Event::Usage {
                model: if self.model.is_empty() {
                    "Other"
                } else {
                    self.model
                },
                input: new_tokens.0,
                cached: new_tokens.1,
                output: new_tokens.2,
            });
        }
    }
}

fn joined_text(item: &Value) -> Cow<'_, str> {
    let mut parts = item
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| str_field(block, "text"));
    let Some(first) = parts.next() else {
        return Cow::Borrowed("");
    };
    match parts.next() {
        None => Cow::Borrowed(first),
        Some(second) => {
            let mut text = format!("{first}\n{second}");
            for part in parts {
                text.push('\n');
                text.push_str(part);
            }
            Cow::Owned(text)
        }
    }
}

/// Extracts the script from Codex's argv form, `["/bin/zsh", "-lc", "..."]`.
fn shell_command(command: &Value) -> Option<Cow<'_, str>> {
    if let Some(text) = command.as_str() {
        return Some(Cow::Borrowed(text));
    }
    let argv: Vec<&str> = command
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    match argv.as_slice() {
        [_, flag, script] if flag.ends_with('c') => Some(Cow::Borrowed(*script)),
        [] => None,
        parts => Some(Cow::Owned(parts.join(" "))),
    }
}

fn legacy_tool_call(payload: &Value) -> Event<'_> {
    let name = str_field(payload, "name").unwrap_or_default();
    let command = str_field(payload, "arguments")
        .and_then(|args| serde_json::from_str::<Value>(args).ok())
        .and_then(|args| {
            let command = args.get("command").or_else(|| args.get("cmd"))?;
            shell_command(command).map(|text| Cow::Owned(text.into_owned()))
        });
    Event::Tool {
        label: tool_label(name),
        command,
    }
}

/// Reads a `~/.claude/history.jsonl` or `~/.codex/history.jsonl` record. These
/// outlive session files, which both tools prune, so they supply prompts for
/// sessions that no longer exist. Returns `(session id, text, timestamp)`.
pub fn history_entry(record: &Value) -> Option<(&str, &str, Option<DateTime<Utc>>)> {
    let session = str_field(record, "sessionId").or_else(|| str_field(record, "session_id"))?;
    let text = str_field(record, "display").or_else(|| str_field(record, "text"))?;
    if is_injected(text) || text.starts_with('/') {
        return None;
    }
    let timestamp = record
        .get("timestamp")
        .and_then(Value::as_i64)
        .and_then(DateTime::from_timestamp_millis)
        .or_else(|| {
            record
                .get("ts")
                .and_then(Value::as_i64)
                .and_then(|secs| DateTime::from_timestamp(secs, 0))
        });
    Some((session, text, timestamp))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn claude(record: Value) -> Vec<Event<'static>> {
        let record: &'static Value = Box::leak(Box::new(record));
        let mut parser = ClaudeParser::default();
        let mut out = Vec::new();
        parser.line(record, &mut out);
        parser.finish(&mut out);
        out
    }

    fn codex(records: Vec<Value>) -> Vec<Event<'static>> {
        let records: &'static Vec<Value> = Box::leak(Box::new(records));
        let mut parser = CodexParser::default();
        let mut out = Vec::new();
        for record in records {
            parser.line(record, &mut out);
        }
        out
    }

    fn prompts(events: &[Event<'_>]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, Event::Prompt(_)))
            .count()
    }

    fn user_message(text: &str) -> Value {
        json!({"type": "event_msg", "payload": {"type": "item_completed",
            "item": {"type": "UserMessage", "id": "i1",
                     "content": [{"type": "text", "text": text}]}}})
    }

    #[test]
    fn working_directories_are_reported_by_both_parsers() {
        let events = claude(json!({"type": "user", "cwd": "/srv/thing",
                                   "message": {"content": "hi"}}));
        assert_eq!(events[0], Event::Cwd("/srv/thing"));
        let events = codex(vec![
            json!({"type": "session_meta", "payload": {"id": "s", "cwd": "/srv/other"}}),
        ]);
        assert_eq!(events, [Event::Cwd("/srv/other")]);
    }

    #[test]
    fn claude_typed_prompt_is_human() {
        for record in [
            json!({"type": "user", "message": {"content": "fix this"}}),
            json!({"type": "user", "origin": {"kind": "human"},
                   "message": {"content": "fix this"}}),
            json!({"type": "user", "promptSource": "typed",
                   "message": {"content": [{"type": "text", "text": "fix this"}]}}),
        ] {
            assert_eq!(prompts(&claude(record)), 1);
        }
    }

    #[test]
    fn claude_non_human_user_records_are_not_prompts() {
        for record in [
            json!({"type": "user", "isMeta": true, "message": {"content": "skill text"}}),
            json!({"type": "user", "isSidechain": true, "message": {"content": "subagent task"}}),
            json!({"type": "user", "isCompactSummary": true, "message": {"content": "summary"}}),
            json!({"type": "user", "promptSource": "system", "message": {"content": "system"}}),
            json!({"type": "user", "origin": {"kind": "task-notification"},
                   "message": {"content": "done"}}),
            json!({"type": "user", "origin": {"kind": "auto-continuation"},
                   "message": {"content": "keep going"}}),
            json!({"type": "user", "message": {"content": "<command-name>/login</command-name>"}}),
            json!({"type": "user", "message": {"content": "<local-command-stdout>x"}}),
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t", "content": "file contents"}]}}),
        ] {
            assert_eq!(prompts(&claude(record.clone())), 0, "{record}");
        }
    }

    #[test]
    fn claude_usage_counts_each_api_message_once() {
        // Three content blocks of one message, each repeating its usage block.
        let usage = json!({"input_tokens": 10, "cache_creation_input_tokens": 5,
                           "cache_read_input_tokens": 100, "output_tokens": 7});
        let records: &'static Vec<Value> = Box::leak(Box::new(vec![
            json!({"type": "assistant", "message": {"id": "m1", "model": "claude-opus-5-5",
                   "usage": usage, "content": [{"type": "thinking", "thinking": ""}]}}),
            json!({"type": "assistant", "message": {"id": "m1", "model": "claude-opus-5-5",
                   "usage": usage, "content": [{"type": "text", "text": "hi"}]}}),
            json!({"type": "assistant", "message": {"id": "m1", "model": "claude-opus-5-5",
                   "usage": usage, "content": [{"type": "tool_use", "name": "Read"}]}}),
        ]));
        let mut parser = ClaudeParser::default();
        let mut out = Vec::new();
        for record in records {
            parser.line(record, &mut out);
        }
        parser.finish(&mut out);
        let usages: Vec<_> = out
            .iter()
            .filter(|e| matches!(e, Event::Usage { .. }))
            .collect();
        assert_eq!(
            usages,
            [&Event::Usage {
                model: "Claude Opus",
                input: 15,
                cached: 100,
                output: 7,
            }]
        );
    }

    #[test]
    fn claude_tool_outcomes_and_interrupts() {
        let events = claude(json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "is_error": true, "content": "exit status 1"},
            {"type": "tool_result", "is_error": true,
             "content": "The user doesn't want to proceed with this tool use."},
            {"type": "text", "text": "[Request interrupted by user]"}]}}));
        assert_eq!(
            events,
            [Event::ToolFailed, Event::ToolDenied, Event::Interrupted]
        );
    }

    #[test]
    fn claude_denial_kind_is_a_denial_even_without_is_error() {
        let events = claude(json!({"type": "user", "toolDenialKind": "user-rejected",
            "message": {"content": [{"type": "tool_result", "content": "declined"}]}}));
        assert_eq!(events, [Event::ToolDenied]);
    }

    #[test]
    fn claude_synthetic_error_is_not_ai_speech() {
        let events = claude(json!({"type": "assistant", "isApiErrorMessage": true,
            "message": {"id": "m", "content": [{"type": "text", "text": "rate limited"}]}}));
        assert_eq!(events, [Event::ApiError]);
    }

    #[test]
    fn claude_refusal_is_detected_and_only_bash_keeps_its_command() {
        let events = claude(json!({"type": "assistant", "message": {
            "stop_reason": "refusal",
            "content": [
                {"type": "tool_use", "name": "Bash", "input": {"command": "ls"}},
                {"type": "tool_use", "name": "mcp__private__x", "input": {"command": "nope"}}]}}));
        assert_eq!(
            events,
            [
                Event::Refusal,
                Event::Tool {
                    label: "Shell",
                    command: Some(Cow::Borrowed("ls")),
                },
                Event::Tool {
                    label: "MCP",
                    command: None,
                },
            ]
        );
    }

    #[test]
    fn codex_user_items_are_prompts_but_injected_context_is_not() {
        let events = codex(vec![
            user_message("please fix the build"),
            user_message("<environment_context>cwd</environment_context>"),
            user_message("<send_user_message_question_reply>[]"),
            json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                   "content": [{"type": "input_text", "text": "replayed"}]}}),
        ]);
        assert_eq!(
            events,
            [Event::Prompt(Cow::Borrowed("please fix the build"))]
        );
    }

    #[test]
    fn codex_subagent_sessions_have_no_human() {
        let events = codex(vec![
            json!({"type": "session_meta", "payload": {
                "id": "s", "source": {"subagent": {"thread_spawn": {}}}}}),
            user_message("orchestrator instructions"),
        ]);
        assert_eq!(prompts(&events), 0);
    }

    #[test]
    fn codex_failed_commands_and_argv_unwrapping() {
        let events = codex(vec![json!({"type": "event_msg", "payload": {
            "type": "item_completed", "item": {
                "type": "CommandExecution", "id": "c", "status": "failed", "exit_code": 2,
                "command": ["/bin/zsh", "-lc", "cargo test"]}}})]);
        assert_eq!(
            events,
            [
                Event::Tool {
                    label: "Shell",
                    command: Some(Cow::Borrowed("cargo test")),
                },
                Event::ToolFailed,
            ]
        );
    }

    #[test]
    fn codex_token_totals_become_deltas() {
        let count = |input: u64, cached: u64, output: u64| {
            json!({"type": "event_msg", "payload": {"type": "token_count", "info": {
                "total_token_usage": {"input_tokens": input,
                                      "cached_input_tokens": cached,
                                      "output_tokens": output}}}})
        };
        let events = codex(vec![
            json!({"type": "turn_context", "payload": {"model": "gpt-6-astra"}}),
            count(100, 40, 10),
            count(100, 40, 10),
            count(180, 90, 30),
        ]);
        let usage = |input, cached, output| Event::Usage {
            model: "GPT-6",
            input,
            cached,
            output,
        };
        assert_eq!(events, [usage(60, 40, 10), usage(30, 50, 20)]);
    }

    #[test]
    fn codex_legacy_shapes_are_read_when_items_are_absent() {
        let events = codex(vec![
            json!({"type": "event_msg", "payload": {"type": "user_message", "message": "hi"}}),
            json!({"type": "event_msg", "payload": {"type": "agent_message", "message": "hello"}}),
            json!({"type": "response_item", "payload": {"type": "function_call", "name": "shell",
                   "arguments": "{\"command\":[\"bash\",\"-lc\",\"ls -la\"]}"}}),
            json!({"type": "compacted", "payload": {"message": ""}}),
        ]);
        assert_eq!(
            events,
            [
                Event::Prompt(Cow::Borrowed("hi")),
                Event::AiText(Cow::Borrowed("hello")),
                Event::Tool {
                    label: "Shell",
                    command: Some(Cow::Owned("ls -la".to_string())),
                },
                Event::Compaction,
            ]
        );
    }

    #[test]
    fn codex_legacy_shapes_are_ignored_once_items_appear() {
        let events = codex(vec![
            user_message("real prompt"),
            json!({"type": "event_msg", "payload": {"type": "user_message", "message": "dupe"}}),
            json!({"type": "response_item", "payload": {"type": "function_call", "name": "shell",
                   "arguments": "{}"}}),
        ]);
        assert_eq!(events, [Event::Prompt(Cow::Borrowed("real prompt"))]);
    }

    #[test]
    fn history_skips_slash_commands_and_reads_both_timestamp_styles() {
        let claude = json!({"display": "fix it", "sessionId": "a", "timestamp": 1_000_000_i64});
        let codex = json!({"text": "fix it", "session_id": "b", "ts": 1000});
        assert_eq!(
            history_entry(&claude).unwrap().2,
            DateTime::from_timestamp(1000, 0)
        );
        assert_eq!(
            history_entry(&codex).unwrap().2,
            DateTime::from_timestamp(1000, 0)
        );
        let command = json!({"display": "/login", "sessionId": "a"});
        assert!(history_entry(&command).is_none());
    }
}
