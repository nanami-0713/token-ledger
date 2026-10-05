# TokenLedger

Every model call on this machine, in one ledger — tokens, plan credits and
API dollars — whatever GUI or CLI made it.

![TokenLedger](docs/light.png)

## What it does

ZCode, DeepSeek Harness, Codex, Claude Code each keep their own usage logs,
in their own formats, under their own directories. TokenLedger reads them
all, normalizes every record to one shape, and merges the same model's
spelling across tools (`GLM-5.3` in ZCode and `glm-5.3` in DSH are one
bucket, one burn curve), so a subscription's quota is measured as it is
spent: across every tool at once.

Three ledgers side by side, on every view:

- **Tokens** — fresh input, cache reads, cache writes, output, reasoning.
- **Plan credits** — the GLM Coding Plan formula (flash/standard tiers,
  peak window Mon–Fri 14:00–18:00 Asia/Shanghai at full price, off-peak at
  half), coefficients configurable.
- **Money** — API list price per model in CNY (`\u{a5} / 1M tokens`), with
  BigModel's published GLM prices built in; a model without a card shows
  tokens only, never a guess.

## Run it

```sh
cargo run                # the window
cargo run -- --smoke     # the whole ledger, printed to stdout
cargo run -- --dark      # start dark
cargo run -- --page 2    # start on a page: 0 overview, 1 models, 2 sessions, 3 sources
```

No full Xcode needed; gpui compiles its Metal shaders at runtime.

## Data sources

Built-ins, discovered on launch, each toggleable on the Sources page:

| Source | Where it reads |
|---|---|
| ZCode | `~/.zcode/cli/db/db.sqlite`, table `model_usage` (per-request, billing-grade) |
| DeepSeek Harness | `~/.dsh/sessions/<workdir>/session-*/session.jsonl.zstd` |
| ChatGPT (Codex engine) | `~/.codex/sessions/**/*.jsonl` |
| Claude Code | `~/.claude/projects/**/*.jsonl` |

### Add any other tool

A source is a few lines in `~/.config/token-ledger/config.toml`. Point a
glob at any JSONL log, name the dotted field paths, and its calls join the
same ledgers as the built-ins:

```toml
[[source]]
id = "my-tool"
label = "My Tool"
kind = "jsonl"
paths = ["~/.mytool/sessions/*.jsonl"]
zstd = false
input_includes_cache = true   # true when the input number contains cache hits
model = "data.model"          # omitted fields are skipped per line
sticky_model = true           # reuse the last seen model (logs that state it once per turn)
time = "data.ts"
input = "data.usage.input"
output = "data.usage.output"
cache_read = "data.usage.cached"
cache_write = "data.usage.cache_write"
```

The same file carries the rest of the ledger:

```toml
[aliases]                     # spellings of one model, onto one key
kimi-k3 = "k3"

[credits]                     # plan formula, per 10k tokens
flash_in = 2.3
flash_cache = 0.56
flash_out = 8.0
std_in = 6.9
std_cache = 1.7
std_out = 24.0
divisor = 10000
offpeak_factor = 0.5

[prices."glm-5.3"]            # \u{a5} per 1M tokens; built-ins below, override freely
input = 8.0
cache_read = 2.0
output = 28.0
# currency = "usd"           # a USD card converts through fx_usd_cny
```

Built-in cards (BigModel list prices, CNY per million tokens; tiered models
take their long-context tier):

| Model | Input | Cache hit | Output |
|---|---|---|---|
| glm-5.3 | 8 | 2 | 28 |
| glm-5.3-flash | 0.8 | 0.23 | 2.8 |
| glm-5.3-flashx | 2 | 0.57 | 7 |
| glm-5.2 | 8 | 2 | 28 |
| glm-5.1 | 8 | 2 | 28 |
| glm-5 | 6 | 1.5 | 22 |
| glm-5-turbo | 7 | 1.8 | 26 |
| glm-4.7 | 4 | 0.8 | 16 |
| glm-4.7-flashx | 0.5 | 0.1 | 3 |
| glm-4.7-flash | free | free | free |

`fx_usd_cny = 7.2` (configurable) converts a `currency = "usd"` card into
the CNY ledger. DeepSeek and Kimi cards are one entry each, away.

An id that matches a built-in replaces it, so a built-in can be redirected
at a different path or turned off (`enabled = false`).

## Notes

- Same-id merging is spelling-only: `GLM-5.3`, `glm-5.3`,
  `bigmodel/glm-5.3` fold together; two different models never do. Unknown
  spellings get their own bucket until an alias maps them.
- ZCode's `input_tokens` is gross (cache reads inside); the ledger stores
  fresh input and cache reads separately for every source.
- A scan is a full re-read; press Rescan after heavy sessions. (Watching
  files live is on the way.)

Built with [Ely GPUI Components](https://github.com/ZacharyZhang-NY/Ely-GPUI-Components).
MIT or Apache-2.0, at your option.
