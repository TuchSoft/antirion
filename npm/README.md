# antirion

Run [Antirion](https://github.com/eliasbiondo/antirion), an AI gateway with its web console built
in, on your machine with one command:

```bash
npx -y antirion
```

The first start prints where everything is, and the admin token, shown that once:

```text
Antirion 0.1.0 is running.

  Console      http://localhost:8081/ui/
  Admin token  ak-antirion-…
               Shown on this first start only; it is kept in
               /home/you/.antirion/bootstrap-token
  Data plane   http://localhost:8080/v1
  Data dir     /home/you/.antirion
```

Open the console, sign in with the admin token, then add a provider (OpenAI, Anthropic and many
more), a model and an API key. Point any OpenAI SDK at `http://localhost:8080/v1`, or an Anthropic
SDK at `http://localhost:8080`, with that key.

## How it works

This package is a small launcher with no dependencies. On the first run it downloads the Antirion
binary of the package's version for your platform from the
[GitHub release](https://github.com/eliasbiondo/antirion/releases), checks its SHA-256 against the
checksums published with the package, and keeps it in `~/.cache/antirion/<version>/`
(`$XDG_CACHE_HOME/antirion` when that is set). Later runs start the cached binary at once.

It runs on Linux with glibc 2.36 or later (Debian 12, Ubuntu 24.04 and newer) and on macOS, on x64
and arm64, with Node.js 18 or later. Elsewhere, run the container image:

```bash
docker run -p 8080:8080 -p 8081:8081 eliasbiondo/antirion
```

## Options

Arguments go to the `antirion` binary: `npx -y antirion --help` lists them. With none, it runs a
standalone gateway with no config file:

| Flag               | Variable                  | Default                                        |
| ------------------ | ------------------------- | ---------------------------------------------- |
| `--data-dir`       | `ANTIRION_DATA_DIR`       | `$XDG_DATA_HOME/antirion`, else `~/.antirion`  |
| `--host`           | `ANTIRION_HOST`           | `127.0.0.1`                                    |
| `--data-port`      | `ANTIRION_DATA_PORT`      | `8080`                                         |
| `--admin-port`     | `ANTIRION_ADMIN_PORT`     | `8081`                                         |
| `--public-origin`  | `ANTIRION_PUBLIC_ORIGIN`  | `http://localhost:<admin port>`                |
| `--data-plane-url` | `ANTIRION_DATA_PLANE_URL` | `http://localhost:<data port>`                 |

`ANTIRION_BOOTSTRAP_TOKEN` and `ANTIRION_ENCRYPTION_KEY` (the base64 of 32 bytes) replace the token
and the key generated into the data directory. `antirion serve --config <file>` runs from a config
file instead.

The launcher itself reads:

- `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` for the download;
- `ANTIRION_CACHE_DIR`, where the binaries are kept;
- `ANTIRION_DOWNLOAD_URL`, a mirror of the release downloads (`<url>/v<version>/<file>`);
- `ANTIRION_BINARY`, a binary to run instead of downloading one.

## License

Apache-2.0. Copyright 2026 Elias Biondo.
