//! Compiles `upstream/Lexer.cpp` and generates `TokenKind` from `upstream/Lexer.h`.
//!
//! The C API returns a token's type as its position in the `APPLY_FOR_TOKENS` macro
//! of `Lexer.h`. Inserting a token upstream renumbers everything after it, so the
//! enum is always generated from the same header that is compiled.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const UPSTREAM: &str = "upstream";
const SOURCES: [&str; 4] = [
    "Lexer.cpp",
    "Lexer.h",
    "LexerStandalone.h",
    "clickhouse_lexer.h",
];

fn main() {
    let upstream = Path::new(UPSTREAM);
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    println!("cargo:rerun-if-changed={UPSTREAM}");

    compile(upstream, &out_dir);
    generate_token_kind(upstream, &out_dir);
}

/// The sources include each other as `<Parsers/...>`, so mirror them under that name.
fn stage_sources(upstream: &Path, out_dir: &Path) -> PathBuf {
    let include_root = out_dir.join("include");
    let parsers = include_root.join("Parsers");
    fs::create_dir_all(&parsers).expect("create include dir");
    for name in SOURCES {
        fs::copy(upstream.join(name), parsers.join(name))
            .unwrap_or_else(|err| panic!("copy {name}: {err}"));
    }
    include_root
}

fn compile(upstream: &Path, out_dir: &Path) {
    let include_root = stage_sources(upstream, out_dir);
    let mut build = cc::Build::new();
    // The flags mirror upstream's scripts/build.sh for the WebAssembly target.
    build
        .cpp(true)
        // Lexer.cpp uses C++20 features (fold expressions, char8_t).
        .std("c++20")
        // -I <out_dir>/include, so that `#include <Parsers/Lexer.h>` resolves.
        .include(&include_root)
        .file(include_root.join("Parsers").join("Lexer.cpp"))
        // Use LexerStandalone.h instead of ClickHouse's internal headers.
        .define("LEXER_STANDALONE_BUILD", None)
        // The lexer never throws and uses no dynamic_cast or typeid.
        .flag("-fno-exceptions")
        .flag("-fno-rtti")
        // The standalone build has no dependencies, so no C++ runtime is needed.
        .cpp_link_stdlib(None)
        // Upstream's warnings would otherwise print on every dependent build.
        .warnings(false);
    // ClickHouse supports only clang; GCC rejects LexerStandalone.h's `namespace std`.
    if !build.get_compiler().is_like_clang() {
        build.compiler("clang++");
    }
    if let Err(err) = build.try_compile("clickhouse_lexer") {
        panic!(
            "failed to compile the ClickHouse lexer: {err}\n\
             clickhouse-lexer-sys needs clang++ (set CXX to point at it)"
        );
    }
}

/// One `M(Name)` entry of the `APPLY_FOR_TOKENS` macro, i.e. one `TokenType` variant.
struct TokenType {
    name: String,
    doc: Vec<String>,
}

/// Reads the `TokenType` variants out of `Lexer.h`, in declaration order, with the
/// `/** */` comments upstream attaches to some of them.
fn parse_token_types(lexer_h: &str) -> Vec<TokenType> {
    let mut lines = lexer_h
        .lines()
        .skip_while(|l| !l.contains("#define APPLY_FOR_TOKENS"));
    assert!(
        lines.next().is_some(),
        "APPLY_FOR_TOKENS not found in Lexer.h"
    );

    let mut entries = Vec::new();
    let mut pending_doc = Vec::new();
    for line in lines {
        let body = line.trim().trim_end_matches('\\').trim();
        if let Some(name) = body
            .strip_prefix("M(")
            .and_then(|rest| rest.split_once(')'))
        {
            let (name, trailing) = name;
            let mut doc = std::mem::take(&mut pending_doc);
            doc.extend(doc_comment(trailing));
            entries.push(TokenType {
                name: name.to_string(),
                doc,
            });
        } else {
            pending_doc.extend(doc_comment(body));
        }
        if !line.trim_end().ends_with('\\') {
            break;
        }
    }
    assert!(
        entries.len() >= 30 && entries.iter().any(|e| e.name == "EndOfStream"),
        "parsed a suspicious token list ({} entries) from Lexer.h",
        entries.len()
    );
    entries
}

/// The text of a `/** ... */` comment from the macro, if `fragment` is one.
fn doc_comment(fragment: &str) -> Option<String> {
    let text = fragment
        .trim()
        .strip_prefix("/**")?
        .strip_suffix("*/")?
        .trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Writes `generated.rs`: `TokenKind`, mirroring `TokenType` of `Lexer.h` variant by
/// variant, plus the provenance constants from `COMMIT` and `VERSION`.
fn generate_token_kind(upstream: &Path, out_dir: &Path) {
    let read = |name: &str| {
        fs::read_to_string(upstream.join(name)).unwrap_or_else(|err| panic!("read {name}: {err}"))
    };
    let tokens = parse_token_types(&read("Lexer.h"));
    let commit = read("COMMIT").trim().to_string();
    let version = read("VERSION").trim().to_string();

    let mut out = String::new();
    out.push_str("// GENERATED by build.rs from upstream/Lexer.h - do not edit.\n\n");
    out.push_str(&format!(
        "/// The ClickHouse/ClickHouse commit the vendored lexer sources come from.\n\
         pub const UPSTREAM_COMMIT: &str = \"{commit}\";\n\n\
         /// The ClickHouse/clickhouse-lexer release the vendored sources were copied from.\n\
         pub const UPSTREAM_VERSION: &str = \"{version}\";\n\n"
    ));

    out.push_str(
        "/// The `TokenType` enum of ClickHouse's `src/Parsers/Lexer.h`, in the same order.\n\
         ///\n\
         /// The discriminants are positional and only valid together with the lexer compiled\n\
         /// from the same revision, which is what this crate guarantees. Upstream may add\n\
         /// variants in a patch release, hence `non_exhaustive`.\n\
         #[repr(u8)]\n\
         #[non_exhaustive]\n\
         #[allow(missing_docs)] // upstream documents only some variants\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]\n\
         pub enum TokenKind {\n",
    );
    for (i, entry) in tokens.iter().enumerate() {
        for line in &entry.doc {
            out.push_str(&format!("    /// {line}\n"));
        }
        out.push_str(&format!("    {} = {i},\n", entry.name));
    }
    out.push_str("}\n\n");

    out.push_str("impl TokenKind {\n    /// Every kind, in discriminant order.\n    pub const ALL: &'static [TokenKind] = &[\n");
    for entry in &tokens {
        out.push_str(&format!("        TokenKind::{},\n", entry.name));
    }
    out.push_str("    ];\n\n");

    out.push_str("    /// The kind with this discriminant, as returned by the C API.\n    pub fn from_raw(raw: u8) -> Option<TokenKind> {\n        match raw {\n");
    for (i, entry) in tokens.iter().enumerate() {
        out.push_str(&format!(
            "            {i} => Some(TokenKind::{}),\n",
            entry.name
        ));
    }
    out.push_str("            _ => None,\n        }\n    }\n\n");

    out.push_str("    /// The upstream enum variant name, e.g. `\"BareWord\"`.\n    pub fn name(self) -> &'static str {\n        match self {\n");
    for entry in &tokens {
        out.push_str(&format!(
            "            TokenKind::{0} => \"{0}\",\n",
            entry.name
        ));
    }
    out.push_str("        }\n    }\n}\n");

    fs::write(out_dir.join("generated.rs"), out).expect("write generated.rs");
}
