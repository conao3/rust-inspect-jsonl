# rust-inspect-jsonl

`rust-inspect-jsonl` inspects Claude/Codex-style JSONL session logs.

The CLI takes input in the AWS CLI grammar: `rust-inspect-jsonl [global options] <command> <subcommand> [--parameter value ...]`.

```sh
rust-inspect-jsonl session inspect-session --session-log file://session.jsonl
cat session.jsonl | rust-inspect-jsonl session inspect-session --session-log file:///dev/stdin
```

## Install and develop

```sh
nix develop
cargo run -- session inspect-session --session-log file://session.jsonl
```

## Grammar

- Every parameter is a named `--kebab-case` option. There are no positional arguments.
- A value starting with `file://` names a file to read. Read stdin with `file:///dev/stdin`. `--session-log` reads its file line by line, so memory use stays flat for any log size; any other value of `--session-log` is the log itself.
- Global options: `--output json|text|table` (default `json`), `--debug` (also reports each skipped line on stderr), `--version`.
- `help` works at every level: `rust-inspect-jsonl help`, `rust-inspect-jsonl session help`, `rust-inspect-jsonl session inspect-session help`.
- Exit codes: `0` success, `252` syntax or parameter error (including a `file://` path that cannot be opened), `254` the input cannot be processed (no non-empty line of `--session-log` is JSON), `255` other failures.

### Streaming exception

`session inspect-session` streams its result instead of printing one JSON document at the end:

- `--output json` writes NDJSON: one compact JSON object per parsed event as soon as its line is read, then one summary line.
- `--output text` and `--output table` render each line on its own.
- With exit code `254` the summary line is still written before the error.

## Commands

### `session inspect-session`

```sh
rust-inspect-jsonl session inspect-session --session-log file://session.jsonl
```

Parameters:

- `--session-log`: the raw JSONL session log (use `file://<path>` or `file:///dev/stdin`).

Output: one event line per JSON line of the log, then a summary line. `Usage` is copied from the log as is. `SkippedLines` lists the lines that are not JSON (or not UTF-8) with their parse error.

```json
{"LineNumber":1,"Timestamp":"2026-02-27T09:36:33Z","EventType":"assistant","ToolName":null,"Usage":{"input_tokens":123,"output_tokens":45,"cache_read_input_tokens":1000},"Payload":{"Kind":"text","Chars":320}}
{"ParsedLines":1,"SkippedLines":[]}
```

Keep only the events:

```sh
rust-inspect-jsonl session inspect-session --session-log file://session.jsonl | jq -c 'select(has("LineNumber"))' > events.inspect.jsonl
```

## Verification

```sh
nix develop -c cargo fmt --check
nix develop -c cargo check
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo test
```
