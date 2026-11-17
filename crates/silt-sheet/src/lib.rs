//! Formulas for Silt's sheets (ADR 0077): pure, so the daemon, the server,
//! agents and the browser compute the same values.
//!
//! A cell holds what was typed. One starting with `=` is a formula, kept
//! with its references to rows' and columns' ids ([`to_ids`]) so inserting
//! a row moves what a formula shows, not what it means, and shown and
//! evaluated with addresses as the sheet stands ([`from_ids`]).

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The most rows and columns a sheet has (ADR 0077).
pub const ROWS: usize = 10_000;
pub const COLUMNS: usize = 200;
/// How deep a formula's references may chain before it's an error.
const DEPTH: usize = 256;
/// How deeply one formula nests parentheses, calls and signs, so a cell
/// can't recurse past a thread's stack.
const NESTING: usize = 64;
/// The longest text a value holds, in bytes, as spreadsheets cap a cell.
pub const TEXT_BYTES: usize = 32 * 1024;
/// How many passes `evaluate` makes over cells whose references ran deeper
/// than [`DEPTH`], each picking up the values the last one kept.
const PASSES: usize = 8;

/// A formula's error, as a sheet shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellError {
    Div0,
    Value,
    Ref,
    Name,
    Na,
    Num,
    Cycle,
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Div0 => "#DIV/0!",
            Self::Value => "#VALUE!",
            Self::Ref => "#REF!",
            Self::Name => "#NAME?",
            Self::Na => "#N/A",
            Self::Num => "#NUM!",
            Self::Cycle => "#CYCLE!",
        })
    }
}

/// A cell's value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(CellError),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => Ok(()),
            Self::Number(number) => write!(f, "{}", number_text(*number)),
            Self::Text(text) => f.write_str(text),
            Self::Bool(true) => f.write_str("TRUE"),
            Self::Bool(false) => f.write_str("FALSE"),
            Self::Error(error) => write!(f, "{error}"),
        }
    }
}

fn number_text(number: f64) -> String {
    // A sum of nothing can come out as negative zero.
    let number = if number == 0.0 { 0.0 } else { number };
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.0}")
    } else {
        let text = format!("{number:.10}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

impl Value {
    /// What a typed cell is: a number, a boolean, text, or nothing.
    pub fn of_typed(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self::Empty;
        }
        if let Ok(number) = trimmed.parse::<f64>()
            && number.is_finite()
        {
            return Self::Number(number);
        }
        match trimmed.to_ascii_uppercase().as_str() {
            "TRUE" => Self::Bool(true),
            "FALSE" => Self::Bool(false),
            _ => Self::Text(raw.to_owned()),
        }
    }

    fn number(&self) -> Result<f64, CellError> {
        match self {
            Self::Empty => Ok(0.0),
            Self::Number(number) => Ok(*number),
            Self::Bool(value) => Ok(f64::from(u8::from(*value))),
            Self::Text(text) => text.trim().parse().map_err(|_| CellError::Value),
            Self::Error(error) => Err(*error),
        }
    }

    fn text(&self) -> Result<String, CellError> {
        match self {
            Self::Error(error) => Err(*error),
            other => Ok(other.to_string()),
        }
    }

    fn truthy(&self) -> Result<bool, CellError> {
        match self {
            Self::Bool(value) => Ok(*value),
            Self::Empty => Ok(false),
            Self::Number(number) => Ok(*number != 0.0),
            Self::Text(text) => match text.to_ascii_uppercase().as_str() {
                "TRUE" => Ok(true),
                "FALSE" => Ok(false),
                _ => Err(CellError::Value),
            },
            Self::Error(error) => Err(*error),
        }
    }
}

/// A column's letters: 0 is `A`, 26 is `AA`.
pub fn column_name(mut column: usize) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (column % 26) as u8);
        if column < 26 {
            break;
        }
        column = column / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

/// The column `letters` name, from 0.
pub fn column_index(letters: &str) -> Option<usize> {
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    letters.bytes().try_fold(0usize, |at, b| Some(at * 26 + usize::from(b - b'A') + 1)).map(|at| at - 1)
}

/// `A1`-style address of a row and a column, from 0.
pub fn address(row: usize, column: usize) -> String {
    format!("{}{}", column_name(column), row + 1)
}

