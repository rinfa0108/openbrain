use crate::{ErrorCode, ErrorEnvelope};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldPath {
    pub segments: Vec<String>,
}

impl FieldPath {
    pub fn new(segments: Vec<String>) -> Self {
        Self { segments }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    String(String),
    Number(f64),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    Compare {
        field: FieldPath,
        op: CmpOp,
        value: Literal,
    },
    In {
        field: FieldPath,
        values: Vec<Literal>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Pred(Predicate),
}

fn line_col(input: &str, index: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in input.char_indices() {
        if i >= index {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn invalid_request(input: &str, index: usize, message: impl Into<String>) -> ErrorEnvelope {
    let (line, col) = line_col(input, index);
    ErrorEnvelope::new(
        ErrorCode::ObInvalidRequest,
        message,
        Some(serde_json::json!({"index": index, "line": line, "col": col})),
    )
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Ident(String),
    String(String),
    Number(f64),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    OpEq,
    OpNe,
    OpGt,
    OpGte,
    OpLt,
    OpLte,
    OpRegex,
    And,
    Or,
    Not,
    In,
    True,
    False,
    Null,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    start: usize,
}

struct Lexer<'a> {
    input: &'a str,
    bytes: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            i: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.i += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\r' | b'\n' => self.i += 1,
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, ErrorEnvelope> {
        self.skip_ws();
        let start = self.i;
        let Some(b) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                start,
            });
        };

        let kind = match b {
            b'(' => {
                self.i += 1;
                TokenKind::LParen
            }
            b')' => {
                self.i += 1;
                TokenKind::RParen
            }
            b'[' => {
                self.i += 1;
                TokenKind::LBracket
            }
            b']' => {
                self.i += 1;
                TokenKind::RBracket
            }
            b',' => {
                self.i += 1;
                TokenKind::Comma
            }
            b'.' => {
                self.i += 1;
                TokenKind::Dot
            }
            b'=' => {
                self.i += 1;
                if self.peek() == Some(b'=') {
                    self.i += 1;
                    TokenKind::OpEq
                } else {
                    return Err(invalid_request(self.input, start, "expected '=='"));
                }
            }
            b'!' => {
                self.i += 1;
                if self.peek() == Some(b'=') {
                    self.i += 1;
                    TokenKind::OpNe
                } else {
                    return Err(invalid_request(self.input, start, "expected '!='"));
                }
            }
            b'>' => {
                self.i += 1;
                if self.peek() == Some(b'=') {
                    self.i += 1;
                    TokenKind::OpGte
                } else {
                    TokenKind::OpGt
                }
            }
            b'<' => {
                self.i += 1;
                if self.peek() == Some(b'=') {
                    self.i += 1;
                    TokenKind::OpLte
                } else {
                    TokenKind::OpLt
                }
            }
            b'~' => {
                self.i += 1;
                if self.peek() == Some(b'=') {
                    self.i += 1;
                    TokenKind::OpRegex
                } else {
                    return Err(invalid_request(self.input, start, "expected '~='"));
                }
            }
            b'"' => {
                self.i += 1;
                let mut out = String::new();
                loop {
                    let Some(ch) = self.bump() else {
                        return Err(invalid_request(self.input, start, "unterminated string"));
                    };
                    match ch {
                        b'"' => break,
                        b'\\' => {
                            let Some(esc) = self.bump() else {
                                return Err(invalid_request(
                                    self.input,
                                    self.i,
                                    "unterminated escape",
                                ));
                            };
                            match esc {
                                b'"' => out.push('"'),
                                b'\\' => out.push('\\'),
                                b'n' => out.push('\n'),
                                b'r' => out.push('\r'),
                                b't' => out.push('\t'),
                                _ => {
                                    return Err(invalid_request(
                                        self.input,
                                        self.i - 1,
                                        "unsupported escape",
                                    ));
                                }
                            }
                        }
                        _ => out.push(ch as char),
                    }
                }
                TokenKind::String(out)
            }
            b'-' | b'0'..=b'9' => {
                let mut j = self.i;
                if self.bytes[j] == b'-' {
                    j += 1;
                }
                let mut saw_digit = false;
                while j < self.bytes.len() && self.bytes[j].is_ascii_digit() {
                    saw_digit = true;
                    j += 1;
                }
                if j < self.bytes.len() && self.bytes[j] == b'.' {
                    j += 1;
                    while j < self.bytes.len() && self.bytes[j].is_ascii_digit() {
                        saw_digit = true;
                        j += 1;
                    }
                }
                if !saw_digit {
                    return Err(invalid_request(self.input, start, "invalid number"));
                }
                let text = &self.input[self.i..j];
                self.i = j;
                let num: f64 = text
                    .parse()
                    .map_err(|_| invalid_request(self.input, start, "invalid number"))?;
                TokenKind::Number(num)
            }
            _ => {
                if (b as char).is_ascii_alphabetic() || b == b'_' {
                    let mut j = self.i + 1;
                    while j < self.bytes.len() {
                        let c = self.bytes[j];
                        if (c as char).is_ascii_alphanumeric() || c == b'_' {
                            j += 1;
                        } else {
                            break;
                        }
                    }
                    let text = &self.input[self.i..j];
                    self.i = j;
                    match text.to_ascii_uppercase().as_str() {
                        "AND" => TokenKind::And,
                        "OR" => TokenKind::Or,
                        "NOT" => TokenKind::Not,
                        "IN" => TokenKind::In,
                        "TRUE" => TokenKind::True,
                        "FALSE" => TokenKind::False,
                        "NULL" => TokenKind::Null,
                        _ => TokenKind::Ident(text.to_string()),
                    }
                } else {
                    return Err(invalid_request(
                        self.input,
                        start,
                        format!("unexpected character: {}", b as char),
                    ));
                }
            }
        };

        Ok(Token { kind, start })
    }

    fn tokenize(mut self) -> Result<Vec<Token>, ErrorEnvelope> {
        let mut tokens = Vec::new();
        loop {
            let t = self.next_token()?;
            let is_eof = matches!(t.kind, TokenKind::Eof);
            tokens.push(t);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }
}

struct Parser<'a> {
    input: &'a str,
    tokens: Vec<Token>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, tokens: Vec<Token>) -> Self {
        Self {
            input,
            tokens,
            pos: 0,
        }
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        self.pos += 1;
        t
    }

