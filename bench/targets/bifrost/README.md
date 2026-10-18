# Bifrost

[Bifrost](https://github.com/maximhq/bifrost)'s HTTP gateway, one of the comparison targets of
[the benchmark](../../../docs/benchmarks.md).

| File | What it is |
| --- | --- |
| `VERSION` | the release under test: `transports/v2.2.4` |
| `setup.sh` | downloads the Linux x86_64 release binary, checks its pinned SHA-256, unpacks the two snapshots below |
| `start.sh` | renders `config.template.json` into a fresh application directory and starts the gateway (one start per repetition) |
| `config.template.json` | the configuration, with `@MOCK@`, `@POOL@`, `@BUFFER@`, `@CONC@`, `@DIR@` and `@APP@` filled by `start.sh` |
| `datasheet.json.gz` | the pricing datasheet Bifrost loads at startup (`pricing_url`), a snapshot taken on 2026-10-04 |
| `model-parameters.json.gz` | the model parameters it loads at startup (`model_parameters_url`), same snapshot |
| `target.yaml` | its entry in the harness's interleaved targets file: nodes, key, models, start command, readiness |

The snapshots are read from local files so every run uses the same data and no startup depends on
a download (a failed fetch is fatal at startup). Their SHA-256 once unpacked:

```
7b941774128abb071c22b5cfe86c0d9c4a5dc8a37d76e83ec7bba80ff1049660  datasheet.json
ae8dbb51370de794a23f46c53ec420e63cfaa77de143a9aeec6e0d97adf29e07  model-parameters.json
```

## Configuration

- **Pool and queue:** `initial_pool_size` 15000 and a per-provider `buffer_size` of 20000 on 4 or more vCPUs
  (10000 and 15000 below), the values of Bifrost's own published t3 benchmark; per-provider
  `concurrency` 10000, above the requests in flight on the slowest mock profile (5,000 RPS × 1.5 s);
  `drop_excess_requests: false`.
- **Logging off:** `enable_logging: false`, `disable_content_logging: true`, `logs_store` disabled,
  log level `error`.
- **No cache:** no plugin is configured, so no response or semantic cache.
- **Authentication on:** `enforce_auth_on_inference: true` with one virtual key allowed only the
  upstream model `bench-upstream`, so every request is checked against a key, as Antirion's P-min does.
  An unknown model is refused with 400 (`statuses` in `target.yaml`).
- **Upstreams:** four custom providers on the OpenAI and Anthropic bases, one per mock latency
  profile (`/L0`, `/LS`, `/L1500`, and Anthropic on `/L0`), with `allow_private_network` (the mock
  is on a private address) and a 60 s request timeout.
- **State:** the configuration store is SQLite in a fresh directory per start (`BIFROST_WORK`).

## By hand

```sh
./setup.sh                                  # Linux x86_64
MOCK=10.0.0.20:19401 PORT=18080 ./start.sh  # the mock's host:port
```
