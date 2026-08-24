# rust-inspect-jsonl

`rust-inspect-jsonl` inspects Claude/Codex-style JSONL session logs.

The first supported mode is a pipeline-friendly CLI that reads JSONL from stdin and writes normalized JSONL to stdout.

```sh
cat session.jsonl \
  | rust-inspect-jsonl inspect --input-jsonl --output-jsonl \
  > events.inspect.jsonl
```

## Install and develop

```sh
nix develop
cargo run -- inspect --input-jsonl --output-jsonl < session.jsonl
```

## Commands

### `inspect`

```sh
rust-inspect-jsonl inspect --input-jsonl --output-jsonl < session.jsonl
```

Input:

- stdin: raw JSONL session log

Output:

- stdout: normalized analysis events as JSONL
- stderr: warnings and parse statistics

Example output:

```json
{"line_number":1,"timestamp":"2026-02-27T09:36:33Z","event_type":"assistant","tool_name":null,"usage":{"input_tokens":123,"output_tokens":45,"cache_read_input_tokens":1000},"payload":{"kind":"text","chars":320}}
```

## Verification

```sh
nix develop -c cargo fmt --check
nix develop -c cargo check
nix develop -c cargo clippy -- -D warnings
```
