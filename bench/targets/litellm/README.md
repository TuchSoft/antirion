# LiteLLM

[LiteLLM](https://github.com/BerriAI/litellm)'s proxy server, one of the comparison targets of
[the benchmark](../../../docs/benchmarks.md).

| File | What it is |
| --- | --- |
| `VERSION` | the release under test: `1.103.1` |
| `setup.sh` | installs `litellm[proxy]==<VERSION>` into a uv virtual environment (Python 3.12) next to it |
| `start.sh` | renders `config.template.yaml` into a fresh directory and starts the proxy (one start per repetition) |
| `config.template.yaml` | the configuration, with `@MOCK@` filled by `start.sh` |
| `target.yaml` | its entry in the harness's interleaved targets file: nodes, key, models, start command, readiness |

## Configuration

- **Workers:** `--num_workers` is the machine's vCPU count (4 on the reference c7i.xlarge). No
  connection-pool setting is given: the proxy's own defaults apply.
- **Logging off:** no database, so no spend or request logs (`disable_spend_logs` and
  `disable_error_logs` are set as well); `set_verbose: false`, `LITELLM_LOG=ERROR`, JSON logs.
- **No cache, no callbacks:** neither is configured.
- **Authentication on:** the master key is the only key and every request carries it (virtual keys
  need a database). An unknown key and an unknown model are refused with 400 (`statuses` in
  `target.yaml`).
- **Upstreams:** one model per mock latency profile (`bench-chat` on `/L0`, `bench-chat-ls` on
  `/LS`, `bench-chat-l1500` on `/L1500`, `bench-embed` on `/L0`, `bench-messages` on the Anthropic
  `/L0`), `num_retries: 0`, a 60 s request timeout, `drop_params: true`.
- **No startup fetch, no telemetry:** `LITELLM_LOCAL_MODEL_COST_MAP=True`,
  `LITELLM_TELEMETRY=False`; the admin UI and the API docs are off.

## By hand

```sh
./setup.sh
MOCK=10.0.0.20:19401 PORT=18090 ./start.sh   # the mock's host:port
```
