use std::ptr;
use std::ffi::CStr;
use llvm::core::*;
use llvm::prelude::*;
use llvm::*;
use compileerror::Pos;
use codegen::cstr;
use codegen::valueref::ValueRef;
use codegen::context::Context;
use codegen::expressions::const_int;
use codegen::builtin::get_slice_type_name;

pub fn is_same_kind(a: LLVMTypeKind, b: LLVMTypeKind) -> bool
{
    (a as usize) == (b as usize)
}

pub unsafe fn is_struct(t: LLVMTypeRef) -> bool
{
    is_same_kind(LLVMGetTypeKind(t), LLVMTypeKind::LLVMStructTypeKind)
}

pub unsafe fn is_array(t: LLVMTypeRef) -> bool
{
    is_same_kind(LLVMGetTypeKind(t), LLVMTypeKind::LLVMArrayTypeKind)
}

pub unsafe fn is_pointer(t: LLVMTypeRef) -> bool
{
    is_same_kind(LLVMGetTypeKind(t), LLVMTypeKind::LLVMPointerTypeKind)
}

unsafe fn array_to_slice(ctx: &Context, from: &ValueRef, to: LLVMTypeRef) -> Option<ValueRef>
{
    let from_type = from.get_element_type();
    if !is_array(from_type) {
        return None;
    }

    let array_element_type = LLVMGetElementType(from_type);
    // Slices are structs containing a length field and a pointer field
    if !is_pointer(to) || !is_struct(LLVMGetElementType(to)) {
        return None
    }

    let sname = LLVMGetStructName(LLVMGetElementType(to));
    if sname == ptr::null() {
        return None;
    }

    let cname = CStr::from_ptr(sname);
    match cname.to_str()
    {
        Ok(cname_str) => {
            if get_slice_type_name(array_element_type) == cname_str
            {
                let ptr = ValueRef::local(ctx.builder, LLVMGetElementType(to));
                let len = LLVMGetArrayLength(from_type);
                LLVMBuildStore(ctx.builder, const_int(ctx.context, len as u64), LLVMBuildStructGEP(ctx.builder, ptr.get(), 0, cstr("length")));

                let index = const_int(ctx.context, 0);
                let first_element_ptr = from.get_array_element(ctx, index, Pos::zero()).expect("Not a valid array");
                LLVMBuildStore(ctx.builder, first_element_ptr.get(), LLVMBuildStructGEP(ctx.builder, ptr.get(), 1, cstr("data")));
                Some(ptr)
            }
            else
            {
                None
            }
        },
        Err(_) => None,
    }
}

unsafe fn array_to_ptr(b: LLVMBuilderRef, from: &ValueRef, to: LLVMTypeRef) -> Option<ValueRef>
{
    let from_type = from.get_element_type();
    let can_convert =
        is_struct(from_type) &&
        is_pointer(to) &&
        LLVMGetElementType(from_type) == LLVMGetElementType(to);
    if can_convert {
        let cast = LLVMBuildBitCast(b, from.load(), to, cstr("cast"));
        Some(ValueRef::new(cast, from.is_const(), b))
    } else {
        None
    }
}

// Convert a value to a different type, if needed and possible
pub unsafe fn convert(ctx: &Context, from: ValueRef, to: LLVMTypeRef) ->  Option<ValueRef>
{
    if from.get_element_type() == to || from.get_value_type() == to {
        return Some(from); // Same types, so no problem
    }

    let c = array_to_ptr(ctx.builder, &from, to);
    if c.is_some() {
        return c;
    }

    let c = array_to_slice(ctx, &from, to);
    if c.is_some() {
        return c;
    }

    None
}