/// A row and column an `A1` address names, from 0.
pub fn parse_address(text: &str) -> Option<(usize, usize)> {
    let text = text.trim().replace('$', "").to_ascii_uppercase();
    let split = text.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = text.split_at(split);
    let column = column_index(letters)?;
    let row: usize = digits.parse().ok()?;
    (row >= 1).then(|| (row - 1, column))
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    /// A cell by address, on another sheet (from 0) when it names one.
    Ref(Option<u16>, usize, usize),
    /// A reference kept by the ids of its row and column, and its sheet.
    Id(Option<u16>, String, String),
    Name(String),
    Op(&'static str),
    Open,
    Close,
    Comma,
    Colon,
    Percent,
    Error(CellError),
}

fn lex(source: &str) -> Result<Vec<(Token, std::ops::Range<usize>)>, CellError> {
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    let end_of = |at: usize| chars.get(at).map_or(source.len(), |(index, _)| *index);
    while at < chars.len() {
        let (start, char) = chars[at];
        match char {
            ' ' | '\t' | '\n' => at += 1,
            '"' => {
                let mut text = String::new();
                at += 1;
                loop {
                    match chars.get(at) {
                        None => return Err(CellError::Value),
                        Some((_, '"')) if chars.get(at + 1).is_some_and(|(_, next)| *next == '"') => {
                            text.push('"');
                            at += 2;
                        }
                        Some((_, '"')) => {
                            at += 1;
                            break;
                        }
                        Some((_, other)) => {
                            text.push(*other);
                            at += 1;
                        }
                    }
                }
                tokens.push((Token::Text(text), start..end_of(at)));
            }
            '⟨' => {
                let close = chars[at..].iter().position(|(_, c)| *c == '⟩').ok_or(CellError::Ref)? + at;
                let inner: String = chars[at + 1..close].iter().map(|(_, c)| c).collect();
                // `⟨row,column⟩` on this sheet, `⟨2:row,column⟩` on sheet 2 from 0.
                let (sheet, inner) = match inner.split_once(':') {
                    Some((sheet, rest)) => (Some(sheet.parse::<u16>().map_err(|_| CellError::Ref)?), rest.to_owned()),
                    None => (None, inner),
                };
                let (row, column) = inner.split_once(',').ok_or(CellError::Ref)?;
                tokens.push((Token::Id(sheet, row.to_owned(), column.to_owned()), start..end_of(close + 1)));
                at = close + 1;
            }
            '#' => {
                let word: String = chars[at..].iter().map(|(_, c)| *c).take_while(|c| !matches!(c, ',' | ')' | ' ')).collect();
                let error = match word.as_str() {
                    "#REF!" => CellError::Ref,
                    "#DIV/0!" => CellError::Div0,
                    "#N/A" => CellError::Na,
                    "#NAME?" => CellError::Name,
                    "#NUM!" => CellError::Num,
                    _ => CellError::Value,
                };
                at += word.chars().count();
                tokens.push((Token::Error(error), start..end_of(at)));
            }
            '0'..='9' | '.' => {
                let mut text: String =
                    chars[at..].iter().map(|(_, c)| *c).take_while(|c| c.is_ascii_digit() || *c == '.').collect();
                // An exponent, as in 1e5 or 2.5E-3.
                let after = at + text.chars().count();
                if chars.get(after).is_some_and(|(_, c)| matches!(c, 'e' | 'E')) {
                    let sign = usize::from(chars.get(after + 1).is_some_and(|(_, c)| matches!(c, '+' | '-')));
                    let digits =
                        chars[(after + 1 + sign).min(chars.len())..].iter().take_while(|(_, c)| c.is_ascii_digit()).count();
                    if digits > 0 {
                        text.extend(chars[after..after + 1 + sign + digits].iter().map(|(_, c)| *c));
                    }
                }
                at += text.chars().count();
                let number = text.parse().map_err(|_| CellError::Value)?;
                tokens.push((Token::Number(number), start..end_of(at)));
            }
            c if c.is_ascii_alphabetic() || c == '$' || c == '_' => {
                let word: String = chars[at..]
                    .iter()
                    .map(|(_, c)| *c)
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '$' | '_' | '.'))
                    .collect();
                at += word.chars().count();
                // `Sheet2!B3`: a cell on another sheet, its tab's name and the address.
                let mut sheet = None;
                let mut word = word;
                if chars.get(at).is_some_and(|(_, c)| *c == '!') {
                    let named = sheet_named(&word).ok_or(CellError::Ref)?;
                    at += 1;
                    word = chars[at..].iter().map(|(_, c)| *c).take_while(|c| c.is_ascii_alphanumeric() || *c == '$').collect();
                    at += word.chars().count();
                    sheet = Some(named);
                }
                let token = match parse_address(&word) {
                    Some((row, column))
                        if word.chars().next().is_some_and(|c| c == '$' || c.is_ascii_alphabetic())
                            && word.bytes().any(|b| b.is_ascii_digit())
                            && !chars.get(at).is_some_and(|(_, c)| *c == '(') =>
                    {
                        // A range's end is on its start's sheet: `Sheet2!A1:B3`.
                        let sheet = sheet.or(match tokens.as_slice() {
                            [.., (Token::Ref(on, ..), _), (Token::Colon, _)] => *on,
                            _ => None,
                        });
                        Token::Ref(sheet, row, column)
                    }
                    _ if sheet.is_some() => return Err(CellError::Ref),
                    _ => Token::Name(word.to_ascii_uppercase()),
                };
                tokens.push((token, start..end_of(at)));
            }
            _ => {
                let two: String = chars[at..].iter().take(2).map(|(_, c)| *c).collect();
                let (token, width) = match two.as_str() {
                    "<>" => (Token::Op("<>"), 2),
                    "<=" => (Token::Op("<="), 2),
                    ">=" => (Token::Op(">="), 2),
                    _ => (
                        match char {
                            '+' => Token::Op("+"),
                            '-' => Token::Op("-"),
                            '*' => Token::Op("*"),
                            '/' => Token::Op("/"),
                            '^' => Token::Op("^"),
                            '&' => Token::Op("&"),
                            '=' => Token::Op("="),
                            '<' => Token::Op("<"),
                            '>' => Token::Op(">"),
                            '(' => Token::Open,
                            ')' => Token::Close,
                            ',' | ';' => Token::Comma,
                            ':' => Token::Colon,
                            '%' => Token::Percent,
                            _ => return Err(CellError::Name),
                        },
                        1,
                    ),
                };
                at += width;
                tokens.push((token, start..end_of(at)));
            }
        }
    }
    Ok(tokens)
}

/// A formula's text with each `A1` reference replaced by its row's and
/// column's ids, so it keeps meaning the same cells as rows and columns
/// move. `rows` and `columns` are the ids in order; ids hold no `,` or `⟩`.
pub fn to_ids(formula: &str, rows: &[String], columns: &[String]) -> String {
    to_ids_in(formula, &[(rows, columns)], 0)
}

/// [`to_ids`] for sheet `sheet` of a book whose sheets' row and column ids
/// are `axes`: a reference to another sheet keeps that sheet's ids.
pub fn to_ids_in(formula: &str, axes: &[(&[String], &[String])], sheet: usize) -> String {
    rewrite(formula, |token| match token {
        Token::Ref(on, row, column) => {
            let of = on.map_or(sheet, usize::from);
            let Some((rows, columns)) = axes.get(of) else { return Some("#REF!".to_owned()) };
            Some(match (rows.get(*row), columns.get(*column)) {
                (Some(row), Some(column)) => match on {
                    Some(on) => format!("⟨{on}:{row},{column}⟩"),
                    None => format!("⟨{row},{column}⟩"),
                },
                _ => "#REF!".to_owned(),
            })
        }
        _ => None,
    })
}

/// A kept formula as the sheet stands: each reference by ids shown as its
/// `A1` address, or `#REF!` where its row or column is gone.
pub fn from_ids(formula: &str, rows: &[String], columns: &[String]) -> String {
    Positions::new(rows, columns).from_ids(formula)
}

/// Where a sheet's rows and its columns stand, by id.
type Axes<'a> = (HashMap<&'a str, usize>, HashMap<&'a str, usize>);

/// Where each row and column id stands, to show many kept formulas of one
/// sheet without finding them again for each.
pub struct Positions<'a> {
    /// Each sheet's rows and columns, by id.
    sheets: Vec<Axes<'a>>,
    /// The sheet whose formulas these show.
    sheet: usize,
}

impl<'a> Positions<'a> {
    pub fn new(rows: &'a [String], columns: &'a [String]) -> Self {
        Self::in_book(&[(rows, columns)], 0)
    }

    /// Positions for sheet `sheet` of a book whose sheets' ids are `axes`.
    pub fn in_book(axes: &[(&'a [String], &'a [String])], sheet: usize) -> Self {
        let at = |ids: &'a [String]| ids.iter().enumerate().map(|(at, id)| (id.as_str(), at)).collect();
        Self { sheets: axes.iter().map(|(rows, columns)| (at(rows), at(columns))).collect(), sheet }
    }

    /// [`from_ids`] against these positions; a reference to another sheet
    /// shows as `Sheet2!B3`.
    pub fn from_ids(&self, formula: &str) -> String {
        // A range's end shows without its sheet, which its start names.
        let after_colon = std::cell::Cell::new(false);
        rewrite(formula, |token| {
            let ends_range = after_colon.replace(*token == Token::Colon);
            match token {
                Token::Id(on, row, column) => {
                    let Some((rows, columns)) = self.sheets.get(on.map_or(self.sheet, usize::from)) else {
                        return Some("#REF!".to_owned());
                    };
                    Some(match (rows.get(row.as_str()), columns.get(column.as_str())) {
                        (Some(row), Some(column)) => match on {
                            Some(on) if !ends_range => format!("{}!{}", sheet_name(*on), address(*row, *column)),
                            _ => address(*row, *column),
                        },
                        _ => "#REF!".to_owned(),
                    })
                }
                _ => None,
            }
        })
    }
}

/// The name a sheet's tab shows, from 0: `Sheet1`, `Sheet2`, ….
pub fn sheet_name(sheet: u16) -> String {
    format!("Sheet{}", u32::from(sheet) + 1)
}

