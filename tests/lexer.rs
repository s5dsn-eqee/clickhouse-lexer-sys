//! The first section mirrors clickhouse-lexer's `test/test.js` case by case, with the
//! same names and inputs, so the two suites stay comparable. The second section covers
//! what only exists on the Rust side: the generated `TokenKind`, the iterator contract
//! and the byte-oriented API.

use clickhouse_lexer_sys::{ffi, Lexer, Token, TokenKind, UPSTREAM_COMMIT, UPSTREAM_VERSION};

fn names_and_text<'a>(tokens: &[Token<'a>]) -> Vec<(&'static str, &'a str)> {
    tokens
        .iter()
        .map(|t| (t.kind.name(), t.text_str().unwrap()))
        .collect()
}

// ---- Mirrors of test/test.js ----------------------------------------------------

#[test]
fn token_table_looks_sane() {
    assert_eq!(TokenKind::Whitespace as u8, 0);
    assert_eq!(TokenKind::Comment as u8, 1);
    assert_eq!(TokenKind::BareWord as u8, 2);
    assert!(TokenKind::EndOfStream > TokenKind::DoubleAt);
    assert!(TokenKind::Error > TokenKind::EndOfStream);
    assert_eq!(TokenKind::StringLiteral.name(), "StringLiteral");
    assert_eq!(TokenKind::StringLiteral.to_string(), "StringLiteral");
}

#[test]
fn tokenizes_a_basic_query() {
    let tokens: Vec<_> = Lexer::new("SELECT 1 AS x FROM `t` WHERE s = 'v' -- c").collect();
    let significant: Vec<_> = tokens
        .iter()
        .copied()
        .filter(|t| t.kind.is_significant())
        .collect();
    assert_eq!(
        names_and_text(&significant),
        [
            ("BareWord", "SELECT"),
            ("Number", "1"),
            ("BareWord", "AS"),
            ("BareWord", "x"),
            ("BareWord", "FROM"),
            ("QuotedIdentifier", "`t`"),
            ("BareWord", "WHERE"),
            ("BareWord", "s"),
            ("Equals", "="),
            ("StringLiteral", "'v'"),
        ]
    );
    assert_eq!(tokens.last().unwrap().kind, TokenKind::Comment);
}

#[test]
fn tokens_tile_the_input_with_byte_offsets() {
    let query = "SELECT 'привет', [1, 2], f(x)\n-- комментарий";
    let mut offset = 0;
    for token in Lexer::new(query) {
        assert_eq!(token.begin, offset);
        assert_eq!(token.text, &query.as_bytes()[token.begin..token.end]);
        offset = token.end;
    }
    assert_eq!(offset, query.len());
}

#[test]
fn reports_an_error_token_for_unlexable_input() {
    let tokens: Vec<_> = Lexer::new("SELECT 'unterminated").collect();
    let last = tokens.last().unwrap();
    assert!(last.kind.is_error());
    assert_eq!(last.kind, TokenKind::ErrorSingleQuoteIsNotClosed);
    assert_eq!(last.text, b"'unterminated");
}

#[test]
fn respects_max_query_size() {
    let tokens: Vec<_> = Lexer::with_max_query_size("SELECT 11111", 3).collect();
    assert_eq!(
        tokens.last().unwrap().kind,
        TokenKind::ErrorMaxQuerySizeExceeded
    );
}

#[test]
fn handles_queries_larger_than_the_initial_memory() {
    let query = "SELECT 1 + 2;\n".repeat(20000);
    let tokens: Vec<_> = Lexer::new(&query).collect();
    assert!(!tokens.last().unwrap().kind.is_error());
    assert_eq!(tokens.last().unwrap().end, query.len());
}

#[test]
fn operators_get_distinct_types() {
    let ops: Vec<_> = Lexer::new("a || b -> c :: d")
        .filter(|t| t.kind.is_significant() && t.kind != TokenKind::BareWord)
        .map(|t| t.kind)
        .collect();
    assert_eq!(
        ops,
        [
            TokenKind::Concatenation,
            TokenKind::Arrow,
            TokenKind::DoubleColon
        ]
    );
}

// ---- Rust API ------------------------------------------------------------------

#[test]
fn token_table_matches_the_header() {
    let header = include_str!("../upstream/Lexer.h");
    let macro_body: Vec<&str> = header
        .lines()
        .skip_while(|l| !l.contains("#define APPLY_FOR_TOKENS"))
        .skip(1)
        .take_while(|l| l.trim_end().ends_with('\\'))
        .collect();
    let names: Vec<&str> = macro_body
        .iter()
        .filter_map(|l| l.trim().strip_prefix("M("))
        .map(|rest| rest.split(')').next().unwrap())
        .collect();
    assert_eq!(
        names,
        TokenKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>()
    );
    for (i, kind) in TokenKind::ALL.iter().enumerate() {
        assert_eq!(*kind as usize, i);
        assert_eq!(TokenKind::from_raw(i as u8), Some(*kind));
    }
    assert_eq!(TokenKind::from_raw(TokenKind::ALL.len() as u8), None);
    assert_eq!(TokenKind::from_raw(u8::MAX), None);
}

