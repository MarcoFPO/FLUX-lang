use serde::Serialize;

use crate::ast::Program;

// ---------------------------------------------------------------------------
// ParseResult — machine-readable JSON output of the parser
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ParseResult {
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ast: Option<Program>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ParseError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Ok,
    Error,
}

// ---------------------------------------------------------------------------
// ParseError — a single parser diagnostic with location info
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

#[derive(Debug, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

// ---------------------------------------------------------------------------
// Conversion from pest errors
// ---------------------------------------------------------------------------

/// Convert a `pest::error::Error<R>` into our `ParseError` representation.
///
/// Extracts line/column from the pest error's `line_col` field and computes
/// the byte span when positional information is available.
pub fn from_pest_error<R: pest::RuleType>(err: &pest::error::Error<R>) -> ParseError {
    let (line, column) = match err.line_col {
        pest::error::LineColLocation::Pos((l, c)) => (l, c),
        pest::error::LineColLocation::Span((l, c), _) => (l, c),
    };

    let span = match &err.location {
        pest::error::InputLocation::Pos(p) => Some(Span {
            start: *p,
            end: *p,
        }),
        pest::error::InputLocation::Span((s, e)) => Some(Span {
            start: *s,
            end: *e,
        }),
    };

    ParseError {
        line,
        column,
        message: err.variant.message().to_string(),
        span,
    }
}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

impl ParseResult {
    /// Create a successful parse result containing the given AST.
    pub fn ok(ast: Program) -> Self {
        Self {
            status: Status::Ok,
            ast: Some(ast),
            errors: Vec::new(),
        }
    }

    /// Create a failed parse result from a list of errors.
    pub fn error(errors: Vec<ParseError>) -> Self {
        Self {
            status: Status::Error,
            ast: None,
            errors,
        }
    }
}

impl ParseError {
    /// Create a parse error without span information (e.g. for semantic errors).
    pub fn new(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            column,
            message: message.into(),
            span: None,
        }
    }
}
