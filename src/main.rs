use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::io::{self, BufRead, Write};

#[derive(Parser)]
#[command(name = "rust-inspect-jsonl")]
#[command(about = "Inspect Claude/Codex JSONL session logs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Inspect {
        #[arg(long)]
        input_jsonl: bool,
        #[arg(long)]
        output_jsonl: bool,
    },
}

#[derive(Serialize)]
struct InspectEvent {
    line_number: usize,
    timestamp: Option<String>,
    event_type: Option<String>,
    tool_name: Option<String>,
    usage: Value,
    payload: PayloadInfo,
}

#[derive(Serialize)]
struct PayloadInfo {
    kind: String,
    chars: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Inspect {
            input_jsonl,
            output_jsonl,
        } => inspect(input_jsonl, output_jsonl),
    }
}

fn inspect(input_jsonl: bool, output_jsonl: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !input_jsonl || !output_jsonl {
        return Err("inspect requires --input-jsonl and --output-jsonl".into());
    }

    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut parsed = 0usize;
    let mut skipped = 0usize;

    for (index, line) in stdin.lock().lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<Value>(&line) {
            Ok(value) => {
                parsed += 1;
                serde_json::to_writer(
                    &mut stdout,
                    &InspectEvent {
                        line_number: index + 1,
                        timestamp: find_string(&value, &["timestamp", "created_at", "createdAt"]),
                        event_type: find_event_type(&value),
                        tool_name: find_tool_name(&value),
                        usage: find_usage(&value),
                        payload: payload_info(&value),
                    },
                )?;
                stdout.write_all(b"\n")?;
            }
            Err(error) => {
                skipped += 1;
                eprintln!("warning: skipped line {}: {}", index + 1, error);
            }
        }
    }

    eprintln!("parsed={} skipped={}", parsed, skipped);
    Ok(())
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| find_string(message, keys))
        })
}

fn find_event_type(value: &Value) -> Option<String> {
    find_string(value, &["type", "role"]).or_else(|| {
        value
            .get("message")
            .and_then(|message| find_string(message, &["type", "role"]))
    })
}

fn find_usage(value: &Value) -> Value {
    value
        .get("usage")
        .cloned()
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.get("usage"))
                .cloned()
        })
        .unwrap_or_else(|| json!({}))
}

fn find_tool_name(value: &Value) -> Option<String> {
    find_tool_name_in_value(value)
        .or_else(|| value.get("message").and_then(find_tool_name_in_value))
}

fn find_tool_name_in_value(value: &Value) -> Option<String> {
    match value.get("content") {
        Some(Value::Array(items)) => items.iter().find_map(tool_name_from_content_item),
        Some(item) => tool_name_from_content_item(item),
        None => None,
    }
}

fn tool_name_from_content_item(value: &Value) -> Option<String> {
    value
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| value.get("tool_name").and_then(Value::as_str))
        .map(str::to_owned)
        .or_else(|| {
            value
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

fn payload_info(value: &Value) -> PayloadInfo {
    let payload = value.get("message").unwrap_or(value);
    let content = payload.get("content").unwrap_or(payload);

    PayloadInfo {
        kind: payload_kind(content),
        chars: content_chars(content),
    }
}

fn payload_kind(value: &Value) -> String {
    match value {
        Value::String(_) => "text".to_owned(),
        Value::Array(items) => items
            .iter()
            .find_map(|item| item.get("type").and_then(Value::as_str))
            .unwrap_or("array")
            .to_owned(),
        Value::Object(map) => map
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("object")
            .to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(_) => "bool".to_owned(),
        Value::Number(_) => "number".to_owned(),
    }
}

fn content_chars(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        Value::Array(items) => items.iter().map(content_chars).sum(),
        Value::Object(map) => object_chars(map),
        Value::Null => 0,
        Value::Bool(value) => value.to_string().len(),
        Value::Number(value) => value.to_string().len(),
    }
}

fn object_chars(map: &Map<String, Value>) -> usize {
    map.get("text")
        .or_else(|| map.get("content"))
        .or_else(|| map.get("input"))
        .map(content_chars)
        .unwrap_or_else(|| Value::Object(map.clone()).to_string().chars().count())
}