#[test]
fn predicates_match_the_c_api() {
    for kind in TokenKind::ALL {
        let raw = *kind as u8;
        // SAFETY: pure functions over a byte.
        let (significant, error, end) = unsafe {
            (
                ffi::clickhouse_lexer_token_is_significant(raw) != 0,
                ffi::clickhouse_lexer_token_is_error(raw) != 0,
                ffi::clickhouse_lexer_token_is_end(raw) != 0,
            )
        };
        assert_eq!(kind.is_significant(), significant, "{kind}");
        assert_eq!(kind.is_error(), error, "{kind}");
        assert_eq!(kind.is_end(), end, "{kind}");
    }
}

#[test]
fn upstream_provenance_is_recorded() {
    assert_eq!(UPSTREAM_COMMIT.len(), 40);
    assert!(UPSTREAM_COMMIT.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(UPSTREAM_COMMIT, include_str!("../upstream/COMMIT").trim());
    assert!(UPSTREAM_VERSION.starts_with('v'));
    assert_eq!(UPSTREAM_VERSION, include_str!("../upstream/VERSION").trim());
}

#[test]
fn stops_after_the_error_token() {
    let mut lexer = Lexer::new("SELECT 'unterminated");
    assert_eq!(lexer.next().unwrap().kind, TokenKind::BareWord);
    assert_eq!(lexer.next().unwrap().kind, TokenKind::Whitespace);
    assert_eq!(
        lexer.next().unwrap().kind,
        TokenKind::ErrorSingleQuoteIsNotClosed
    );
    assert_eq!(lexer.next(), None);
    assert_eq!(lexer.next(), None);
}

#[test]
fn clickhouse_specific_syntax() {
    let significant: Vec<_> = Lexer::new("SELECT 0x1F /* a /* nested */ b */ FROM t |> WHERE x\\G")
        .filter(|t| t.kind.is_significant())
        .collect();
    assert_eq!(
        names_and_text(&significant),
        [
            ("BareWord", "SELECT"),
            ("Number", "0x1F"),
            ("BareWord", "FROM"),
            ("BareWord", "t"),
            ("PipeOperator", "|>"),
            ("BareWord", "WHERE"),
            ("BareWord", "x"),
            ("VerticalDelimiter", "\\G"),
        ]
    );
}

#[test]
fn dollar_sign_forms() {
    let kinds = |sql: &str| Lexer::new(sql).map(|t| t.kind).collect::<Vec<_>>();
    assert_eq!(kinds("$"), [TokenKind::DollarSign]);
    assert_eq!(kinds("$$"), [TokenKind::DollarSign, TokenKind::DollarSign]);
    assert_eq!(kinds("$x"), [TokenKind::BareWord]);
    assert_eq!(kinds("$tag$text$tag$"), [TokenKind::HereDoc]);
}

#[test]
fn empty_input_yields_nothing() {
    assert_eq!(Lexer::new("").next(), None);
    assert_eq!(Lexer::new(b"").next_token().kind, TokenKind::EndOfStream);
}

#[test]
fn next_token_exposes_end_of_stream() {
    let mut lexer = Lexer::new("1");
    assert_eq!(lexer.next_token().kind, TokenKind::Number);
    let end = lexer.next_token();
    assert_eq!(end.kind, TokenKind::EndOfStream);
    assert_eq!((end.begin, end.end), (1, 1));
    assert_eq!(lexer.next_token().kind, TokenKind::EndOfStream);
}

#[test]
fn next_token_continues_after_an_error() {
    let mut lexer = Lexer::new("!1");
    for (kind, begin, end) in [
        (TokenKind::ErrorSingleExclamationMark, 0, 1),
        (TokenKind::Number, 1, 2),
        (TokenKind::EndOfStream, 2, 2),
    ] {
        let token = lexer.next_token();
        assert_eq!((token.kind, token.begin, token.end), (kind, begin, end));
    }
}

#[test]
fn next_token_repeats_query_size_errors() {
    let mut lexer = Lexer::with_max_query_size("123", 1);
    let first = lexer.next_token();
    assert_eq!(first.kind, TokenKind::ErrorMaxQuerySizeExceeded);
    assert_eq!((first.begin, first.end), (0, 3));
    for _ in 0..2 {
        let repeated = lexer.next_token();
        assert_eq!(repeated.kind, first.kind);
        assert_eq!((repeated.begin, repeated.end), (3, 3));
    }
}

#[test]
fn accepts_bytes_that_are_not_utf8() {
    let tokens: Vec<_> = Lexer::new(b"SELECT '\xff'").collect();
    let literal = tokens.last().unwrap();
    assert_eq!(literal.kind, TokenKind::StringLiteral);
    assert!(literal.text_str().is_err());
}

#[test]
fn lexer_is_send() {
    fn assert_send<T: Send>(_: &T) {}
    let input = String::from("SELECT 1");
    assert_send(&Lexer::new(&input));
}
