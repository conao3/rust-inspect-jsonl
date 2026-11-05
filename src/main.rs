mod error;
mod grammar;
mod output;

use clap::{Args, CommandFactory, Parser, Subcommand};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Cursor, Write};

use error::{Result, other, parameter, target};
use output::OutputFormat;

const SESSION_LOG: &str = "session-log";

#[derive(Parser)]
#[command(
    name = "rust-inspect-jsonl",
    version,
    about = "Inspect Claude/Codex JSONL session logs"
)]
struct Cli {
    /// Output format, applied to each output line (json writes one JSON object per line)
    #[arg(long, global = true, help_heading = "Global Options", value_enum, default_value_t = OutputFormat::Json)]
    output: OutputFormat,
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
    /// Stream one normalized analysis event per JSON line of a session log (timestamp, type, tool name, usage, payload size), then one summary line
    #[command(name = "inspect-session")]
    Inspect(InspectSessionArgs),
}

#[derive(Args)]
struct InspectSessionArgs {
    /// JSONL session log, read line by line. Use file://session.jsonl to read a file and file:///dev/stdin to read stdin; any other value is the log itself
    #[arg(long)]
    session_log: String,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "PascalCase")]
struct InspectSessionSummary {
    parsed_lines: usize,
    skipped_lines: Vec<SkippedLine>,
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

fn open_session_log(session_log: String) -> Result<Box<dyn BufRead>> {
    match session_log.strip_prefix("file://") {
        Some(path) => {
            let file = File::open(path).map_err(|error| {
                parameter(format!(
                    "Error parsing parameter: Unable to load paramfile {session_log}: {error}"
                ))
            })?;
            Ok(Box::new(BufReader::new(file)))
        }
        None => Ok(Box::new(Cursor::new(session_log.into_bytes()))),
    }
}

fn run() -> Result<()> {
    let mut root = Cli::command();
    root.build();
    let argv = grammar::prepare(&root, std::env::args_os().collect(), &[SESSION_LOG])?;
    let debug = argv.iter().any(|token| token == "--debug");
    if debug {
        let shown: Vec<String> = argv[1..]
            .iter()
            .map(|token| token.chars().take(200).collect())
            .collect();
        eprintln!("debug: argv {shown:?}");
    }

    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(error) => {
            let code = if error.use_stderr() { 252 } else { 0 };
            let _ = error.print();
            std::process::exit(code);
        }
    };

    let Command::Session(SessionCommand::Inspect(args)) = cli.command;
    let title = grammar::pascal_case("inspect-session");
    let reader = open_session_log(args.session_log)?;
    let mut out = BufWriter::new(io::stdout().lock());
    let mut emit = |value: Value| -> Result<()> {
        let line = output::render_line(&title, value, cli.output)?;
        write_out(&mut out, line.as_bytes())
    };
    let summary = inspect(reader, &mut emit, debug)?;
    let not_jsonl = summary.parsed_lines == 0 && !summary.skipped_lines.is_empty();
    let skipped = summary.skipped_lines.len();
    emit(to_value(&summary)?)?;
    drop(emit);
    out.flush().or_else(broken_pipe)?;
    if not_jsonl {
        return Err(target(format!(
            "--session-log is not JSONL: none of its {skipped} non-empty lines is JSON"
        )));
    }
    Ok(())
}

fn broken_pipe(error: io::Error) -> Result<()> {
    if error.kind() == io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
    Err(other(format!("failed to write the output: {error}")))
}

fn write_out(out: &mut impl Write, bytes: &[u8]) -> Result<()> {
    out.write_all(bytes).or_else(broken_pipe)
}

fn inspect(
    mut reader: impl BufRead,
    emit: &mut impl FnMut(Value) -> Result<()>,
    debug: bool,
) -> Result<InspectSessionSummary> {
    let mut summary = InspectSessionSummary::default();
    let mut buffer = Vec::new();
    let mut line_number = 0;

    loop {
        buffer.clear();
        let read = reader
            .read_until(b'\n', &mut buffer)
            .map_err(|error| target(format!("failed to read --session-log: {error}")))?;
        if read == 0 {
            break;
        }
        line_number += 1;
        let parsed = std::str::from_utf8(&buffer)
            .map_err(|error| error.to_string())
            .and_then(|line| {
                if line.trim().is_empty() {
                    Ok(None)
                } else {
                    serde_json::from_str::<Value>(line)
                        .map(Some)
                        .map_err(|error| error.to_string())
                }
            });

        match parsed {
            Ok(None) => {}
            Ok(Some(value)) => {
                summary.parsed_lines += 1;
                emit(to_value(&InspectEvent {
                    line_number,
                    timestamp: find_string(&value, &["timestamp", "created_at", "createdAt"]),
                    event_type: find_event_type(&value),
                    tool_name: find_tool_name(&value),
                    usage: find_usage(&value),
                    payload: payload_info(&value),
                })?)?;
            }
            Err(error) => {
                if debug {
                    eprintln!("debug: skipped line {line_number}: {error}");
                }
                summary
                    .skipped_lines
                    .push(SkippedLine { line_number, error });
            }
        }
    }

    Ok(summary)
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

    fn run_inspect(log: &str) -> (Vec<Value>, Value) {
        let mut events = Vec::new();
        let summary = inspect(
            Cursor::new(log.as_bytes().to_vec()),
            &mut |value| {
                events.push(value);
                Ok(())
            },
            false,
        )
        .unwrap();
        (events, to_value(&summary).unwrap())
    }

    #[test]
    fn inspect_streams_events_and_summarizes_skipped_lines() {
        let log = concat!(
            r#"{"type":"assistant","timestamp":"2026-02-27T09:36:33Z","message":{"usage":{"output_tokens":45},"content":[{"type":"tool_use","name":"Read","input":"abc"}]}}"#,
            "\n\nnot json\n"
        );
        let (events, summary) = run_inspect(log);
        assert_eq!(
            events,
            [json!({
                "LineNumber": 1,
                "Timestamp": "2026-02-27T09:36:33Z",
                "EventType": "assistant",
                "ToolName": "Read",
                "Usage": {"output_tokens": 45},
                "Payload": {"Kind": "tool_use", "Chars": 3}
            })]
        );
        assert_eq!(
            summary,
            json!({
                "ParsedLines": 1,
                "SkippedLines": [{"LineNumber": 3, "Error": "expected ident at line 1 column 2"}]
            })
        );
    }

    #[test]
    fn inspect_counts_a_line_without_trailing_newline() {
        let (events, summary) = run_inspect("{\"type\":\"user\"}");
        assert_eq!(events.len(), 1);
        assert_eq!(summary["ParsedLines"], 1);
    }
}
