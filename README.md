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
- A value starting with `file://` is replaced by the content of that file. Read stdin with `file:///dev/stdin`.
- `--cli-input-json file://input.json` passes all parameters as a JSON object keyed by the PascalCase parameter names (`SessionLog`). Parameters given on the command line take precedence.
- `--generate-cli-skeleton input` prints the input skeleton and `--generate-cli-skeleton output` prints the output skeleton.
- Global options: `--output json|text|table` (default `json`), `--query <JMESPath>`, `--debug`, `--version`.
- `help` works at every level: `rust-inspect-jsonl help`, `rust-inspect-jsonl session help`, `rust-inspect-jsonl session inspect-session help`.
- Exit codes: `0` success, `252` syntax or parameter error, `254` the input cannot be processed (no line of `--session-log` is JSON), `255` other failures.

## Commands

### `session inspect-session`

```sh
rust-inspect-jsonl session inspect-session --session-log file://session.jsonl
```

Parameters:

- `--session-log`: content of the raw JSONL session log (use `file://`).

Output: one JSON object. `Events` holds one normalized analysis event per JSON line, `ParsedLines` counts them, and `SkippedLines` lists the lines that are not JSON with their parse error. `Usage` is copied from the log as is.

```json
{
  "ParsedLines": 1,
  "SkippedLines": [],
  "Events": [
    {
      "LineNumber": 1,
      "Timestamp": "2026-02-27T09:36:33Z",
      "EventType": "assistant",
      "ToolName": null,
      "Usage": {"input_tokens": 123, "output_tokens": 45, "cache_read_input_tokens": 1000},
      "Payload": {"Kind": "text", "Chars": 320}
    }
  ]
}
```

Write the events as JSONL for later processing with `jq`:

```sh
rust-inspect-jsonl session inspect-session --session-log file://session.jsonl | jq -c '.Events[]' > events.inspect.jsonl
```

## Verification

```sh
nix develop -c cargo fmt --check
nix develop -c cargo check
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo test
```