    fn expect(&mut self, f: fn(&TokenKind) -> bool, msg: &str) -> Result<Token, ErrorEnvelope> {
        let t = self.tokens[self.pos].clone();
        if f(&t.kind) {
            self.pos += 1;
            Ok(t)
        } else {
            Err(invalid_request(self.input, t.start, msg))
        }
    }

    fn parse(&mut self) -> Result<Expr, ErrorEnvelope> {
        let expr = self.parse_or()?;
        if !matches!(self.peek(), TokenKind::Eof) {
            let t = &self.tokens[self.pos];
            return Err(invalid_request(
                self.input,
                t.start,
                "unexpected trailing tokens",
            ));
        }
        Ok(expr)
    }

    fn parse_or(&mut self) -> Result<Expr, ErrorEnvelope> {
        let mut items = vec![self.parse_and()?];
        while matches!(self.peek(), TokenKind::Or) {
            self.bump();
            items.push(self.parse_and()?);
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Expr::Or(items)
        })
    }

    fn parse_and(&mut self) -> Result<Expr, ErrorEnvelope> {
        let mut items = vec![self.parse_not()?];
        while matches!(self.peek(), TokenKind::And) {
            self.bump();
            items.push(self.parse_not()?);
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Expr::And(items)
        })
    }

    fn parse_not(&mut self) -> Result<Expr, ErrorEnvelope> {
        if matches!(self.peek(), TokenKind::Not) {
            self.bump();
            Ok(Expr::Not(Box::new(self.parse_not()?)))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ErrorEnvelope> {
        match self.peek() {
            TokenKind::LParen => {
                self.bump();
                let e = self.parse_or()?;
                self.expect(|k| matches!(k, TokenKind::RParen), "expected ')'")?;
                Ok(e)
            }
            _ => Ok(Expr::Pred(self.parse_predicate()?)),
        }
    }

    fn parse_field(&mut self) -> Result<FieldPath, ErrorEnvelope> {
        let mut segments = Vec::new();
        let first = self.expect(
            |k| matches!(k, TokenKind::Ident(_)),
            "expected field identifier",
        )?;
        if let TokenKind::Ident(s) = first.kind {
            segments.push(s);
        }

        while matches!(self.peek(), TokenKind::Dot) {
            self.bump();
            let seg = self.expect(
                |k| matches!(k, TokenKind::Ident(_)),
                "expected identifier after '.'",
            )?;
            if let TokenKind::Ident(s) = seg.kind {
                segments.push(s);
            }
        }

        Ok(FieldPath::new(segments))
    }

    fn parse_literal(&mut self) -> Result<Literal, ErrorEnvelope> {
        let t = self.bump();
        match t.kind {
            TokenKind::String(s) => Ok(Literal::String(s)),
            TokenKind::Number(n) => Ok(Literal::Number(n)),
            TokenKind::True => Ok(Literal::Bool(true)),
            TokenKind::False => Ok(Literal::Bool(false)),
            TokenKind::Null => Ok(Literal::Null),
            _ => Err(invalid_request(self.input, t.start, "expected literal")),
        }
    }

    fn parse_list(&mut self) -> Result<Vec<Literal>, ErrorEnvelope> {
        self.expect(|k| matches!(k, TokenKind::LBracket), "expected '['")?;
        if matches!(self.peek(), TokenKind::RBracket) {
            let t = &self.tokens[self.pos];
            return Err(invalid_request(
                self.input,
                t.start,
                "IN list must not be empty",
            ));
        }

        let mut items = Vec::new();
        loop {
            items.push(self.parse_literal()?);
            match self.peek() {
                TokenKind::Comma => {
                    self.bump();
                }
                TokenKind::RBracket => break,
                _ => {
                    let t = &self.tokens[self.pos];
                    return Err(invalid_request(self.input, t.start, "expected ',' or ']'"));
                }
            }
        }
        self.expect(|k| matches!(k, TokenKind::RBracket), "expected ']'")?;
        Ok(items)
    }

    fn parse_predicate(&mut self) -> Result<Predicate, ErrorEnvelope> {
        let field = self.parse_field()?;
        match self.peek() {
            TokenKind::In => {
                self.bump();
                let values = self.parse_list()?;
                Ok(Predicate::In { field, values })
            }
            TokenKind::OpRegex => {
                let t = self.bump();
                Err(invalid_request(
                    self.input,
                    t.start,
                    "regex operator '~=' is disabled in v0.1",
                ))
            }
            TokenKind::OpEq
            | TokenKind::OpNe
            | TokenKind::OpGt
            | TokenKind::OpGte
            | TokenKind::OpLt
            | TokenKind::OpLte => {
                let op = match self.bump().kind {
                    TokenKind::OpEq => CmpOp::Eq,
                    TokenKind::OpNe => CmpOp::Ne,
                    TokenKind::OpGt => CmpOp::Gt,
                    TokenKind::OpGte => CmpOp::Gte,
                    TokenKind::OpLt => CmpOp::Lt,
                    TokenKind::OpLte => CmpOp::Lte,
                    _ => unreachable!(),
                };
                let value = self.parse_literal()?;
                Ok(Predicate::Compare { field, op, value })
            }
            _ => {
                let t = &self.tokens[self.pos];
                Err(invalid_request(
                    self.input,
                    t.start,
                    "expected operator (==, !=, >, >=, <, <=, IN)",
                ))
            }
        }
    }
}

pub fn parse_where(input: &str) -> Result<Expr, ErrorEnvelope> {
    let tokens = Lexer::new(input).tokenize()?;
    let mut p = Parser::new(input, tokens);
    p.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_precedence_not_and_or() {
        let expr = parse_where(r#"NOT type == "claim" OR status == "draft" AND id == "x""#)
            .expect("parse");

        match expr {
            Expr::Or(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0], Expr::Not(_)));
                assert!(matches!(items[1], Expr::And(_)));
            }
            _ => panic!("expected OR"),
        }
    }

    #[test]
    fn rejects_semicolon_injection_like_input() {
        let err = parse_where(r#"type == "claim"; DROP TABLE ob_objects"#).unwrap_err();
        assert_eq!(err.code, "OB_INVALID_REQUEST");
    }

    #[test]
    fn rejects_regex_operator() {
        let err = parse_where(r#"type ~= "claim""#).unwrap_err();
        assert_eq!(err.code, "OB_INVALID_REQUEST");
    }
}
