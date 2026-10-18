# Contributing to Antirion

Thanks for helping. This guide covers the development setup, the checks every pull request must
pass, and how changes get in. For the shape of the code (the crate map, where each concern lives and
the conventions it follows), read [docs/development.md](docs/development.md) first.

## Development setup

You need:

- **Rust**: the toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml); `rustup` picks
  it up on the first `cargo` command in the repository.
- **Node 22 or later and pnpm** for the console in `console/` (the pnpm version comes from
  `packageManager` in `console/package.json`).
- **Docker** (optional) for the Compose setups and the container image.

### Run the gateway

```bash
cargo build -p antirion
export ANTIRION_BOOTSTRAP_TOKEN=dev-bootstrap-token-0123456789abcdef0123
./target/debug/antirion serve --test-mode --config ci/standalone.yaml
```

`ci/standalone.yaml` listens on `127.0.0.1:18080` (the data plane) and `127.0.0.1:18081` (the admin
plane) and keeps its embedded store in `./dev-data`; delete that directory for a fresh node. Test
mode adds the test clock, seed and reset endpoints under `/admin/v1/test/`. Never use it in
production.

To send requests without a provider key, follow the no-cost option in
[docs/getting-started.md](docs/getting-started.md).

### Run the console

```bash
cd console
pnpm install --frozen-lockfile
ANTIRION_DEV_ADMIN_URL=http://127.0.0.1:18081 pnpm dev
```

Open <http://localhost:3000/ui/> and sign in with the bootstrap token. The dev server proxies
`/admin/v1` to the gateway's admin listener and reloads as you edit. `pnpm dev:mock` serves the
screens with contract mocks and demo data, without a gateway. [console/README.md](console/README.md)
describes the layout, the stores and what each screen reads.

To embed your console build in the binary:

```bash
(cd console && pnpm build)
ANTIRION_CONSOLE_DIR=$PWD/console/out cargo build -p antirion
```

## Checks

A pull request must pass the same checks CI runs (`.github/workflows/pr.yml`). Run them locally
before you push.

**Rust**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo xtask policy      # no open-work markers in the repository's text
cargo xtask layers      # crates only depend on lower layers
cargo xtask lints       # every crate inherits the workspace lints
cargo deny --locked check advisories bans licenses sources
cargo audit
```

`ci/advisory-ignores.sh` checks that every ignored advisory still has an unexpired date.

**Console** (in `console/`)

```bash
pnpm lint
pnpm format:check
pnpm typecheck
pnpm test
pnpm build
pnpm e2e        # Playwright against a test-mode gateway; see console/README.md
```

`pnpm e2e` starts `../target/debug/antirion` itself, so build the export and the gateway with it
embedded first (the commands are in [Run the console](#run-the-console)).

**When you touch them**

- The Admin API contract (`openapi/admin-api.yaml`): run `pnpm api:generate` in `console/` and
  commit the regenerated `src/lib/api/schema.d.ts` and `src/lib/api/operations.ts`.
- The Helm chart: `helm lint deploy/helm/antirion --set mode=standalone` and `--set mode=cluster`.
- The benchmark workspace in `bench/`: `cargo fmt`, `cargo clippy` and `cargo test` with
  `--manifest-path bench/Cargo.toml`.
- Parsers: `ci/fuzz.sh 30` runs every fuzz target briefly (it needs a nightly toolchain and
  `cargo-fuzz`).
- The npm launcher in `npm/`: `node --test` there (Node 18 or later).
- The image: `docker/test/smoke.sh <image>` and `docker/test/quickstart.sh <image>` after
  `docker build -f docker/Dockerfile`.

## Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org):

```
type(scope): subject in the imperative, at most 72 characters

A short body that says what changed and why.
```

- **type**: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `build`, `ci` or `chore`.
- **scope**: the crate name without `antirion-` (`router`, `pipeline`, `admin`, …), or `console`,
  `bench`, `helm`, `docker`, `workflows`, `readme`.

Examples from the history: `feat(console): link the Admin API reference from the account menu`,
`fix(admin): …`, `ci(workflows): place the tested binary from the artifact's root`.

## Branches and pull requests

1. Fork the repository (or create a branch if you have write access) and branch off `main`, with a
   short descriptive name such as `fix-stream-usage` or `console-budget-chart`.
2. Keep each pull request to one change. Add or update tests with the code, and update
   `docs/development.md` when you change a convention or move a concern.
3. Run the checks above, then open the pull request against `main`. Describe what changed, why, and
   how you tested it; link the issue it closes.
4. CI must be green and a maintainer must approve before the pull request is merged. The `pr`
   workflow also runs for merge-queue entries and on `main`, so the result is checked again.

The repository does not accept open-work markers in its text: finish the change, or describe what is
left in the pull request or an issue.

## Where to start

- [docs/development.md](docs/development.md): the crate map (layers 0 to 7), where a request goes
  through the data plane, and the conventions of the code.
- [docs/getting-started.md](docs/getting-started.md): running Antirion with Compose, Helm or from
  source.
- [console/README.md](console/README.md): the console's commands, layout and screens.
- The "Known gaps" sections of `docs/development.md` list what is not built yet.

## Releases

A `vX.Y.Z` tag on `main` runs `.github/workflows/release.yml`: it builds the binaries for Linux and
macOS on amd64 and arm64 with the console embedded, their SHA-256 list, SBOMs, signatures and
provenance, and the multi-arch image on `ghcr.io`, verifies them and publishes the GitHub Release.
It then starts `.github/workflows/publish.yml`, which copies the image to Docker Hub and publishes
the npm package `antirion`, whose version it sets from the tag.

`publish.yml` needs these repository secrets; without them, it skips that part with a notice:

| Secret               | For                                                                  |
| -------------------- | -------------------------------------------------------------------- |
| `DOCKERHUB_USERNAME` | the Docker Hub account that can push to `antirion/antirion`          |
| `DOCKERHUB_TOKEN`    | a Docker Hub access token of that account, with read and write scope |
| `NPM_TOKEN`          | an npm automation (or granular, publish) token for the package       |

To publish an existing release again, run the `publish` workflow by hand with its tag; what is
already published is left as it is.

## Reporting security issues

Do not open a public issue for a vulnerability. Report it privately as described in
[SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions are licensed under the
[Apache License, Version 2.0](LICENSE).
