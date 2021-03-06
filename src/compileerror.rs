use std::error::Error;
use std::convert::From;
use std::iter::repeat;
use std::fs::File;
use std::io;
use std::io::BufRead;
use std::fmt;
use ast::Type;
use span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorData
{
    pub span: Span,
    pub msg: String,
}

impl ErrorData
{
    pub fn new<S: Into<String>>(span: &Span, msg: S) -> ErrorData
    {
        ErrorData{
            span: span.clone(),
            msg: msg.into(),
        }
    }
}

impl fmt::Display for ErrorData
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        writeln!(f, "{}: {}", self.span, self.msg)
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError
{
    Other(String),
    IO(String),
    Parse(ErrorData),
    Type(ErrorData),
    UnknownName(ErrorData),
    UnknownType(String, Type), // Name and expected type
    Many(Vec<CompileError>),
}

impl CompileError
{
    pub fn print(&self)
    {
        match *self
        {
            CompileError::Other(ref msg) |
            CompileError::IO(ref msg) => println!("{}", msg),
            CompileError::Parse(ref ed) |
            CompileError::Type(ref ed) |
            CompileError::UnknownName(ref ed) => print_message(&ed.msg, &ed.span),
            CompileError::UnknownType(ref name, ref typ) => println!("{} has unknown type, expecting {}", name, typ),
            CompileError::Many(ref errors) => {
                for e in errors {
                    e.print();
                }
            }
        }
    }
}

impl Error for CompileError
{
    fn description(&self) -> &str {"CompileError"}
}

impl fmt::Display for CompileError
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        match *self
        {
            CompileError::Other(ref msg) |
            CompileError::IO(ref msg) => writeln!(f, "{}", msg),
            CompileError::Parse(ref ed) |
            CompileError::Type(ref ed) |
            CompileError::UnknownName(ref ed) => ed.fmt(f),
            CompileError::UnknownType(ref name, ref typ) => writeln!(f, "{} has unknown type, expecting {}", name, typ),
            CompileError::Many(ref errors) => {
                for err in errors {
                    err.fmt(f)?;
                }
                Ok(())
            }
        }
    }
}

pub fn print_message(msg: &str, span: &Span)
{
    fn repeat_string(s: &str, count: usize) -> String
    {
        repeat(s).take(count).collect()
    }

    let prefix = "| ";
    println!("{}: {}", span, msg);
    if let Ok(file) = File::open(&span.file) {
        let start_line = if span.start.line >= 4 {span.start.line - 4} else {0};
        let reader = io::BufReader::new(file);

        for (idx, line) in reader.lines().enumerate().skip(start_line)
        {
            let line = line.unwrap();
            let line_idx = idx + 1;
            println!("{:>4} {}{}", line_idx, prefix, line);
            if line_idx == span.start.line
            {
                let end = if line_idx == span.end.line {span.end.offset} else {line.len()};
                let carets = repeat_string("^", end - span.start.offset + 1);
                let whitespace = repeat_string(" ", span.start.offset - 1);
                println!("     {}{}{}", prefix, whitespace, carets);
            }
            else if line_idx == span.end.line
            {
                let carets = repeat_string("^", span.end.offset);
                println!("     {}{}", prefix, carets);
            }
            else if line_idx > span.start.line && line_idx < span.end.line && !line.is_empty()
            {
                let carets = repeat_string("^", line.len());
                println!("     {}{}", prefix, carets);
            }

            if line_idx >= span.end.line + 3 {break;}
        }
    }
}

pub type CompileResult<T> = Result<T, CompileError>;

pub fn parse_error_result<T, Msg: Into<String>>(span: &Span, msg: Msg) -> CompileResult<T>
{
    Err(CompileError::Parse(ErrorData::new(span, msg.into())))
}

pub fn type_error_result<T, Msg: Into<String>>(span: &Span, msg: Msg) -> CompileResult<T>
{
    Err(CompileError::Type(ErrorData::new(span, msg.into())))
}

pub fn type_error<Msg: Into<String>>(span: &Span, msg: Msg) -> CompileError
{
    CompileError::Type(ErrorData::new(span, msg))
}

pub fn unknown_name<Msg: Into<String>>(span: &Span, msg: Msg) -> CompileError
{
    CompileError::UnknownName(ErrorData::new(span, msg))
}

pub fn unknown_name_result<T, Msg: Into<String>>(span: &Span, msg: Msg) -> CompileResult<T>
{
    Err(CompileError::UnknownName(ErrorData::new(span, msg)))
}

pub fn unknown_type_result<T>(name: &str, typ: &Type) -> CompileResult<T>
{
    Err(CompileError::UnknownType(name.into(), typ.clone()))
}

impl From<io::Error> for CompileError
{
    fn from(e: io::Error) -> Self
    {
        CompileError::IO(format!("IO Error: {}", e))
    }
}

impl From<String> for CompileError
{
    fn from(e: String) -> Self
    {
        CompileError::Other(e)
    }
}

