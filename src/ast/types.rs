use std::fmt;
use std::ops::Deref;
use std::rc::Rc;
use itertools::free::join;
use ast::*;
use target::Target;
use span::Span;

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
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

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
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

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
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

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub struct StructMember
{
    pub name: String,
    pub typ: Type,
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub struct StructType
{
    pub name: String,
    pub members: Vec<StructMember>,
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub struct FuncType
{
    pub args: Vec<Type>,
    pub return_type: Type,
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub struct ArrayType
{
    pub element_type: Type,
    pub len: usize,
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub struct SliceType
{
    pub element_type: Type,
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub struct UnresolvedType
{
    pub name: String,
    pub generic_args: Vec<Type>,
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub struct InterfaceType
{
    pub name: String,
    pub generic_args: Vec<Type>,
    pub functions: Vec<FunctionSignature>
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub enum GenericType
{
    Any(String),
    Restricted(Vec<Type>),
}


#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash, Serialize, Deserialize)]
pub enum IntSize
{
    I8,
    I16,
    I32,
    I64,
}

impl IntSize
{
    pub fn size_in_bits(&self) -> u32
    {
        match *self {
            IntSize::I8 => 8,
            IntSize::I16 => 16,
            IntSize::I32 => 32,
            IntSize::I64 => 64,
        }
    }
}

impl fmt::Display for IntSize
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        write!(f, "{}", self.size_in_bits())
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash, Serialize, Deserialize)]
pub enum FloatSize
{
    F32,
    F64
}

impl fmt::Display for FloatSize
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        match *self {
            FloatSize::F32 => write!(f, "32"),
            FloatSize::F64 => write!(f, "64"),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Serialize, Deserialize)]
pub enum Type
{
    Void,
    Unknown,
    Int(IntSize),
    UInt(IntSize),
    Float(FloatSize),
    Char,
    Bool,
    String,
    SelfType,
    Pointer(Rc<Type>),
    Unresolved(Rc<UnresolvedType>),
    Array(Rc<ArrayType>),
    Slice(Rc<SliceType>),
    Generic(Rc<GenericType>),
    Func(Rc<FuncType>),
    Struct(Rc<StructType>),
    Sum(Rc<SumType>),
    Enum(Rc<EnumType>),
    Optional(Rc<Type>),
    Interface(Rc<InterfaceType>),
}

#[derive(Debug,  Eq, PartialEq, Clone, Serialize, Deserialize)]
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
            Type::Array(_) |
            Type::Slice(_) |
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
            Type::Pointer(ref inner) |
            Type::Optional(ref inner) => Some(inner.deref().clone()),
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
    pub fn convert(&self, from_type: &Type, expr: &Expression) -> Option<Expression>
    {
        match (self, from_type)
        {
            (&Type::Slice(ref st), &Type::Array(ref at)) if st.element_type == at.element_type => {
                Some(array_to_slice(expr.clone(), expr.span()))
            }

            (&Type::Optional(ref inner), _) if *inner.deref() == *from_type => {
                Some(to_optional(expr.clone(), self.clone()))
            }

            (&Type::Bool, &Type::Optional(_)) => {
                Some(Expression::OptionalToBool(Box::new(expr.clone())))
            }

            (&Type::Optional(ref inner), _) if from_type.is_optional_of(&Type::Unknown) => {
                Some(nil_expr_with_type(expr.span(), inner.deref().clone()))
            }

            (&Type::Void, _) => {
                Some(Expression::Block(Box::new(Block{
                    expressions: vec![
                        expr.clone(),
                        Expression::Void,
                    ],
                    typ: Type::Void,
                    span: expr.span(),
                })))
            }

            (&Type::Pointer(ref to), &Type::Pointer(ref from)) => {
                if *to.deref() == Type::Void {
                    Some(type_cast(expr.clone(), ptr_type(Type::Void), expr.span()))
                } else if *from.deref() == Type::Void {
                    Some(type_cast(expr.clone(), self.clone(), expr.span()))
                } else {
                    None
                }
            }

            (&Type::Bool, &Type::Pointer(_)) => {
                Some(type_cast(expr.clone(), Type::Bool, expr.span()))
            }

            _ => None,
        }
    }

    pub fn is_convertible(&self, dst_type: &Type) -> bool
    {
        match (self, dst_type)
        {
            (&Type::Array(ref at), &Type::Slice(ref st)) => at.element_type == st.element_type,
            (_, &Type::Optional(ref inner)) => *inner.deref() == *dst_type,
            _ => false,
        }
    }

    pub fn is_binary_operator_supported(&self, op: BinaryOperator) -> bool
    {
        const GENERAL_NUMERIC_OPERATORS: [BinaryOperator; 10] = [
            BinaryOperator::Add, BinaryOperator::Sub, BinaryOperator::Div, BinaryOperator::Mul,
            BinaryOperator::Equals, BinaryOperator::NotEquals, BinaryOperator::GreaterThan, BinaryOperator::LessThan,
            BinaryOperator::GreaterThanEquals, BinaryOperator::LessThanEquals,
        ];

        const COMPARISON_OPERATORS: [BinaryOperator; 6] = [
            BinaryOperator::Equals, BinaryOperator::NotEquals, BinaryOperator::GreaterThan, BinaryOperator::LessThan,
            BinaryOperator::GreaterThanEquals, BinaryOperator::LessThanEquals,
        ];

        match *self
        {
            Type::Int(_) | Type::UInt(_) => op == BinaryOperator::Mod || GENERAL_NUMERIC_OPERATORS.contains(&op),
            Type::Float(_) => GENERAL_NUMERIC_OPERATORS.contains(&op),
            Type::Char=> COMPARISON_OPERATORS.contains(&op),
            Type::Bool => COMPARISON_OPERATORS.contains(&op) || op == BinaryOperator::And || op == BinaryOperator::Or,
            Type::String | Type::Pointer(_) | Type::Optional(_) => op == BinaryOperator::Equals || op == BinaryOperator::NotEquals,
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
            Type::Interface(ref i) => !i.generic_args.is_empty(),
            _ => false,
        }
    }

    pub fn is_numeric(&self) -> bool
    {
        match *self
        {
            Type::Int(_) | Type::UInt(_) | Type::Float(_) => true,
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

    pub fn get_property_type(&self, name: &str, target: &Target) -> Option<(Type, MemberAccessType)>
    {
        match (self, name)
        {
            (&Type::Array(_), "len") |
            (&Type::Slice(_), "len") |
            (&Type::String, "len") =>
                Some((target.native_uint_type.clone(), MemberAccessType::Property(Property::Len))),

            (&Type::Slice(ref st), "data") =>
                Some((ptr_type(st.element_type.clone()), MemberAccessType::Property(Property::Data))),

            (&Type::String, "data") =>
                Some((ptr_type(Type::UInt(IntSize::I8)), MemberAccessType::Property(Property::Data))),

            _ => None,
        }
    }

    pub fn is_optional_of(&self, other_type: &Type) -> bool
    {
        if let Type::Optional(ref inner_type) = *self {
            *inner_type.deref() == *other_type
        } else {
            false
        }
    }

    pub fn is_optional(&self) -> bool
    {
        if let Type::Optional(_) = *self {
            true
        } else {
            false
        }
    }

    pub fn is_pointer(&self) -> bool
    {
        if let Type::Pointer(_) = *self {
            true
        } else {
            false
        }
    }

    pub fn is_pointer_to(&self, t: &Type) -> bool {
        if let Type::Pointer(ref inner) = *self {
            *inner.deref() == *t
        } else {
            false
        }
    }

    pub fn is_pointer_to_optional(&self) -> bool
    {
        if let Type::Pointer(ref inner) = *self {
            inner.is_optional()
        } else {
            false
        }
    }

    pub fn name(&self) -> String
    {
        match *self
        {
            Type::Struct(ref st) => st.name.clone(),
            Type::Sum(ref st) => st.name.clone(),
            Type::Enum(ref st) => st.name.clone(),
            Type::Interface(ref i) => i.name.clone(),
            _ => format!("{}", self),
        }
    }

    pub fn pass_by_value(&self) -> bool
    {
        match *self
        {
            Type::Int(_) |
            Type::UInt(_) |
            Type::Float(_) |
            Type::Char |
            Type::Bool |
            Type::Pointer(_) |
            Type::Enum(_) |
            Type::Func(_) => true,
            _ => false,
        }
    }

    pub fn get_pointer_element_type(&self) -> Option<&Type>
    {
        if let Type::Pointer(ref inner) = *self {
            Some(inner.deref())
        } else {
            None
        }
    }

    pub fn ptr_of(&self) -> Type
    {
        Type::Pointer(Rc::new(self.clone()))
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

pub fn string_type_representation(native_int_size: IntSize) -> StructType
{
    StructType{
        name: "string".into(),
        members: vec![
            struct_member("data", ptr_type(Type::UInt(IntSize::I8))),
            struct_member("len", Type::UInt(native_int_size))
        ]
    }
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

pub fn optional_type(inner: Type) -> Type
{
    Type::Optional(Rc::new(inner))
}

pub fn generic_type(name: &str) -> Type
{
    Type::Generic(Rc::new(GenericType::Any(name.into())))
}

pub fn generic_type_with_constraints(constraints: Vec<Type>) -> Type
{
    Type::Generic(
        Rc::new(
            GenericType::Restricted(constraints)
        )
    )
}

pub fn struct_member(name: &str, typ: Type) -> StructMember
{
    StructMember{name: name.into(), typ: typ}
}

/*
pub fn type_alias(name: &str, original: Type, span: Span) -> TypeAlias
{
    TypeAlias{
        name: name.into(),
        original: original,
        span: span,
    }
}
*/

pub fn unresolved_type(name: &str, generic_args: Vec<Type>) -> Type
{
    Type::Unresolved(Rc::new(UnresolvedType{
        name: name.into(),
        generic_args: generic_args,
    }))
}

pub fn interface_type(name: &str, generic_args: Vec<Type>, functions: Vec<FunctionSignature>) -> Type
{
    Type::Interface(Rc::new(InterfaceType{
        name: name.into(),
        generic_args: generic_args,
        functions: functions,
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
            Type::Int(precision) => write!(f, "int{}", precision),
            Type::UInt(precision) => write!(f, "uint{}", precision),
            Type::Float(precision) => write!(f, "float{}", precision),
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
            Type::Optional(ref inner) => write!(f, "?{}", inner),
            Type::Interface(ref i) => write!(f, "interface {}", i.name),
            Type::SelfType => write!(f, "Self"),
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

impl fmt::Display for GenericType
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        match *self
        {
            GenericType::Any(ref name) => write!(f, "{}", name),
            GenericType::Restricted(ref constraints) => write!(f, "({})", join(constraints.iter(), " + ")),
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
