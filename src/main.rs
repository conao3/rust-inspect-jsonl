mod error;
mod grammar;
mod output;

use clap::{Args, CommandFactory, Parser, Subcommand};
use serde::Serialize;
use serde_json::{Map, Value, json};

use error::{Result, other, parameter, target};
use grammar::{CliInputArgs, SkeletonKind};
use output::{OutputFormat, Response};

#[derive(Parser)]
#[command(
    name = "rust-inspect-jsonl",
    version,
    about = "Inspect Claude/Codex JSONL session logs"
)]
struct Cli {
    /// Output format
    #[arg(long, global = true, help_heading = "Global Options", value_enum, default_value_t = OutputFormat::Json)]
    output: OutputFormat,
    /// JMESPath expression applied to the output
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        value_name = "JMESPATH"
    )]
    query: Option<String>,
    /// Print debug logs to stderr
    #[arg(long, global = true, help_heading = "Global Options")]
    debug: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect JSONL session logs
    #[command(subcommand)]
    Session(SessionCommand),
}

#[derive(Subcommand)]
enum SessionCommand {
    /// Normalize every JSON line of a session log into an analysis event with its timestamp, type, tool name, usage, and payload size
    #[command(name = "inspect-session")]
    Inspect(InspectSessionArgs),
}

#[derive(Args)]
struct InspectSessionArgs {
    /// Content of the JSONL session log. Use file://session.jsonl to read a file and file:///dev/stdin to read stdin
    #[arg(long)]
    session_log: String,
    #[command(flatten)]
    input: CliInputArgs,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "PascalCase")]
struct InspectSessionReport {
    parsed_lines: usize,
    skipped_lines: Vec<SkippedLine>,
    events: Vec<InspectEvent>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "PascalCase")]
struct SkippedLine {
    line_number: usize,
    error: String,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "PascalCase")]
struct InspectEvent {
    line_number: usize,
    timestamp: Option<String>,
    event_type: Option<String>,
    tool_name: Option<String>,
    usage: Value,
    payload: PayloadInfo,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "PascalCase")]
struct PayloadInfo {
    kind: String,
    chars: usize,
}

impl InspectSessionReport {
    fn skeleton() -> Self {
        Self {
            parsed_lines: 0,
            skipped_lines: vec![SkippedLine::default()],
            events: vec![InspectEvent {
                usage: json!({}),
                ..InspectEvent::default()
            }],
        }
    }
}

fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            error.exit_code()
        }
    };
    std::process::exit(code);
}

fn to_value<T: Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| other(error.to_string()))
}

fn output_skeleton(path: &[String]) -> Result<Value> {
    match path.last().map(String::as_str) {
        Some("inspect-session") => to_value(&InspectSessionReport::skeleton()),
        _ => Err(parameter(
            "--generate-cli-skeleton needs a command and a subcommand",
        )),
    }
}

fn run() -> Result<()> {
    let mut root = Cli::command();
    root.build();
    let prepared = grammar::prepare(&root, std::env::args_os().collect())?;
    let debug = prepared.argv.iter().any(|token| token == "--debug");
    if debug {
        let argv: Vec<String> = prepared.argv[1..]
            .iter()
            .map(|token| token.chars().take(200).collect())
            .collect();
        eprintln!("debug: argv {argv:?}");
    }

    if let Some(kind) = prepared.skeleton {
        let mut command = &root;
        for name in &prepared.path {
            command = command
                .find_subcommand(name)
                .ok_or_else(|| other(format!("unknown subcommand {name}")))?;
        }
        if command.has_subcommands() {
            return Err(parameter(
                "--generate-cli-skeleton needs a command and a subcommand",
            ));
        }
        let value = match kind {
            SkeletonKind::Input => grammar::input_skeleton(command),
            SkeletonKind::Output => output_skeleton(&prepared.path)?,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| other(error.to_string()))?
        );
        return Ok(());
    }

    let cli = match Cli::try_parse_from(&prepared.argv) {
        Ok(cli) => cli,
        Err(error) => {
            let code = if error.use_stderr() { 252 } else { 0 };
            let _ = error.print();
            std::process::exit(code);
        }
    };

    if let Some(query) = &cli.query {
        output::compile_query(query)?;
    }
    let Command::Session(command) = cli.command;
    let (name, report) = match command {
        SessionCommand::Inspect(args) => ("inspect-session", inspect(&args.session_log)?),
    };
    if debug {
        eprintln!(
            "debug: parsed {} lines, skipped {} lines",
            report.parsed_lines,
            report.skipped_lines.len()
        );
    }
    print!(
        "{}",
        output::render(
            &grammar::pascal_case(name),
            Response {
                value: to_value(&report)?,
                text: None,
            },
            cli.output,
            cli.query.as_deref()
        )?
    );
    Ok(())
}

fn inspect(session_log: &str) -> Result<InspectSessionReport> {
    let mut report = InspectSessionReport::default();

    for (index, line) in session_log.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<Value>(line) {
            Ok(value) => {
                report.parsed_lines += 1;
                report.events.push(InspectEvent {
                    line_number: index + 1,
                    timestamp: find_string(&value, &["timestamp", "created_at", "createdAt"]),
                    event_type: find_event_type(&value),
                    tool_name: find_tool_name(&value),
                    usage: find_usage(&value),
                    payload: payload_info(&value),
                });
            }
            Err(error) => report.skipped_lines.push(SkippedLine {
                line_number: index + 1,
                error: error.to_string(),
            }),
        }
    }

    if report.parsed_lines == 0 && !report.skipped_lines.is_empty() {
        return Err(target(format!(
            "--session-log is not JSONL: none of its {} non-empty lines is JSON",
            report.skipped_lines.len()
        )));
    }
    Ok(report)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_normalizes_lines_and_records_skipped_ones() {
        let log = concat!(
            r#"{"type":"assistant","timestamp":"2026-02-27T09:36:33Z","message":{"usage":{"output_tokens":45},"content":[{"type":"tool_use","name":"Read","input":"abc"}]}}"#,
            "\n\nnot json\n"
        );
        let value = to_value(&inspect(log).unwrap()).unwrap();
        assert_eq!(
            value,
            json!({
                "ParsedLines": 1,
                "SkippedLines": [{"LineNumber": 3, "Error": "expected ident at line 1 column 2"}],
                "Events": [{
                    "LineNumber": 1,
                    "Timestamp": "2026-02-27T09:36:33Z",
                    "EventType": "assistant",
                    "ToolName": "Read",
                    "Usage": {"output_tokens": 45},
                    "Payload": {"Kind": "tool_use", "Chars": 3}
                }]
            })
        );
    }

    #[test]
    fn inspect_rejects_input_without_json_lines() {
        let error = inspect("a\nb\n").err().unwrap();
        assert_eq!(error.exit_code(), 254);
    }
}
