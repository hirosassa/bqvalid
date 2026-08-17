# bqvalid

[![build](https://github.com/hirosassa/bqvalid/actions/workflows/test.yaml/badge.svg)](https://github.com/hirosassa/bqvalid/actions/workflows/test.yaml)
[![codecov](https://codecov.io/gh/hirosassa/bqvalid/branch/main/graph/badge.svg?token=Q5FIA58YTN)](https://codecov.io/gh/hirosassa/bqvalid)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/hirosassa/bqvalid/blob/main/LICENSE)

## What bqvalid does

`bqvalid` is a SQL linter tool for BigQuery GoogleSQL (formerly known as StandardSQL).
`bqvalid` fails with error message if there are the violation of rules described in the [rules page](https://github.com/hirosassa/bqvalid/blob/main/docs/rules.md).

## Installation

Download the archive for your platform from the [release page](https://github.com/hirosassa/bqvalid/releases) and extract the `bqvalid` binary. The assets are named `bqvalid-<arch>-<os>`:

- `bqvalid-x86_64-linux.tar.gz`
- `bqvalid-arm64-darwin.tar.gz` (Apple Silicon)

For example, on Apple Silicon:

```shell
curl -LO https://github.com/hirosassa/bqvalid/releases/latest/download/bqvalid-arm64-darwin.tar.gz
tar xzf bqvalid-arm64-darwin.tar.gz
./bqvalid --help
```

Both archives bundle a `libguest_ffi.{so,dylib}` shared library next to the binary; keep the two together (the binary loads it from its own directory). Only `x86_64` Linux and Apple Silicon are supported, since googlesql only prebuilds the `libguest_ffi` sidecar for those targets.

## Usage

```shell
cat sample.sql | bqvalid
```

If the SQL is contained the expressions that comparing `_TABLE_SUFFIX` with subquery, `bqvalid` outputs the reason and its position like:
```
5:7: Full scan will cause! Should not compare _TABLE_SUFFIX with subquery
```

Also, you can input file paths or directory. `bqvalid` collects files whose extension is `.sql` (ignores files that has other extensions) :

```shell
bqvalid one.sql two.sql three.sql
```

or
```shell
bqvalid sql/
```

Then, the output will as follows:
```
one.sql:6:6: Full scan will cause! Should not compare _TABLE_SUFFIX with subquery
three.sql:5:19: Full scan will cause! Should not compare _TABLE_SUFFIX with subquery
```

### Output formats

By default `bqvalid` prints the human-readable format shown above. Use `--format`
to emit machine-readable output for CI and editor integrations:

```shell
bqvalid --format json sql/    # a single JSON document with a flat `diagnostics` array
bqvalid --format sarif sql/   # SARIF 2.1.0, e.g. for GitHub code scanning
```

Lint diagnostics go to stdout; the tool's own errors (unreadable files, parse
failures) go to stderr, so either format can be piped cleanly.

### Ignoring rules

Every rule is enabled by default. You can suppress individual rules by their
stable rule ID (listed on the [rules page](https://github.com/hirosassa/bqvalid/blob/main/docs/rules.md))
either on the command line or via a config file.

On the command line, pass `--ignore` with a rule ID. To ignore more than one
rule, give a comma-separated list or repeat the flag:

```shell
# ignore a single rule
bqvalid --ignore use_current_date sql/

# ignore multiple rules, comma-separated
bqvalid --ignore use_current_date,unnecessary_order_by sql/

# equivalently, by repeating the flag
bqvalid --ignore use_current_date --ignore unnecessary_order_by sql/
```

Or put the ignore list in a `bqvalid.toml` file:

```toml
# bqvalid.toml
ignore = ["use_current_date", "unnecessary_order_by"]
```

`bqvalid` looks for `bqvalid.toml` in the current directory and walks up to the
git repository root (the directory containing `.git`), using the nearest one it
finds; outside a git repository only the current directory is checked. Point at
a specific file with `--config`:

```shell
bqvalid --config path/to/bqvalid.toml sql/
```

When `--ignore` is given on the command line it replaces (does not merge with)
the `ignore` list from the config file. An unknown rule ID is reported as a
warning on stderr rather than silently ignored.

## Using in CI (GitHub Actions)

To run `bqvalid` in GitHub Actions, use the [`setup-bqvalid`](https://github.com/hirosassa/setup-bqvalid)
action to install the binary and add it to `PATH`:

```yaml
- uses: hirosassa/setup-bqvalid@v1
- run: bqvalid path/to/queries/
```

To pin a specific version, pass it via the `version` input (it defaults to
`latest`):

```yaml
- uses: hirosassa/setup-bqvalid@v1
  with:
    version: 0.3.0
- run: bqvalid path/to/queries/
```

For example, a workflow that lints all SQL files under `sql/` on every pull
request looks like:

```yaml
name: bqvalid

on: pull_request

jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: hirosassa/setup-bqvalid@v1
      - run: bqvalid sql/
```

`bqvalid` exits with a non-zero status when it finds a violation, so the step
(and the job) fails automatically.

To surface diagnostics in GitHub code scanning instead of failing the job, emit
SARIF and upload it. Because `bqvalid` returns a non-zero status on violations,
the linting step would otherwise fail and skip the upload; set
`continue-on-error: true` on it so the run always proceeds to the upload, and
let code scanning report the findings:

```yaml
      - uses: hirosassa/setup-bqvalid@v1
      - run: bqvalid --format sarif sql/ > bqvalid.sarif
        continue-on-error: true
      - uses: github/codeql-action/upload-sarif@v3
        if: always()
        with:
          sarif_file: bqvalid.sarif
```

## Linting Rules

See the [rules page](https://github.com/hirosassa/bqvalid/blob/main/docs/rules.md)


## Contributing

See the [contributing guide](https://github.com/hirosassa/bqvalid/blob/main/docs/contribute.md)!
