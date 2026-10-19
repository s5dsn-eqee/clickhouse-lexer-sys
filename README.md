# clickhouse-lexer-sys

[![CI](https://github.com/s5dsn-eqee/clickhouse-lexer-sys/actions/workflows/ci.yml/badge.svg)](https://github.com/s5dsn-eqee/clickhouse-lexer-sys/actions/workflows/ci.yml)
[![Sync](https://github.com/s5dsn-eqee/clickhouse-lexer-sys/actions/workflows/sync.yml/badge.svg)](https://github.com/s5dsn-eqee/clickhouse-lexer-sys/actions/workflows/sync.yml)
[![crates.io](https://img.shields.io/crates/v/clickhouse-lexer-sys.svg)](https://crates.io/crates/clickhouse-lexer-sys)
[![docs.rs](https://docs.rs/clickhouse-lexer-sys/badge.svg)](https://docs.rs/clickhouse-lexer-sys)

Rust bindings to the **actual SQL lexer of ClickHouse**
([`src/Parsers/Lexer.cpp`](https://github.com/ClickHouse/ClickHouse/blob/master/src/Parsers/Lexer.cpp)).
The sources are copied unchanged from
[ClickHouse/clickhouse-lexer](https://github.com/ClickHouse/clickhouse-lexer), the
standalone build ClickHouse maintains for its web UI, and compiled natively at build
time. `TokenKind` is generated from the same `Lexer.h`, so the token numbering can
never drift from the compiled lexer.

Requires `clang++` (ClickHouse only supports clang; set `CXX` to point at it if it
is not on `PATH`). There is no C++ runtime dependency.

Besides the raw bindings in `ffi`, the crate ships a minimal safe wrapper, `Lexer`,
so a separate wrapper crate is not needed for the common case.

## Usage

```rust
use clickhouse_lexer_sys::{Lexer, TokenKind};

let tokens: Vec<_> = Lexer::new("SELECT count() FROM `events` WHERE user != 'guest' -- filter")
    .filter(|t| t.kind.is_significant())
    .collect();

assert_eq!(tokens[0].kind, TokenKind::BareWord);
assert_eq!(tokens[0].text, b"SELECT");
assert_eq!(tokens.last().unwrap().kind, TokenKind::StringLiteral);

// The lexer stops at unlexable input and reports what went wrong:
let last = Lexer::new("SELECT 'unterminated").last().unwrap();
assert_eq!(last.kind, TokenKind::ErrorSingleQuoteIsNotClosed);
```

Each `Token` is `{kind, begin, end, text}`. Tokens tile the input from the start:
every byte up to the end of the last token belongs to exactly one token, whitespace
and comments included. `begin` and `end` are byte offsets, `text` is the covered
bytes, and `text_str()` decodes it when it is valid UTF-8. Iteration ends after the
first error token, which is yielded, or at the end of the input, where
`TokenKind::EndOfStream` is not yielded; `Lexer::next_token` exposes the raw
stream including `EndOfStream`.

| Item | Description |
|---|---|
| `Lexer::new(input)` | Lex a `&str` or `&[u8]`. |
| `Lexer::with_max_query_size(input, n)` | Stop with `ErrorMaxQuerySizeExceeded` past `n` bytes. |
| `TokenKind` | The upstream `TokenType` enum, same names, same order. |
| `TokenKind::{is_significant, is_error, is_end}` | The upstream predicates. |
| `TokenKind::{from_raw, name, ALL}` | Conversions for the raw C API. |
| `UPSTREAM_COMMIT`, `UPSTREAM_VERSION` | The ClickHouse commit and clickhouse-lexer release the sources come from. |
| `ffi` | The raw C API from `upstream/clickhouse_lexer.h`. |

`TokenKind` is `#[non_exhaustive]`: upstream adds token types from time to time, and
a sync then lands in a patch release.

## How this repository is synced

[`upstream/`](upstream/) holds the lexer sources verbatim from the `upstream/`
directory of a clickhouse-lexer release, which in turn mirrors `src/Parsers` of
ClickHouse. `upstream/VERSION` names that release and `upstream/COMMIT` the
ClickHouse commit it was taken from.

The [Sync workflow](.github/workflows/sync.yml) runs weekly:

1. `scripts/sync.sh` copies `upstream/` from the latest clickhouse-lexer release and
   stops if nothing changed.
2. It bumps the patch version, runs `cargo test`, and opens a pull request.

[CI](.github/workflows/ci.yml) checks Linux and macOS builds, the supported Rust and direct dependency minima, formatting, lints, and docs. It also checks that `upstream/` is byte-identical to the recorded release and passes the upstream test suite.

To release after merging a sync pull request, push the matching tag:

```sh
git tag "v$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')"
git push --tags
```

The [Release workflow](.github/workflows/release.yml) checks that the tag matches
`Cargo.toml` and sits on `main`, runs CI, publishes to crates.io through
[trusted publishing](https://crates.io/docs/trusted-publishing), and creates the
GitHub release. One-time setup: publish the first version by hand with
`cargo publish`, then on the crate's crates.io settings page add a trusted
publisher for this repository with workflow `release.yml` and environment `release`.

## License

[Apache License 2.0](https://github.com/s5dsn-eqee/clickhouse-lexer-sys/blob/main/LICENSE), same as ClickHouse. The `upstream/` sources are
verbatim copies from the ClickHouse repository and remain © ClickHouse, Inc.
