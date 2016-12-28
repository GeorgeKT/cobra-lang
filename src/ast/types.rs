use std::fmt;
use std::ops::Deref;
use std::hash::{Hasher, Hash};
use std::rc::Rc;
use itertools::free::join;
use ast::{Expression, TreePrinter, MemberAccessType, Property, prefix};
use span::Span;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct SumTypeCase
{
    pub name: String,
    pub typ: Type,
}

pub trait SumTypeCaseIndexOf
{
    fn index_of(&self, case_name: &str) -> Option<usize>;
    fn num_cases(&self) -> usize;
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct SumType
{
    pub name: String,
    pub cases: Vec<SumTypeCase>,
}

impl SumTypeCaseIndexOf for SumType
{
    fn index_of(&self, case_name: &str) -> Option<usize>
    {
        self.cases.iter().position(|cn| cn.name == case_name)
    }

    fn num_cases(&self) -> usize
    {
        self.cases.len()
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct EnumType
{
    pub name: String,
    pub cases: Vec<String>,
}

impl SumTypeCaseIndexOf for EnumType
{
    fn index_of(&self, case_name: &str) -> Option<usize>
    {
        self.cases.iter().position(|cn| cn == case_name)
    }

    fn num_cases(&self) -> usize
    {
        self.cases.len()
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StructMember
{
    pub name: String,
    pub typ: Type,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StructType
{
    pub name: String,
    pub members: Vec<StructMember>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct FuncType
{
    pub args: Vec<Type>,
    pub return_type: Type,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct ArrayType
{
    pub element_type: Type,
    pub len: usize,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct SliceType
{
    pub element_type: Type,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct UnresolvedType
{
    pub name: String,
    pub generic_args: Vec<Type>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Type
{
    Void,
    Unknown,
    Int,
    Float,
    Char,
    Bool,
    String,
    Pointer(Rc<Type>),
    Unresolved(Rc<UnresolvedType>),
    Array(Rc<ArrayType>),
    Slice(Rc<SliceType>),
    Generic(String),
    Func(Rc<FuncType>),
    Struct(Rc<StructType>),
    Sum(Rc<SumType>),
    Enum(Rc<EnumType>),
}

#[derive(Debug,  Eq, PartialEq, Clone)]
pub struct TypeAlias
{
    pub name: String,
    pub original: Type,
    pub span: Span,
}


impl Type
{
    pub fn is_sequence(&self) -> bool
    {
        match *self
        {
            Type::Array(_) => true,
            Type::Slice(_) => true,
            Type::String => true,
            _ => false,
        }
    }

    pub fn get_element_type(&self) -> Option<Type>
    {
        match *self
        {
            Type::Array(ref at) => Some(at.element_type.clone()),
            Type::Slice(ref at) => Some(at.element_type.clone()),
            Type::String => Some(Type::Char),
            Type::Pointer(ref inner) => Some(inner.deref().clone()),
            _ => None,
        }
    }

    pub fn is_matchable(&self, other: &Type) -> bool
    {
        match (self, other)
        {
            (&Type::Array(ref a), &Type::Array(ref b)) => a.element_type == b.element_type,
            (&Type::Slice(ref a), &Type::Array(ref b)) => a.element_type == b.element_type,
            (&Type::Array(ref a), &Type::Slice(ref b)) => a.element_type == b.element_type,
            _ => *self == *other,
        }
    }

    // If possible generate a conversion expression
    pub fn convert(&self, from_type: &Type, _expr: &Expression) -> Option<Expression>
    {
        match (self, from_type)
        {
            _ => None,
        }
    }

    pub fn is_convertible(&self, dst_type: &Type) -> bool
    {
        match (self, dst_type)
        {
            _ => false,
        }
    }

    pub fn is_generic(&self) -> bool
    {
        match *self
        {
            Type::Generic(_) => true,
            Type::Array(ref at) => at.element_type.is_generic(),
            Type::Slice(ref st) => st.element_type.is_generic(),
            Type::Func(ref ft) => ft.return_type.is_generic() || ft.args.iter().any(|a| a.is_generic()),
            Type::Struct(ref st) => st.members.iter().any(|m| m.typ.is_generic()),
            Type::Sum(ref st) => st.cases.iter().any(|c| c.typ.is_generic()),
            Type::Unresolved(ref ut) => ut.generic_args.iter().any(|t| t.is_generic()),
            Type::Pointer(ref inner) => inner.is_generic(),
            _ => false,
        }
    }

    pub fn is_numeric(&self) -> bool
    {
        match *self
        {
            Type::Int | Type::Float => true,
            _ => false,
        }
    }

    pub fn is_integer(&self) -> bool
    {
        match *self
        {
            Type::Int => true,
            _ => false,
        }
    }

    pub fn is_bool(&self) -> bool
    {
        match *self
        {
            Type::Bool => true,
            _ => false,
        }
    }

    pub fn is_unknown(&self) -> bool
    {
        match *self
        {
            Type::Unknown => true,
            _ => false,
        }
    }

    pub fn get_property_type(&self, name: &str) -> Option<(Type, MemberAccessType)>
    {
        match *self
        {
            Type::Array(_) | Type::Slice(_) | Type::String => {
                match name
                {
                    "len" => Some((Type::Int, MemberAccessType::Property(Property::Len))),
                    _ => None,
                }
            },
            _ => None,
        }
    }
}

pub fn func_type(args: Vec<Type>, ret: Type) -> Type
{
    Type::Func(Rc::new(FuncType{
        args: args,
        return_type: ret,
    }))
}

pub fn array_type(element_type: Type, len: usize) -> Type
{
    Type::Array(Rc::new(ArrayType{
        element_type: element_type,
        len: len
    }))
}

pub fn slice_type(element_type: Type) -> Type
{
    Type::Slice(Rc::new(SliceType{
        element_type: element_type,
    }))
}

pub fn string_type() -> Type
{
    Type::String
}

pub fn sum_type_case(name: &str, typ: Type) -> SumTypeCase
{
    SumTypeCase{
        name: name.into(),
        typ: typ,
    }
}

pub fn sum_type(name: &str, cases: Vec<SumTypeCase>) -> Type
{
    Type::Sum(Rc::new(SumType{
        name: name.into(),
        cases: cases,
    }))
}

pub fn enum_type(name: &str, cases: Vec<String>) -> Type
{
    Type::Enum(Rc::new(EnumType{
        name: name.into(),
        cases: cases,
    }))
}

pub fn struct_type(name: &str, members: Vec<StructMember>) -> Type
{
    Type::Struct(Rc::new(StructType{
        name: name.into(),
        members: members,
    }))
}

pub fn ptr_type(inner: Type) -> Type
{
    Type::Pointer(Rc::new(inner))
}

pub fn struct_member(name: &str, typ: Type) -> StructMember
{
    StructMember{name: name.into(), typ: typ}
}

pub fn type_alias(name: &str, original: Type, span: Span) -> TypeAlias
{
    TypeAlias{
        name: name.into(),
        original: original,
        span: span,
    }
}

pub fn unresolved_type(name: &str, generic_args: Vec<Type>) -> Type
{
    Type::Unresolved(Rc::new(UnresolvedType{
        name: name.into(),
        generic_args: generic_args,
    }))
}

pub fn addition_type(at: &Type, bt: &Type) -> Option<Type>
{
    match (at, bt)
    {
        (&Type::Int, &Type::Int) => Some(Type::Int),
        (&Type::Float, &Type::Float) => Some(Type::Float),
        (&Type::Char, &Type::Char) => Some(Type::Char),
        _ => None,
    }

}

impl fmt::Display for Type
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        match *self
        {
            Type::Void => write!(f, "void"),
            Type::Unknown => write!(f, "unknown"),
            Type::Int => write!(f, "int"),
            Type::Float => write!(f, "float"),
            Type::Char => write!(f, "char"),
            Type::Bool => write!(f, "bool"),
            Type::String => write!(f, "string"),
            Type::Pointer(ref inner) => write!(f, "*{}", inner),
            Type::Unresolved(ref s) =>
                if s.generic_args.is_empty() {
                    write!(f, "{}", s.name)
                } else {
                    write!(f, "{}<{}>", s.name, join(s.generic_args.iter(), ","))
                },
            Type::Array(ref at) => write!(f, "{}[{}]", at.element_type, at.len),
            Type::Slice(ref at) => write!(f, "{}[]", at.element_type),
            Type::Generic(ref g) => write!(f, "${}", g),
            Type::Func(ref ft) => write!(f, "({}) -> {}", join(ft.args.iter(), ", "), ft.return_type),
            Type::Struct(ref st) => write!(f, "{{{}}}", join(st.members.iter(), ", ")),
            Type::Sum(ref st) => write!(f, "{}", join(st.cases.iter().map(|m| &m.typ), " | ")),
            Type::Enum(ref st) => write!(f, "{}", join(st.cases.iter(), " | ")),
        }
    }
}

impl fmt::Display for StructMember
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        if self.name.is_empty() {
            write!(f, "{}", self.typ)
        } else {
            write!(f, "{}: {}", self.name, self.typ)
        }
    }
}

impl TreePrinter for Type
{
    fn print(&self, level: usize)
    {
        println!("{}{}", prefix(level), self);
    }
}


impl TreePrinter for TypeAlias
{
    fn print(&self, level: usize)
    {
        println!("{}{} = {} ({})", prefix(level), self.name, self.original, self.span);
    }
}

pub fn to_primitive(name: &str) -> Option<Type>
{
    match name
    {
        "int" => Some(Type::Int),
        "float" => Some(Type::Float),
        "string" => Some(Type::String),
        "bool" => Some(Type::Bool),
        "char" => Some(Type::Char),
        _ => None,
    }
}

impl Hash for Type
{
    fn hash<H>(&self, state: &mut H) where H: Hasher
    {
        let s = format!("{}", self);
        s.hash(state);
    }
}
