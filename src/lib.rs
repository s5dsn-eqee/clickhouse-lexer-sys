#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

use std::ffi::{c_char, c_void};
use std::fmt;
use std::str::Utf8Error;

/// The C API of `upstream/clickhouse_lexer.h`, as compiled by this crate.
pub mod ffi {
    use std::ffi::{c_char, c_int, c_uchar, c_void};

    extern "C" {
        /// Size in bytes of the lexer state `clickhouse_lexer_create` initializes.
        pub static clickhouse_lexer_size: usize;
        /// Placement-constructs a lexer over `[begin, end)` into `ptr`.
        pub fn clickhouse_lexer_create(
            ptr: *mut c_void,
            begin: *const c_char,
            end: *const c_char,
            max_query_size: usize,
        );
        /// Returns the next token's type and writes its bounds to the out-pointers.
        pub fn clickhouse_lexer_next_token(
            ptr: *mut c_void,
            out_token_begin: *mut *const c_char,
            out_token_end: *mut *const c_char,
        ) -> c_uchar;
        /// Whether the type is neither whitespace nor a comment.
        pub fn clickhouse_lexer_token_is_significant(token: c_uchar) -> c_int;
        /// Whether the type is one of the error types.
        pub fn clickhouse_lexer_token_is_error(token: c_uchar) -> c_int;
        /// Whether the type is `EndOfStream`.
        pub fn clickhouse_lexer_token_is_end(token: c_uchar) -> c_int;
    }
}

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

impl TokenKind {
    /// Neither whitespace nor a comment.
    pub fn is_significant(self) -> bool {
        !matches!(self, TokenKind::Whitespace | TokenKind::Comment)
    }

    /// One of the `Error*` kinds.
    pub fn is_error(self) -> bool {
        self > TokenKind::EndOfStream
    }

    /// `EndOfStream`.
    pub fn is_end(self) -> bool {
        self == TokenKind::EndOfStream
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A token of the input: its kind and the byte range it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token<'a> {
    /// The token's kind.
    pub kind: TokenKind,
    /// Byte offset into the input where the token starts.
    pub begin: usize,
    /// Byte offset into the input where the token ends (exclusive).
    pub end: usize,
    /// The bytes of the input the token covers.
    pub text: &'a [u8],
}

impl<'a> Token<'a> {
    /// The token text as a string slice, if it is valid UTF-8.
    pub fn text_str(&self) -> Result<&'a str, Utf8Error> {
        std::str::from_utf8(self.text)
    }
}

/// ClickHouse's lexer over one input.
///
/// Iterating yields the tokens that tile the input from the start, whitespace and
/// comments included. Iteration ends after the first error token, which is yielded,
/// or at the end of the input, where `EndOfStream` is not yielded.
pub struct Lexer<'a> {
    input: &'a [u8],
    // u64 slots keep the pointer fields of the C++ Lexer aligned.
    state: Box<[u64]>,
    previous_end: usize,
    done: bool,
}

impl<'a> Lexer<'a> {
    /// A lexer over `input` with no query size limit.
    pub fn new(input: &'a (impl AsRef<[u8]> + ?Sized)) -> Self {
        Self::with_max_query_size(input, 0)
    }

    /// A lexer over `input` that reports `ErrorMaxQuerySizeExceeded` once a token ends
    /// past `max_query_size` bytes; `0` means unlimited.
    pub fn with_max_query_size(
        input: &'a (impl AsRef<[u8]> + ?Sized),
        max_query_size: usize,
    ) -> Self {
        let input = input.as_ref();
        // SAFETY: a plain `size_t` the C++ side initializes statically.
        let size = unsafe { ffi::clickhouse_lexer_size };
        let mut state = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let begin = input.as_ptr().cast::<c_char>();
        // SAFETY: `state` holds `clickhouse_lexer_size` bytes, and `input` outlives
        // `self`, so the pointers stored in the state stay valid.
        unsafe {
            ffi::clickhouse_lexer_create(
                state.as_mut_ptr().cast::<c_void>(),
                begin,
                begin.add(input.len()),
                max_query_size,
            );
        }
        Self {
            input,
            state,
            previous_end: 0,
            done: false,
        }
    }

    /// The input being lexed.
    pub fn input(&self) -> &'a [u8] {
        self.input
    }

    /// The next token straight from the lexer, including `EndOfStream`.
    ///
    /// Error tokens do not stop subsequent calls.
    pub fn next_token(&mut self) -> Token<'a> {
        let mut begin = std::ptr::null();
        let mut end = std::ptr::null();
        // SAFETY: the state was initialized by `clickhouse_lexer_create` over `input`.
        let raw = unsafe {
            ffi::clickhouse_lexer_next_token(
                self.state.as_mut_ptr().cast::<c_void>(),
                &mut begin,
                &mut end,
            )
        };
        let kind = TokenKind::from_raw(raw)
            .unwrap_or_else(|| panic!("ClickHouse lexer returned unknown token type {raw}"));
        let begin = self.offset(begin);
        let end = self.offset(end);
        Token {
            kind,
            begin,
            end,
            text: &self.input[begin..end],
        }
    }

    /// Byte offset of a pointer the lexer returned, which always lies inside the input.
    fn offset(&self, ptr: *const c_char) -> usize {
        let offset = (ptr as usize).wrapping_sub(self.input.as_ptr() as usize);
        assert!(
            offset <= self.input.len(),
            "ClickHouse lexer returned a pointer outside the input"
        );
        offset
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        if self.done {
            return None;
        }
        let token = self.next_token();
        if token.kind.is_end() {
            self.done = true;
            return None;
        }
        if token.kind.is_error() {
            self.done = true;
            return Some(token);
        }
        // Non-error tokens tile the input, so each one must advance.
        assert!(
            token.end > self.previous_end,
            "ClickHouse lexer stopped advancing at byte {} of {}",
            token.end,
            self.input.len()
        );
        self.previous_end = token.end;
        Some(token)
    }
}

impl fmt::Debug for Lexer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lexer")
            .field("input_len", &self.input.len())
            .field("previous_end", &self.previous_end)
            .field("done", &self.done)
            .finish()
    }
}