/// The sheet, from 0, a name such as `Sheet2` names.
fn sheet_named(name: &str) -> Option<u16> {
    let number: u16 = name.get(..5).filter(|start| start.eq_ignore_ascii_case("sheet")).and(name.get(5..))?.parse().ok()?;
    number.checked_sub(1)
}

fn rewrite(formula: &str, change: impl Fn(&Token) -> Option<String>) -> String {
    let Some(body) = formula.strip_prefix('=') else {
        return formula.to_owned();
    };
    let Ok(tokens) = lex(body) else {
        return formula.to_owned();
    };
    let mut out = String::from("=");
    let mut last = 0;
    for (token, span) in &tokens {
        if let Some(replacement) = change(token) {
            out.push_str(&body[last..span.start]);
            out.push_str(&replacement);
            last = span.end;
        }
    }
    out.push_str(&body[last..]);
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Value(Value),
    /// A cell, on another sheet when it names one.
    Ref(Option<u16>, usize, usize),
    Range(Option<u16>, usize, usize, usize, usize),
    Negate(Box<Expr>),
    Percent(Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    /// How deeply what's being read nests.
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn binary(&mut self, level: usize) -> Result<Expr, CellError> {
        const LEVELS: [&[&str]; 4] = [&["=", "<>", "<", ">", "<=", ">="], &["&"], &["+", "-"], &["*", "/"]];
        if level == LEVELS.len() {
            return self.power();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(Token::Op(op)) = self.peek() {
            let op = *op;
            if !LEVELS[level].contains(&op) {
                break;
            }
            self.at += 1;
            let right = self.binary(level + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Exponents left to right, as spreadsheets read `2^3^2`.
    fn power(&mut self) -> Result<Expr, CellError> {
        let mut base = self.unary()?;
        while self.peek() == Some(&Token::Op("^")) {
            self.at += 1;
            let exponent = self.unary()?;
            base = Expr::Binary("^", Box::new(base), Box::new(exponent));
        }
        Ok(base)
    }

    /// One level deeper, refused past [`NESTING`].
    fn deeper<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T, CellError>) -> Result<T, CellError> {
        if self.depth >= NESTING {
            return Err(CellError::Value);
        }
        self.depth += 1;
        let read = read(self);
        self.depth -= 1;
        read
    }

    fn unary(&mut self) -> Result<Expr, CellError> {
        match self.peek() {
            Some(Token::Op("-")) => {
                self.at += 1;
                Ok(Expr::Negate(Box::new(self.deeper(Self::unary)?)))
            }
            Some(Token::Op("+")) => {
                self.at += 1;
                self.deeper(Self::unary)
            }
            _ => {
                let mut expr = self.primary()?;
                while self.peek() == Some(&Token::Percent) {
                    self.at += 1;
                    expr = Expr::Percent(Box::new(expr));
                }
                Ok(expr)
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, CellError> {
        match self.next().ok_or(CellError::Value)? {
            Token::Number(number) => Ok(Expr::Value(Value::Number(number))),
            Token::Text(text) => Ok(Expr::Value(Value::Text(text))),
            Token::Error(error) => Ok(Expr::Value(Value::Error(error))),
            Token::Id(..) => Err(CellError::Ref),
            Token::Ref(sheet, row, column) => {
                if self.peek() == Some(&Token::Colon) {
                    self.at += 1;
                    // `Sheet2!A1:B3`: the range's end is on its start's sheet.
                    let Some(Token::Ref(to_sheet, to_row, to_column)) = self.next() else {
                        return Err(CellError::Ref);
                    };
                    if to_sheet.is_some() && to_sheet != sheet {
                        return Err(CellError::Ref);
                    }
                    return Ok(Expr::Range(
                        sheet,
                        row.min(to_row),
                        column.min(to_column),
                        row.max(to_row),
                        column.max(to_column),
                    ));
                }
                Ok(Expr::Ref(sheet, row, column))
            }
            Token::Name(name) if name == "TRUE" && self.peek() != Some(&Token::Open) => Ok(Expr::Value(Value::Bool(true))),
            Token::Name(name) if name == "FALSE" && self.peek() != Some(&Token::Open) => Ok(Expr::Value(Value::Bool(false))),
            Token::Name(name) => {
                if self.next() != Some(Token::Open) {
                    return Err(CellError::Name);
                }
                let mut args = Vec::new();
                if self.peek() == Some(&Token::Close) {
                    self.at += 1;
                    return Ok(Expr::Call(name, args));
                }
                loop {
                    args.push(self.deeper(|parser| parser.binary(0))?);
                    match self.next() {
                        Some(Token::Comma) => continue,
                        Some(Token::Close) => break,
                        _ => return Err(CellError::Value),
                    }
                }
                Ok(Expr::Call(name, args))
            }
            Token::Open => {
                let inner = self.deeper(|parser| parser.binary(0))?;
                if self.next() != Some(Token::Close) {
                    return Err(CellError::Value);
                }
                Ok(inner)
            }
            _ => Err(CellError::Value),
        }
    }
}

/// The functions this evaluator computes, which a workbook made from a sheet
/// may keep as formulas.
pub const FUNCTIONS: &[&str] = &[
    "SUM", "AVERAGE", "MIN", "MAX", "COUNT", "COUNTA", "IF", "IFS", "AND", "OR", "NOT", "ROUND", "ABS", "CONCAT", "TEXT", "LEFT",
    "RIGHT", "LEN", "TODAY", "DATE", "VLOOKUP", "XLOOKUP", "SUMIF", "COUNTIF",
];

/// Whether `formula` (its `=` optional) is one this evaluator reads, calling
/// only [`FUNCTIONS`]: what a workbook may keep as a live formula.
pub fn formula_ok(formula: &str) -> bool {
    fn calls_known(expr: &Expr) -> bool {
        match expr {
            Expr::Value(_) | Expr::Ref(..) | Expr::Range(..) => true,
            Expr::Negate(inner) | Expr::Percent(inner) => calls_known(inner),
            Expr::Binary(_, left, right) => calls_known(left) && calls_known(right),
            Expr::Call(name, args) => FUNCTIONS.contains(&name.as_str()) && args.iter().all(calls_known),
        }
    }
    parse(formula.strip_prefix('=').unwrap_or(formula)).is_ok_and(|expr| calls_known(&expr))
}

/// The cells `formula` refers to by `A1` address, by row and column.
pub fn addresses_in(formula: &str) -> Vec<(usize, usize)> {
    let Some(body) = formula.strip_prefix('=') else { return Vec::new() };
    lex(body).map_or_else(
        |_| Vec::new(),
        |tokens| {
            tokens
                .into_iter()
                .filter_map(|(token, _)| match token {
                    Token::Ref(None, row, column) => Some((row, column)),
                    _ => None,
                })
                .collect()
        },
    )
}

fn parse(formula: &str) -> Result<Expr, CellError> {
    let tokens = lex(formula)?.into_iter().map(|(token, _)| token).collect();
    let mut parser = Parser { tokens, at: 0, depth: 0 };
    let expr = parser.binary(0)?;
    if parser.at != parser.tokens.len() {
        return Err(CellError::Value);
    }
    Ok(expr)
}

/// A sheet's typed cells, by row and column from 0, and what day it is.
pub struct Sheet<'a> {
    pub cells: &'a [Vec<String>],
    /// Today as a date serial, days since 1899-12-30, as spreadsheets count.
    pub today: f64,
}

/// Every sheet of an artifact, by its typed cells, for formulas that reach
/// across them, and what day it is.
pub struct Book<'a> {
    pub sheets: &'a [&'a [Vec<String>]],
    pub today: f64,
}

/// Every cell's value; a formula's references are `A1` addresses here (see
/// [`from_ids`]). A cycle is an error in each cell on it, never a hang.
pub fn evaluate(sheet: &Sheet<'_>) -> Vec<Vec<Value>> {
    evaluate_in(&Book { sheets: &[sheet.cells], today: sheet.today }, 0)
}

/// [`evaluate`] for sheet `sheet` of `book`, whose formulas may name cells
/// of its other sheets, `Sheet2!B3`.
pub fn evaluate_in(book: &Book<'_>, sheet: usize) -> Vec<Vec<Value>> {
    let Some(cells) = book.sheets.get(sheet) else { return Vec::new() };
    let mut evaluator = Evaluator::new(book, sheet);
    // A chain of references deeper than DEPTH isn't a cycle: each pass keeps
    // what it could reach, so the next goes further, as a running total
    // whose sum sits above it needs.
    for _ in 0..PASSES {
        evaluator.deep = false;
        for row in 0..cells.len() {
            for column in 0..cells[row].len() {
                evaluator.cell(sheet, row, column);
            }
        }
        if !evaluator.deep {
            break;
        }
    }
    (0..cells.len())
        .map(|row| {
            (0..cells[row].len())
                .map(|column| evaluator.done.get(&(sheet, row, column)).cloned().unwrap_or(Value::Error(CellError::Num)))
                .collect()
        })
        .collect()
}

/// One formula's value against `sheet`, as an agent asks for one.
pub fn evaluate_formula(sheet: &Sheet<'_>, formula: &str) -> Value {
    let book = Book { sheets: &[sheet.cells], today: sheet.today };
    let mut evaluator = Evaluator::new(&book, 0);
    match parse(formula.strip_prefix('=').unwrap_or(formula)) {
        Ok(expr) => evaluator.value(&expr),
        Err(error) => Value::Error(error),
    }
}

struct Evaluator<'s, 'a> {
    book: &'s Book<'a>,
    /// The sheet whose cell is being worked out, which its own references mean.
    current: usize,
    done: HashMap<(usize, usize, usize), Value>,
    visiting: Vec<(usize, usize, usize)>,
    /// Whether a value since this was last cleared ran into [`DEPTH`], so
    /// it isn't kept: a later pass may reach it.
    deep: bool,
}

impl<'s, 'a> Evaluator<'s, 'a> {
    fn new(book: &'s Book<'a>, current: usize) -> Self {
        Self { book, current, done: HashMap::new(), visiting: Vec::new(), deep: false }
    }

    fn cells(&self, sheet: usize) -> &'a [Vec<String>] {
        self.book.sheets.get(sheet).copied().unwrap_or_default()
    }

    /// The sheet a reference means: the one it names, or the current one.
    fn sheet_of(&self, named: Option<u16>) -> Result<usize, CellError> {
        match named {
            None => Ok(self.current),
            Some(sheet) if usize::from(sheet) < self.book.sheets.len() => Ok(usize::from(sheet)),
            Some(_) => Err(CellError::Ref),
        }
    }

    /// The last row, and each row's last column, a range may reach: the
    /// sheet's own, as every cell past them is empty.
    fn clamped(
        &self,
        sheet: usize,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> Vec<(usize, std::ops::RangeInclusive<usize>)> {
        let cells = self.cells(sheet);
        let rows = cells.len().min(ROWS);
        (top..=bottom.min(rows.saturating_sub(1)))
            .filter(|row| *row < rows)
            .map(|row| {
                let columns = cells[row].len().min(COLUMNS);
                (row, left..=right.min(columns.saturating_sub(1)))
            })
            .filter(|(row, columns)| columns.start() < &cells[*row].len().min(COLUMNS))
            .collect()
    }

    fn raw(&self, sheet: usize, row: usize, column: usize) -> &'a str {
        self.cells(sheet).get(row).and_then(|cells| cells.get(column)).map_or("", String::as_str)
    }

    fn cell(&mut self, sheet: usize, row: usize, column: usize) -> Value {
        if let Some(value) = self.done.get(&(sheet, row, column)) {
            return value.clone();
        }
        if self.visiting.contains(&(sheet, row, column)) {
            return Value::Error(CellError::Cycle);
        }
        if self.visiting.len() >= DEPTH {
            self.deep = true;
            return Value::Error(CellError::Num);
        }
        let deep_before = std::mem::take(&mut self.deep);
        let raw = self.raw(sheet, row, column);
        let value = match raw.strip_prefix('=') {
            Some(formula) => {
                self.visiting.push((sheet, row, column));
                // Its own references mean its own sheet, whichever asked.
                let asked_from = std::mem::replace(&mut self.current, sheet);
                let value = match parse(formula) {
                    Ok(expr) => match self.value(&expr) {
                        Value::Empty => Value::Number(0.0),
                        value => value,
                    },
                    Err(error) => Value::Error(error),
                };
                self.current = asked_from;
                self.visiting.pop();
                value
            }
            None => Value::of_typed(raw),
        };
        // A value that ran out of depth isn't the cell's; a later pass may reach it.
        if !self.deep {
            self.done.insert((sheet, row, column), value.clone());
        }
        self.deep |= deep_before;
        value
    }

    fn range(&mut self, expr: &Expr) -> Vec<Value> {
        match expr {
            Expr::Range(named, top, left, bottom, right) => {
                let Ok(sheet) = self.sheet_of(*named) else { return vec![Value::Error(CellError::Ref)] };
                let mut values = Vec::new();
                for (row, columns) in self.clamped(sheet, *top, *left, *bottom, *right) {
                    for column in columns {
                        values.push(self.cell(sheet, row, column));
                    }
                }
                values
            }
            other => vec![self.value(other)],
        }
    }

    fn rows_of(&mut self, expr: &Expr) -> Result<Vec<Vec<Value>>, CellError> {
        match expr {
            Expr::Range(named, top, left, bottom, right) => {
                let sheet = self.sheet_of(*named)?;
                let rows = self.clamped(sheet, *top, *left, *bottom, *right);
                Ok(rows.into_iter().map(|(row, columns)| columns.map(|column| self.cell(sheet, row, column)).collect()).collect())
            }
            Expr::Ref(named, row, column) => {
                let sheet = self.sheet_of(*named)?;
                Ok(vec![vec![self.cell(sheet, *row, *column)]])
            }
            _ => Err(CellError::Value),
        }
    }

    fn value(&mut self, expr: &Expr) -> Value {
        match self.try_value(expr) {
            // Nothing infinite or undefined leaves a formula.
            Ok(Value::Number(number)) if !number.is_finite() => Value::Error(CellError::Num),
            Ok(value) => value,
            Err(error) => Value::Error(error),
        }
    }

    fn number(&mut self, expr: &Expr) -> Result<f64, CellError> {
        self.value(expr).number()
    }

    fn try_value(&mut self, expr: &Expr) -> Result<Value, CellError> {
        Ok(match expr {
            Expr::Value(value) => value.clone(),
            Expr::Ref(named, row, column) => {
                if *row >= ROWS || *column >= COLUMNS {
                    return Err(CellError::Ref);
                }
                let sheet = self.sheet_of(*named)?;
                self.cell(sheet, *row, *column)
            }
            Expr::Range(..) => return Err(CellError::Value),
            Expr::Negate(inner) => Value::Number(-self.number(inner)?),
            Expr::Percent(inner) => Value::Number(self.number(inner)? / 100.0),
            Expr::Binary(op, left, right) => {
                let (a, b) = (self.value(left), self.value(right));
                if let Value::Error(error) = a {
                    return Err(error);
                }
                if let Value::Error(error) = b {
                    return Err(error);
                }
                match *op {
                    "&" => {
                        let (a, b) = (a.text()?, b.text()?);
                        if a.len() + b.len() > TEXT_BYTES {
                            return Err(CellError::Value);
                        }
                        Value::Text(a + &b)
                    }
                    "+" => Value::Number(a.number()? + b.number()?),
                    "-" => Value::Number(a.number()? - b.number()?),
                    "*" => Value::Number(a.number()? * b.number()?),
                    "/" => {
                        let divisor = b.number()?;
                        if divisor == 0.0 {
                            return Err(CellError::Div0);
                        }
                        Value::Number(a.number()? / divisor)
                    }
                    "^" => {
                        let power = a.number()?.powf(b.number()?);
                        if !power.is_finite() {
                            return Err(CellError::Num);
                        }
                        Value::Number(power)
                    }
                    comparison => Value::Bool(compare(&a, &b, comparison)),
                }
            }
            Expr::Call(name, args) => self.call(name, args)?,
        })
    }

    fn numbers(&mut self, args: &[Expr]) -> Result<Vec<f64>, CellError> {
        let mut numbers = Vec::new();
        for arg in args {
            let direct = !matches!(arg, Expr::Range(..));
            for value in self.range(arg) {
                match value {
                    Value::Number(number) => numbers.push(number),
                    Value::Error(error) => return Err(error),
                    Value::Bool(_) | Value::Text(_) if direct => numbers.push(value.number()?),
                    _ => {}
                }
            }
        }
        Ok(numbers)
    }

    fn call(&mut self, name: &str, args: &[Expr]) -> Result<Value, CellError> {
        let arg = |at: usize| args.get(at).ok_or(CellError::Value);
        Ok(match name {
            "SUM" => Value::Number(self.numbers(args)?.iter().sum()),
            "AVERAGE" => {
                let numbers = self.numbers(args)?;
                if numbers.is_empty() {
                    return Err(CellError::Div0);
                }
                Value::Number(numbers.iter().sum::<f64>() / numbers.len() as f64)
            }
            "MIN" => Value::Number(self.numbers(args)?.into_iter().reduce(f64::min).unwrap_or(0.0)),
            "MAX" => Value::Number(self.numbers(args)?.into_iter().reduce(f64::max).unwrap_or(0.0)),
            "COUNT" => Value::Number(
                args.iter().flat_map(|arg| self.range(arg)).filter(|value| matches!(value, Value::Number(_))).count() as f64,
            ),
            "COUNTA" => {
                Value::Number(args.iter().flat_map(|arg| self.range(arg)).filter(|value| *value != Value::Empty).count() as f64)
            }
            "IF" => {
                if self.value(arg(0)?).truthy()? {
                    self.value(arg(1)?)
                } else {
                    args.get(2).map_or(Value::Bool(false), |otherwise| self.value(otherwise))
                }
            }
            "IFS" => {
                for pair in args.chunks(2) {
                    let [condition, then] = pair else { return Err(CellError::Value) };
                    if self.value(condition).truthy()? {
                        return Ok(self.value(then));
                    }
                }
                return Err(CellError::Na);
            }
            "AND" | "OR" => {
                let mut values = Vec::new();
                for arg in args {
                    for value in self.range(arg) {
                        if value != Value::Empty {
                            values.push(value.truthy()?);
                        }
                    }
                }
                Value::Bool(if name == "AND" { values.iter().all(|value| *value) } else { values.iter().any(|value| *value) })
            }
            "NOT" => Value::Bool(!self.value(arg(0)?).truthy()?),
            "ROUND" => {
                // Past 15 digits either way, rounding changes nothing a double holds.
                let digits = (args.get(1).map_or(Ok(0.0), |digits| self.number(digits))?.trunc() as i32).clamp(-15, 15);
                let scale = 10f64.powi(digits);
                Value::Number((self.number(arg(0)?)? * scale).round() / scale)
            }
            "ABS" => Value::Number(self.number(arg(0)?)?.abs()),
            "CONCAT" => {
                let mut text = String::new();
                for arg in args {
                    for value in self.range(arg) {
                        let more = value.text()?;
                        if text.len() + more.len() > TEXT_BYTES {
                            return Err(CellError::Value);
                        }
                        text.push_str(&more);
                    }
                }
                Value::Text(text)
            }
            "TEXT" => {
                let number = self.number(arg(0)?)?;
                let format = self.value(arg(1)?).text()?;
                Value::Text(formatted(number, &format))
            }
            "LEFT" | "RIGHT" => {
                let text = self.value(arg(0)?).text()?;
                let count = args.get(1).map_or(Ok(1.0), |count| self.number(count))?;
                if count < 0.0 {
                    return Err(CellError::Value);
                }
                let count = count as usize;
                let chars: Vec<char> = text.chars().collect();
                let taken: String = if name == "LEFT" {
                    chars.iter().take(count).collect()
                } else {
                    chars[chars.len().saturating_sub(count)..].iter().collect()
                };
                Value::Text(taken)
            }
            "LEN" => Value::Number(self.value(arg(0)?).text()?.chars().count() as f64),
            "TODAY" => Value::Number(self.book.today),
            "DATE" => {
                let (year, month, day) = (self.number(arg(0)?)?, self.number(arg(1)?)?, self.number(arg(2)?)?);
                // Spreadsheets take years 0 to 9999; months and days roll over within reason.
                if !(0.0..=9999.0).contains(&year) || month.abs() > 120_000.0 || day.abs() > 4_000_000.0 {
                    return Err(CellError::Num);
                }
                Value::Number(date_serial(year as i64, month as i64, day as i64))
            }
            "VLOOKUP" => {
                let key = self.value(arg(0)?);
                let table = self.rows_of(arg(1)?)?;
                let column = self.number(arg(2)?)? as usize;
                let exact = args.get(3).map_or(Ok(false), |exact| self.value(exact).truthy().map(|approximate| !approximate))?;
                if column == 0 || table.first().is_none_or(|row| column > row.len()) {
                    return Err(CellError::Ref);
                }
                let found = if exact {
                    table.iter().find(|row| compare(&row[0], &key, "="))
                } else {
                    table.iter().take_while(|row| compare(&row[0], &key, "<=")).last()
                };
                found.map(|row| row[column - 1].clone()).ok_or(CellError::Na)?
            }
            "XLOOKUP" => {
                let key = self.value(arg(0)?);
                let keys = self.range(arg(1)?);
                let results = self.range(arg(2)?);
                match keys.iter().position(|candidate| compare(candidate, &key, "=")) {
                    Some(at) => results.get(at).cloned().ok_or(CellError::Ref)?,
                    None => match args.get(3) {
                        Some(otherwise) => self.value(otherwise),
                        None => return Err(CellError::Na),
                    },
                }
            }
            "SUMIF" | "COUNTIF" => {
                let tested = self.range(arg(0)?);
                let criterion = self.value(arg(1)?);
                let summed = match args.get(2) {
                    Some(range) => self.range(range),
                    None => tested.clone(),
                };
                let matching: Vec<usize> =
                    tested.iter().enumerate().filter(|(_, value)| meets(value, &criterion)).map(|(at, _)| at).collect();
                if name == "COUNTIF" {
                    Value::Number(matching.len() as f64)
                } else {
                    Value::Number(matching.iter().filter_map(|at| summed.get(*at)).filter_map(|value| value.number().ok()).sum())
                }
            }
            _ => return Err(CellError::Name),
        })
    }
}

/// Whether `a op b`: numbers by value, text without case, numbers before text.
fn compare(a: &Value, b: &Value, op: &str) -> bool {
    use std::cmp::Ordering;
    let order = match (a, b) {
        (Value::Number(_) | Value::Empty | Value::Bool(_), Value::Number(_) | Value::Empty | Value::Bool(_)) => {
            a.number().unwrap_or(0.0).partial_cmp(&b.number().unwrap_or(0.0)).unwrap_or(Ordering::Equal)
        }
        (Value::Text(x), Value::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Value::Text(x), Value::Empty) => {
            if x.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        (Value::Empty, Value::Text(y)) => {
            if y.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        (Value::Text(_), _) => Ordering::Greater,
        (_, Value::Text(_)) => Ordering::Less,
        _ => Ordering::Equal,
    };
    match op {
        "=" => order == Ordering::Equal,
        "<>" => order != Ordering::Equal,
        "<" => order == Ordering::Less,
        ">" => order == Ordering::Greater,
        "<=" => order != Ordering::Greater,
        ">=" => order != Ordering::Less,
        _ => false,
    }
}

/// Whether `value` meets a `SUMIF` criterion such as `">5"`, `"<>done"` or `3`.
fn meets(value: &Value, criterion: &Value) -> bool {
    let Value::Text(text) = criterion else {
        return compare(value, criterion, "=");
    };
    let (op, operand) = ["<>", "<=", ">=", "=", "<", ">"]
        .iter()
        .find_map(|op| text.strip_prefix(op).map(|rest| (*op, Value::of_typed(rest))))
        .unwrap_or(("=", Value::of_typed(text)));
    // A number is compared with numbers only, and text with text.
    match (&operand, value) {
        (Value::Number(_), Value::Number(_)) | (Value::Text(_), Value::Text(_)) => compare(value, &operand, op),
        (Value::Empty, _) => compare(value, &operand, op),
        _ => op == "<>",
    }
}

/// Days since 1899-12-30 of a date, as spreadsheets count them.
pub fn date_serial(year: i64, month: i64, day: i64) -> f64 {
    // Months past 12 or below 1 roll into the years, as spreadsheets do.
    let months = year * 12 + (month - 1);
    let (year, month) = (months.div_euclid(12), months.rem_euclid(12) + 1);
    let days_from_civil = |y: i64, m: i64, d: i64| {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    };
    (days_from_civil(year, month, day) - days_from_civil(1899, 12, 30)) as f64
}

/// The date formats a column may show date serials in, as a spreadsheet
/// app reads them too (ADR 0083).
pub const DATE_FORMATS: &[&str] = &["yyyy-mm-dd", "d mmm yyyy", "mmm yyyy", "dd/mm/yyyy", "mm/dd/yyyy"];

/// The year, month and day of date serial `serial`, days since 1899-12-30
/// as spreadsheets count them; none past what a date can be.
pub fn civil_date(serial: f64) -> Option<(i64, u32, u32)> {
    if !serial.is_finite() || serial.abs() > 3_000_000.0 {
        return None;
    }
    // Days since 1970-01-01, then Howard Hinnant's civil_from_days.
    let z = serial.floor() as i64 - 25_569 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some((year, u32::try_from(month).ok()?, u32::try_from(day).ok()?))
}

fn date_text(serial: f64, format: &str) -> Option<String> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (year, month, day) = civil_date(serial)?;
    let name = MONTHS[usize::try_from(month).ok()?.checked_sub(1)?];
    Some(match format {
        "yyyy-mm-dd" => format!("{year:04}-{month:02}-{day:02}"),
        "d mmm yyyy" => format!("{day} {name} {year}"),
        "mmm yyyy" => format!("{name} {year}"),
        "dd/mm/yyyy" => format!("{day:02}/{month:02}/{year:04}"),
        "mm/dd/yyyy" => format!("{month:02}/{day:02}/{year:04}"),
        _ => return None,
    })
}

/// Whether `format` is one a column may show its numbers in (ADR 0083):
/// decimals (`0`, `0.00`), thousands (`#,##0.0`), a percentage (`0%`,
/// `0.0%`), those after a currency sign (`$#,##0.00`), and
/// [`DATE_FORMATS`] for date serials.
pub fn number_format_ok(format: &str) -> bool {
    if DATE_FORMATS.contains(&format) {
        return true;
    }
    let body = format.strip_prefix(['$', '€', '£', '¥']).unwrap_or(format);
    let body = body.strip_suffix('%').unwrap_or(body);
    let (whole, decimals) = body.split_once('.').unwrap_or((body, ""));
    matches!(whole, "0" | "#,##0") && decimals.len() <= 6 && decimals.chars().all(|c| c == '0') && !body.ends_with('.')
}

/// `number` in a column's `format`, as [`number_format_ok`] takes them;
/// a currency's sign goes after the minus.
pub fn format_number(number: f64, format: &str) -> String {
    if DATE_FORMATS.contains(&format) {
        return date_text(number, format).unwrap_or_else(|| Value::Number(number).to_string());
    }
    match format.chars().next().filter(|c| "$€£¥".contains(*c)) {
        Some(symbol) => {
            let shown = formatted(number.abs(), &format[symbol.len_utf8()..]);
            format!("{}{symbol}{shown}", if number < 0.0 { "-" } else { "" })
        }
        None => formatted(number, format),
    }
}

/// `number` as a `TEXT` format shows it: decimals from the zeros after a
/// point, thousands with a comma, and a percentage with `%`.
fn formatted(number: f64, format: &str) -> String {
    let percent = format.ends_with('%');
    let value = if percent { number * 100.0 } else { number };
    let decimals = format.split_once('.').map_or(0, |(_, after)| after.chars().filter(|c| *c == '0' || *c == '#').count());
    // Halves away from zero, as a spreadsheet rounds, where formatting rounds them to even.
    let factor = 10f64.powi(i32::try_from(decimals).unwrap_or(0));
    let rounded = (value * factor).round() / factor;
    let value = if rounded.is_finite() { rounded } else { value };
    let mut text = format!("{value:.decimals$}");
    if format.contains(',') {
        let (whole, fraction) = text.split_once('.').map_or((text.as_str(), None), |(whole, fraction)| (whole, Some(fraction)));
        let negative = whole.starts_with('-');
        let digits: Vec<char> = whole.trim_start_matches('-').chars().collect();
        let mut grouped = String::new();
        for (at, digit) in digits.iter().enumerate() {
            if at > 0 && (digits.len() - at).is_multiple_of(3) {
                grouped.push(',');
            }
            grouped.push(*digit);
        }
        text = format!(
            "{}{grouped}{}",
            if negative { "-" } else { "" },
            fraction.map(|fraction| format!(".{fraction}")).unwrap_or_default()
        );
    }
    if percent {
        text.push('%');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|row| row.iter().map(|cell| (*cell).to_owned()).collect()).collect()
    }

    #[test]
    fn a_column_shows_its_numbers_in_its_format() {
        assert_eq!(format_number(0.5, "0%"), "50%");
        assert_eq!(format_number(1234.5, "#,##0.00"), "1,234.50");
        assert_eq!(format_number(-1234.5, "$#,##0"), "-$1,235");
        assert_eq!(format_number(2.71, "0.0"), "2.7");
        assert!(number_format_ok("€#,##0.00") && number_format_ok("0.0%") && number_format_ok("0"));
        assert!(!number_format_ok("yyyy") && !number_format_ok("0.") && !number_format_ok("#,##0.0#") && !number_format_ok(""));
        let day = date_serial(2026, 11, 16);
        assert_eq!(format_number(day, "yyyy-mm-dd"), "2026-11-16");
        assert_eq!(format_number(day + 0.75, "d mmm yyyy"), "16 Nov 2026", "a time of day stays the day");
        assert_eq!(format_number(date_serial(2024, 2, 29), "dd/mm/yyyy"), "29/02/2024");
        assert_eq!(format_number(date_serial(1900, 3, 1), "mm/dd/yyyy"), "03/01/1900");
        assert_eq!(format_number(date_serial(2026, 1, 5), "mmm yyyy"), "Jan 2026");
        for serial in [-700_000.0, -1.0, 0.0, 59.0, 60.0, 45_000.0, 2_900_000.0] {
            let (year, month, day) = civil_date(serial).expect("a date");
            assert_eq!(date_serial(year, i64::from(month), i64::from(day)), serial, "{serial} round-trips");
        }
    }

    #[test]
    fn a_formula_reaches_another_sheets_cells() {
        let first = cells(&[&["=Sheet2!A1*2", "=SUM(Sheet2!A1:A3)", "=sheet2!B1"], &["=Sheet3!A1", "=Sheet2!C1", ""]]);
        let second = cells(&[&["5", "=Sheet1!A1+1", "=Sheet1!B2"], &["6", "", ""], &["7", "", ""]]);
        let book = Book { sheets: &[&first, &second], today: 46_000.0 };
        let values = evaluate_in(&book, 0);
        assert_eq!(values[0][0], Value::Number(10.0));
        assert_eq!(values[0][1], Value::Number(18.0));
        assert_eq!(values[0][2], Value::Number(11.0), "its own references mean its own sheet");
        assert_eq!(values[1][0], Value::Error(CellError::Ref), "no third sheet");
        assert_eq!(values[1][1], Value::Error(CellError::Cycle), "a cycle across sheets is a cycle");
        assert_eq!(evaluate_in(&book, 1)[0][1], Value::Number(11.0));
    }

    #[test]
    fn a_reference_to_another_sheet_keeps_that_sheets_ids() {
        let (rows_1, columns_1) = (vec!["r1".to_owned()], vec!["c1".to_owned()]);
        let (rows_2, columns_2) = (vec!["x1".to_owned(), "x2".to_owned()], vec!["y1".to_owned(), "y2".to_owned()]);
        let axes: [(&[String], &[String]); 2] = [(&rows_1, &columns_1), (&rows_2, &columns_2)];
        let kept = to_ids_in("=Sheet2!B2+A1+SUM(Sheet2!A1:B2)", &axes, 0);
        assert_eq!(kept, "=⟨1:x2,y2⟩+⟨r1,c1⟩+SUM(⟨1:x1,y1⟩:⟨1:x2,y2⟩)");
        // A row goes in at the top of Sheet2: the reference follows it.
        let moved = vec!["x0".to_owned(), "x1".to_owned(), "x2".to_owned()];
        let shown: [(&[String], &[String]); 2] = [(&rows_1, &columns_1), (&moved, &columns_2)];
        assert_eq!(Positions::in_book(&shown, 0).from_ids(&kept), "=Sheet2!B3+A1+SUM(Sheet2!A2:B3)");
        assert_eq!(to_ids_in("=Sheet9!A1", &axes, 0), "=#REF!", "no such sheet");
        assert!(formula_ok("=SUM(Sheet2!A1:B2)") && !formula_ok("=Sheet2!"));
        assert_eq!(addresses_in("=Sheet2!C9+B2"), vec![(1, 1)], "only its own sheet's cells grow it");
    }

    fn sheet(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|row| row.iter().map(|cell| (*cell).to_owned()).collect()).collect()
    }

    fn values(rows: &[&[&str]]) -> Vec<Vec<String>> {
        let cells = sheet(rows);
        evaluate(&Sheet { cells: &cells, today: 46_000.0 })
            .iter()
            .map(|row| row.iter().map(ToString::to_string).collect())
            .collect()
    }

    /// `rows` evaluated on a thread with a worker's small stack, as the
    /// server and the daemon evaluate sheets.
    fn on_small_stack(rows: Vec<Vec<String>>) -> Vec<Vec<String>> {
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                evaluate(&Sheet { cells: &rows, today: 46_000.0 })
                    .iter()
                    .map(|row| row.iter().map(ToString::to_string).collect())
                    .collect()
            })
            .expect("spawns")
            .join()
            .expect("no overflow")
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let parens = format!("={}1{}", "(".repeat(10_000), ")".repeat(10_000));
        let signs = format!("={}1", "-".repeat(10_000));
        let calls = format!("={}1{}", "ABS(".repeat(5_000), ")".repeat(5_000));
        let shown = on_small_stack(vec![vec![parens, signs, calls, "=((((1))))".into()]]);
        assert_eq!(shown[0], ["#VALUE!", "#VALUE!", "#VALUE!", "1"]);
    }

    #[test]
    fn a_long_chain_of_references_is_reached_not_called_a_cycle() {
        // A running total over 600 rows, summed above it.
        let mut rows = vec![vec!["1".to_owned(), "=B602".to_owned()], vec!["1".to_owned(), "=A2".to_owned()]];
        for row in 3..=602 {
            rows.push(vec!["1".to_owned(), format!("=B{}+A{row}", row - 1)]);
        }
        let shown = on_small_stack(rows);
        assert_eq!(shown[0][1], "601");
        assert_eq!(values(&[&["=A1"]])[0][0], "#CYCLE!", "a real cycle still is one");
    }

    #[test]
    fn what_a_formula_could_grow_is_bounded() {
        // Text that doubles each row stops at the cap.
        let mut rows = vec![vec!["abcdefgh".to_owned()]];
        for row in 2..=30 {
            rows.push(vec![format!("=A{}&A{}", row - 1, row - 1)]);
        }
        let shown = on_small_stack(rows);
        assert_eq!(shown[29][0], "#VALUE!");
        // A range past the sheet reads only the sheet, quickly.
        let started = std::time::Instant::now();
        let huge = values(&[&["=SUM(A2:GS10000)+SUM(A2:GS10000)", "=VLOOKUP(2,B2:C100000000,1,FALSE)"], &["1", "2"]]);
        assert_eq!((huge[0][0].as_str(), huge[0][1].as_str()), ("6", "2"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn numbers_read_and_show_as_spreadsheets_do() {
        let shown =
            values(&[&["=2^3^2", "=1e5", "=2.5E-1", "=SUM(B2:B3)", "=-0", "=ROUND(1,400)", "=DATE(1e300,1,1)", "=10^400"]]);
        assert_eq!(shown[0], ["64", "100000", "0.25", "0", "0", "1", "#NUM!", "#NUM!"]);
    }

    #[test]
    fn addresses_name_rows_and_columns_from_zero() {
        assert_eq!(
            (column_name(0), column_name(25), column_name(26), column_name(701)),
            ("A".into(), "Z".into(), "AA".into(), "ZZ".into())
        );
        assert_eq!(parse_address("$AB$12"), Some((11, 27)));
        assert_eq!(parse_address("A0"), None);
        assert_eq!(column_index("ZZ"), Some(701));
    }

    #[test]
    fn arithmetic_follows_precedence_and_errors_carry() {
        let got = values(&[&["2", "3", "=A1+B1*2", "=(A1+B1)*2", "=-A1^2", "=B1/0", "=C1&\"x\"", "=50%"]]);
        // A leading minus binds before a power, as in spreadsheets: (-2)^2.
        assert_eq!(got[0][2..], ["8", "10", "4", "#DIV/0!", "8x", "0.5"]);
        assert_eq!(values(&[&["=1/0", "=A1+1"]])[0][1], "#DIV/0!");
        assert_eq!(values(&[&["=NOPE(1)"]])[0][0], "#NAME?");
        assert_eq!(values(&[&["=1+"]])[0][0], "#VALUE!");
    }

    #[test]
    fn a_cycle_is_an_error_not_a_hang() {
        let got = values(&[&["=B1", "=A1", "=C1"]]);
        assert_eq!(got[0], ["#CYCLE!", "#CYCLE!", "#CYCLE!"]);
    }

    #[test]
    fn the_functions_compute_as_spreadsheets_do() {
        let rows: &[&[&str]] = &[
            &["apples", "3", "done"],
            &["pears", "5", "open"],
            &["plums", "", "done"],
            &["figs", "x", "open"],
            &[
                "=SUM(B1:B4)",
                "=AVERAGE(B1:B3)",
                "=MIN(B1:B2)",
                "=MAX(B1:B4)",
                "=COUNT(B1:B4)",
                "=COUNTA(B1:B4)",
                "=IF(B1>4,\"big\",\"small\")",
                "=IFS(B2>9,\"a\",B2>4,\"b\")",
                "=AND(TRUE,B1>1)",
                "=OR(FALSE,B1>9)",
                "=NOT(TRUE)",
                "=ROUND(2.345,2)",
                "=ABS(-3)",
                "=CONCAT(A1,\"-\",A2)",
                "=TEXT(1234.5,\"#,##0.00\")",
                "=LEFT(A1,3)",
                "=RIGHT(A1)",
                "=LEN(A2)",
                "=DATE(2026,11,15)",
                "=VLOOKUP(\"pears\",A1:C4,2,FALSE)",
                "=XLOOKUP(\"plums\",A1:A4,C1:C4)",
                "=SUMIF(C1:C4,\"done\",B1:B4)",
                "=COUNTIF(B1:B4,\">2\")",
                "=TODAY()",
                "=TEXT(0.256,\"0.0%\")",
                "=XLOOKUP(\"kiwi\",A1:A4,C1:C4,\"none\")",
            ],
        ];
        let got = &values(rows)[4];
        let expected = [
            "8",
            "4",
            "3",
            "5",
            "2",
            "3",
            "small",
            "b",
            "TRUE",
            "FALSE",
            "FALSE",
            "2.35",
            "3",
            "apples-pears",
            "1,234.50",
            "app",
            "s",
            "5",
            "46341",
            "5",
            "done",
            "3",
            "2",
            "46000",
            "25.6%",
            "none",
        ];
        for (at, (got, expected)) in got.iter().zip(expected).enumerate() {
            assert_eq!(got, expected, "{}", rows[4][at]);
        }
    }

    #[test]
    fn a_formula_kept_by_ids_keeps_its_meaning_when_a_row_is_inserted() {
        let rows = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        let columns = rows(&["c0", "c1"]);
        let kept = to_ids("=SUM(A1:A2)*B2", &rows(&["r0", "r1"]), &columns);
        assert_eq!(kept, "=SUM(⟨r0,c0⟩:⟨r1,c0⟩)*⟨r1,c1⟩");
        // A row goes in above: the same cells are now a row down.
        assert_eq!(from_ids(&kept, &rows(&["new", "r0", "r1"]), &columns), "=SUM(A2:A3)*B3");
        // A row it named is gone.
        assert_eq!(from_ids(&kept, &rows(&["r0"]), &columns), "=SUM(A1:#REF!)*#REF!");
        assert_eq!(to_ids("plain text A1", &rows(&["r0"]), &columns), "plain text A1", "only formulas are rewritten");
        assert_eq!(to_ids("=\"A1\"&A1", &rows(&["r0"]), &columns), "=\"A1\"&⟨r0,c0⟩", "text in quotes is left alone");
    }

    /// Random sheets of numbers: `SUM`, `COUNT` and `MAX` over a column agree
    /// with adding, counting and comparing them by hand.
    #[test]
    fn sums_counts_and_maxima_agree_with_the_numbers_by_hand() {
        let mut seed: u64 = 0x5eed;
        let mut next = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            seed >> 33
        };
        for _ in 0..200 {
            let rows = 1 + (next() % 30) as usize;
            let numbers: Vec<Option<i64>> = (0..rows).map(|_| (next() % 4 != 0).then(|| (next() % 2001) as i64 - 1000)).collect();
            let mut cells: Vec<Vec<String>> =
                numbers.iter().map(|number| vec![number.map(|n| n.to_string()).unwrap_or_default()]).collect();
            cells.push(vec![format!("=SUM(A1:A{rows})"), format!("=COUNT(A1:A{rows})"), format!("=MAX(A1:A{rows})")]);
            let got = evaluate(&Sheet { cells: &cells, today: 0.0 });
            let present: Vec<i64> = numbers.iter().flatten().copied().collect();
            assert_eq!(got[rows][0], Value::Number(present.iter().sum::<i64>() as f64));
            assert_eq!(got[rows][1], Value::Number(present.len() as f64));
            assert_eq!(got[rows][2], Value::Number(present.iter().copied().max().unwrap_or(0) as f64));
        }
    }

    #[test]
    fn dates_count_days_as_spreadsheets_do() {
        assert_eq!(date_serial(1900, 3, 1), 61.0);
        assert_eq!(date_serial(2026, 1, 1), 46_023.0);
        assert_eq!(date_serial(2025, 13, 1), date_serial(2026, 1, 1), "month 13 rolls over");
    }
}
