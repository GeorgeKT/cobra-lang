use std::cmp::{Eq, PartialEq};
use std::fmt;
use std::hash::{Hasher, Hash};
use std::rc::Rc;
use itertools::free::join;
use ast::{Expression, TreePrinter, StructMember, prefix};
use compileerror::Span;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct SumTypeCase
{
    pub name: String,
    pub typ: Type,
}

#[derive(Debug, Clone)]
pub struct SumType
{
    pub cases: Vec<SumTypeCase>,
}

impl SumType
{
    pub fn index_of(&self, case_name: &str) -> Option<usize>
    {
        self.cases.iter().position(|cn| cn.name == case_name)
    }
}

#[derive(Debug, Clone)]
pub struct EnumType
{
    pub cases: Vec<String>,
}

impl EnumType
{
    pub fn index_of(&self, case_name: &str) -> Option<usize>
    {
        self.cases.iter().position(|cn| cn == case_name)
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct StructType
{
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
    pub length: usize,
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
    String,
    Bool,
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
    pub fn concat_allowed(&self, other: &Type) -> bool
    {
        if self.is_empty_array() || other.is_empty_array() {
            return true;
        }

        if self.is_sequence() && other.is_sequence() {
            return self.get_element_type() == other.get_element_type();
        }

        if self.is_sequence() && !other.is_sequence() {
            return self.get_element_type().map(|et| et == *other).unwrap_or(false);
        }

        if other.is_sequence() && !self.is_sequence() {
            return other.get_element_type().map(|et| et == *self).unwrap_or(false);
        }

        false
    }

    pub fn is_empty_array(&self) -> bool
    {
        match *self
        {
            Type::Array(ref at) => at.length == 0,
            _ => false,
        }
    }

    pub fn is_sequence(&self) -> bool
    {
        match *self
        {
            Type::Array(_) => true,
            Type::Slice(_) => true,
            _ => false,
        }
    }

    pub fn get_element_type(&self) -> Option<Type>
    {
        match *self
        {
            Type::Array(ref at) => Some(at.element_type.clone()),
            Type::Slice(ref st) => Some(st.element_type.clone()),
            _ => None,
        }
    }

    pub fn is_matchable(&self, other: &Type) -> bool
    {
        if (self.is_empty_array() && other.is_sequence()) || (other.is_empty_array() && self.is_sequence()) {
            return true;
        }

        if self.is_sequence() && other.is_sequence() {
            return self.get_element_type() == other.get_element_type();
        }

        *self == *other
    }

    // If possible generate a conversion expression
    pub fn convert(&self, from_type: &Type, expr: &Expression) -> Option<Expression>
    {
        match (self, from_type)
        {
            (&Type::Slice(ref s), &Type::Array(ref t)) if s.element_type == t.element_type =>
                // arrays can be converted to slices if the element type is the same
                Some(Expression::ArrayToSliceConversion(Box::new(expr.clone())))
            ,
            _ => None,
        }
    }

    pub fn is_convertible(&self, dst_type: &Type) -> bool
    {
        match (self, dst_type)
        {
            (&Type::Array(ref s), &Type::Slice(ref t)) if s.element_type == t.element_type => true,
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
            _ => false,
        }
    }

    pub fn pass_by_ptr(&self) -> bool
    {
        match *self
        {
            Type::Array(_) => true,
            Type::Slice(_) => true,
            Type::Func(_) => true,
            Type::Struct(_) => true,
            Type::Sum(_) => true,
            _ => false,
        }
    }

    pub fn return_by_ptr(&self) -> bool
    {
        match *self
        {
            Type::Array(_) => true,
            Type::Slice(_) => true,
            Type::Struct(_) => true,
            Type::Sum(_) => true,
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
        length: len,
    }))
}

pub fn slice_type(element_type: Type) -> Type
{
    Type::Slice(Rc::new(SliceType{
        element_type: element_type,
    }))
}

pub fn sum_type_case(name: &str, typ: Type) -> SumTypeCase
{
    SumTypeCase{
        name: name.into(),
        typ: typ,
    }
}

pub fn sum_type(cases: Vec<SumTypeCase>) -> Type
{
    Type::Sum(Rc::new(SumType{
        cases: cases,
    }))
}

pub fn enum_type(cases: Vec<String>) -> Type
{
    Type::Enum(Rc::new(EnumType{
        cases: cases,
    }))
}

pub fn struct_type(members: Vec<StructMember>) -> Type
{
    Type::Struct(Rc::new(StructType{
        members: members,
    }))
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
            Type::String => write!(f, "string"),
            Type::Bool => write!(f, "bool"),
            Type::Unresolved(ref s) =>
                if s.generic_args.is_empty() {
                    write!(f, "{}", s.name)
                } else {
                    write!(f, "{}<{}>", s.name, join(s.generic_args.iter(), ","))
                },
            Type::Array(ref at) =>
                if at.length == 0 {
                    write!(f, "[]")
                } else {
                    write!(f, "[{}; {}]", at.element_type, at.length)
                },
            Type::Slice(ref at) => write!(f, "[{}]", at.element_type),
            Type::Generic(ref g) => write!(f, "${}", g),
            Type::Func(ref ft) => write!(f, "({}) -> {}", join(ft.args.iter(), ", "), ft.return_type),
            Type::Struct(ref st) => write!(f, "{{{}}}", join(st.members.iter().map(|m| &m.typ), ", ")),
            Type::Sum(ref st) => write!(f, "{}", join(st.cases.iter().map(|m| &m.typ), " | ")),
            Type::Enum(ref st) => write!(f, "{}", join(st.cases.iter(), " | ")),
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

impl PartialEq<SumType> for SumType
{
    fn eq(&self, other: &SumType) -> bool
    {
        self.cases.eq(&other.cases)
    }
}

impl Eq for SumType {}

impl PartialEq<EnumType> for EnumType
{
    fn eq(&self, other: &EnumType) -> bool
    {
        self.cases.eq(&other.cases)
    }
}

impl Eq for EnumType {}
