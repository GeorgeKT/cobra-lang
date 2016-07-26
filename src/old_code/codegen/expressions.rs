use std::rc::Rc;
use std::ops::Deref;

use llvm::*;
use llvm::core::*;
use llvm::prelude::*;
use libc;

use ast::{UnaryOp, BinaryOp, Type, Expression, Call, MemberAccess, Member, IndexOperation, ObjectConstruction, NameRef, Assignment, ArrayLiteral, ArrayInitializer};
use codegen::{cstr, type_name};
use codegen::context::{Context};
use codegen::symbols::{StructType, FunctionInstance, PassingMode};
use codegen::conversions::{convert};
use codegen::valueref::{ValueRef};
use compileerror::{Span, Pos, CompileError, CompileResult, ErrorCode, err, type_error, expected_const_expr};
use parser::{Operator};

unsafe fn is_integer(ctx: LLVMContextRef, tr: LLVMTypeRef) -> bool
{
    tr == LLVMInt64TypeInContext(ctx)
}

unsafe fn is_floating_point(ctx: LLVMContextRef, tr: LLVMTypeRef) -> bool
{
    tr == LLVMDoubleTypeInContext(ctx)
}

unsafe fn is_numeric(ctx: LLVMContextRef, tr: LLVMTypeRef) -> bool
{
    is_integer(ctx, tr) || is_floating_point(ctx, tr)
}

pub unsafe fn const_int(ctx: LLVMContextRef, v: u64) -> LLVMValueRef
{
    LLVMConstInt(LLVMInt64TypeInContext(ctx), v, 0)
}

unsafe fn gen_float(ctx: &Context, num: &str, span: &Span) -> CompileResult<ValueRef>
{
    match num.parse::<f64>() {
        Ok(f) => Ok(ValueRef::new(LLVMConstReal(LLVMDoubleTypeInContext(ctx.context), f), true, ctx.builder)),
        Err(_) => err(span.start, ErrorCode::InvalidFloatingPoint, format!("{} is not a valid floating point number", num))
    }
}

unsafe fn gen_integer(ctx: &Context, i: u64, _span: &Span) -> CompileResult<ValueRef>
{
    Ok(ValueRef::new(const_int(ctx.context, i), true, ctx.builder))
}

unsafe fn gen_string_literal(ctx: &Context, s: &str, _span: &Span) -> CompileResult<ValueRef>
{
    let glob = LLVMAddGlobal(ctx.get_module_ref(), LLVMArrayType(LLVMInt8TypeInContext(ctx.context), s.len() as u32), cstr("string"));

    LLVMSetLinkage(glob, LLVMLinkage::LLVMInternalLinkage);
    LLVMSetGlobalConstant(glob, 1);

    // Initialize with string:
    LLVMSetInitializer(glob, try!(gen_const_string_literal(ctx, s)).load());
    Ok(ValueRef::new(glob, true, ctx.builder))
}

unsafe fn gen_const_string_literal(ctx: &Context, s: &str) -> CompileResult<ValueRef>
{
    Ok(ValueRef::new(LLVMConstStringInContext(ctx.context, s.as_ptr() as *const i8, s.len() as u32, 1), true, ctx.builder))
}

unsafe fn gen_unary(ctx: &mut Context, op: &UnaryOp) -> CompileResult<ValueRef>
{
    let e_val = try!(gen_expression(ctx, &op.expression));
    let e_type = e_val.get_element_type();
    match op.operator {
        Operator::Sub => {
            if !is_numeric(ctx.context, e_type) {
                err(op.span.start, ErrorCode::TypeError, "Operator '-', expects and integer or floating point expression as argument".into())
            } else {
                Ok(ValueRef::new(LLVMBuildNeg(ctx.builder, e_val.load(), cstr("neg")), true, ctx.builder))
            }
        },
        Operator::Not => {
            if !is_integer(ctx.context, e_type) {
                err(op.span.start, ErrorCode::TypeError, "Operator '!', expects an integer or boolean expression".into())
            } else {
                Ok(ValueRef::new(LLVMBuildNot(ctx.builder, e_val.load(), cstr("not")), true, ctx.builder))
            }
        },
        Operator::Increment => {
            if !is_integer(ctx.context, e_type) {
                err(op.span.start, ErrorCode::TypeError, "Operator '++', expects an integer expression".into())
            } else {
                Ok(ValueRef::new(LLVMBuildAdd(ctx.builder, e_val.load(), const_int(ctx.context, 1), cstr("inc")), true, ctx.builder))
            }
        },
        Operator::Decrement => {
            if !is_integer(ctx.context, e_type) {
                err(op.span.start, ErrorCode::TypeError, "Operator '--', expects an integer expression".into())
            } else {
                Ok(ValueRef::new(LLVMBuildSub(ctx.builder, e_val.load(), const_int(ctx.context, 1), cstr("dec")), true, ctx.builder))
            }
        },
        _ => err(op.span.start, ErrorCode::InvalidUnaryOperator, format!("Operator {} is not a unary operator", op.operator)),
    }
}

unsafe fn gen_pf_unary(ctx: &mut Context, op: &UnaryOp) -> CompileResult<ValueRef>
{
    match op.operator {
        Operator::Increment | Operator::Decrement =>
        {
            if op.expression.is_assignable()
            {
                let ptr = try!(gen_target(ctx, &op.expression));
                let val = ptr.load();
                if !is_integer(ctx.context, LLVMTypeOf(val)) {
                    return err(op.span.start, ErrorCode::TypeError, format!("Operator {} expects an integer expression", op.operator));
                }

                let nval = if op.operator == Operator::Increment {
                    LLVMBuildAdd(ctx.builder, val, const_int(ctx.context, 1), cstr("inc"))
                } else {
                    LLVMBuildSub(ctx.builder, val, const_int(ctx.context, 1), cstr("dec"))
                };
                try!(ptr.store(ctx, ValueRef::new(nval, true, ctx.builder), op.span.start));
                Ok(ValueRef::new(val, true, ctx.builder))
            }
            else
            {
                gen_unary(ctx, op)
            }

        },
        _ => err(op.span.start, ErrorCode::InvalidUnaryOperator, format!("Operator {} is not a unary operator", op.operator)),
    }
}

unsafe fn check_numeric_operands(ctx: &Context, op: Operator, left_type: LLVMTypeRef, right_type: LLVMTypeRef, pos: Pos) -> CompileResult<()>
{
    if left_type != right_type {
        err(pos, ErrorCode::TypeError, format!("Operator '{}', expects both operands to be of the same type", op))
    } else if !is_numeric(ctx.context, left_type) || !is_numeric(ctx.context, right_type){
        err(pos, ErrorCode::TypeError, format!("Operator '{}', expects integer or floating point expression as operands", op))
    } else {
        Ok(())
    }
}

unsafe fn check_bool_operands(ctx: &Context, op: Operator, left_type: LLVMTypeRef, right_type: LLVMTypeRef, pos: Pos) -> CompileResult<()>
{
    if left_type != right_type {
        err(pos, ErrorCode::TypeError, format!("Operator '{}', expects both operands to be of the same type", op, ))
    } else if !is_integer(ctx.context, left_type) || !is_integer(ctx.context, right_type){
        err(pos, ErrorCode::TypeError, format!("Operator '{}', expects integer or boolean point expression as operands", op))
    } else {
        Ok(())
    }
}


unsafe fn gen_binary(ctx: &mut Context, op: &BinaryOp) -> CompileResult<ValueRef>
{
    let left_val = try!(gen_expression(ctx, &op.left)).load();
    let right_val = try!(gen_expression(ctx, &op.right)).load();
    let left_type = LLVMTypeOf(left_val);
    let right_type = LLVMTypeOf(right_val);

    let v = match op.operator {
        Operator::Add => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFAdd(ctx.builder, left_val, right_val, cstr("add")))
            } else {
                Ok(LLVMBuildAdd(ctx.builder, left_val, right_val, cstr("add")))
            }
        },
        Operator::Sub => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFSub(ctx.builder, left_val, right_val, cstr("sub")))
            } else {
                Ok(LLVMBuildSub(ctx.builder, left_val, right_val, cstr("sub")))
            }
        },
        Operator::Div => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFDiv(ctx.builder, left_val, right_val, cstr("div")))
            } else {
                Ok(LLVMBuildUDiv(ctx.builder, left_val, right_val, cstr("div")))
            }
        },
        Operator::Mod => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFRem(ctx.builder, left_val, right_val, cstr("mod")))
            } else {
                Ok(LLVMBuildURem(ctx.builder, left_val, right_val, cstr("mod")))
            }
        },
        Operator::Mul => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFMul(ctx.builder, left_val, right_val, cstr("mul")))
            } else {
                Ok(LLVMBuildMul(ctx.builder, left_val, right_val, cstr("mul")))
            }
        },
        Operator::And => {
            try!(check_bool_operands(ctx, op.operator, left_type, right_type, op.span.start));
            Ok(LLVMBuildAnd(ctx.builder, left_val, right_val, cstr("and")))
        },
        Operator::Or => {
            try!(check_bool_operands(ctx, op.operator, left_type, right_type, op.span.start));
            Ok(LLVMBuildOr(ctx.builder, left_val, right_val, cstr("or")))
        },
        Operator::LessThan => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFCmp(ctx.builder, LLVMRealPredicate::LLVMRealOLT, left_val, right_val, cstr("cmp")))
            } else {
                Ok(LLVMBuildICmp(ctx.builder, LLVMIntPredicate::LLVMIntSLT, left_val, right_val, cstr("cmp")))
            }
        },
        Operator::LessThanEquals => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFCmp(ctx.builder, LLVMRealPredicate::LLVMRealOLE, left_val, right_val, cstr("cmp")))
            } else {
                Ok(LLVMBuildICmp(ctx.builder, LLVMIntPredicate::LLVMIntSLE, left_val, right_val, cstr("cmp")))
            }
        },
        Operator::GreaterThan => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFCmp(ctx.builder, LLVMRealPredicate::LLVMRealOGT, left_val, right_val, cstr("cmp")))
            } else {
                Ok(LLVMBuildICmp(ctx.builder, LLVMIntPredicate::LLVMIntSGT, left_val, right_val, cstr("cmp")))
            }
        },
        Operator::GreaterThanEquals => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFCmp(ctx.builder, LLVMRealPredicate::LLVMRealOGE, left_val, right_val, cstr("cmp")))
            } else {
                Ok(LLVMBuildICmp(ctx.builder, LLVMIntPredicate::LLVMIntSGE, left_val, right_val, cstr("cmp")))
            }
        },
        Operator::Equals => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFCmp(ctx.builder, LLVMRealPredicate::LLVMRealOEQ, left_val, right_val, cstr("cmp")))
            } else {
                Ok(LLVMBuildICmp(ctx.builder, LLVMIntPredicate::LLVMIntEQ, left_val, right_val, cstr("cmp")))
            }
        },
        Operator::NotEquals => {
            try!(check_numeric_operands(ctx, op.operator, left_type, right_type, op.span.start));
            if is_floating_point(ctx.context, left_type) {
                Ok(LLVMBuildFCmp(ctx.builder, LLVMRealPredicate::LLVMRealONE, left_val, right_val, cstr("cmp")))
            } else {
                Ok(LLVMBuildICmp(ctx.builder, LLVMIntPredicate::LLVMIntNE, left_val, right_val, cstr("cmp")))
            }
        },
        _ => err(op.span.start, ErrorCode::InvalidBinaryOperator, format!("Operator {} is not a binary operator", op.operator)),
    };

    v.map(|val| ValueRef::new(val, true, ctx.builder))
}

unsafe fn gen_enclosed(ctx: &mut Context, e: &Expression, _span: &Span) -> CompileResult<ValueRef>
{
    gen_expression(ctx, e)
}

unsafe fn gen_member_call(ctx: &mut Context, c: &Call, st: &StructType, self_ptr: ValueRef, private_allowed: bool) -> CompileResult<ValueRef>
{
    let func_name = try!(c.get_function_name());
    let full_name = format!("{}::{}", st.name, func_name);
    let func = try!(ctx
        .get_function(&full_name)
        .ok_or(CompileError::new(c.span.start, ErrorCode::UnknownFunction, format!("Unknown function {}", full_name))));

    if !private_allowed && !func.public {
        return err(c.span.start, ErrorCode::PrivateMemberAccess, format!("Attempting to access private member {}", func_name));
    }

    let mut arg_vals = Vec::with_capacity(c.args.len() + 1);

    // Add self argument
    let (expected_self_type, _) = *func.args.first().expect("Self argument missing");
    let self_element_type = self_ptr.get_element_type();
    if self_element_type == expected_self_type {
        arg_vals.push(self_ptr.load());
    } else if self_ptr.get_value_type() == expected_self_type {
        arg_vals.push(self_ptr.get());
    } else {
        return Err(type_error(c.span.start, format!("Self type mismatch (got {}, expected {})",
            type_name(self_ptr.get_value_type()), type_name(expected_self_type))));
    }

    for arg in &c.args {
        arg_vals.push(try!(gen_expression(ctx, arg)).load());
    }

    gen_call_common(ctx, c, &func, arg_vals)
}

unsafe fn gen_call_common(ctx: &mut Context, c: &Call, func: &FunctionInstance, mut arg_vals: Vec<LLVMValueRef>)  -> CompileResult<ValueRef>
{
    let func_name = try!(c.get_function_name());
    if arg_vals.len() != func.args.len() {
        return err(c.span.start, ErrorCode::ArgumentCountMismatch,
            format!("Function '{}', expects {} arguments, {} are provided",
                func_name, func.args.len(), c.args.len()));
    }

    for (i, arg) in c.args.iter().enumerate()
    {
        let (ref arg_type, ref arg_mode) = func.args[i];
        let arg_val = match *arg_mode
        {
            PassingMode::Copy => try!(ValueRef::new(arg_vals[i], true, ctx.builder).copy(ctx, c.span.start)).load(),
            PassingMode::Value => arg_vals[i],
        };

        let nval = convert(ctx, ValueRef::new(arg_val, true, ctx.builder), *arg_type);
        match nval
        {
            Some(val) => {
                arg_vals[i] = val.load();
                println!("arg_vals[{}] : {}", i, type_name(LLVMTypeOf(arg_vals[i])));
            },
            None => {
                let val_type = LLVMTypeOf(arg_vals[i]);
                let msg = format!("Argument {} of function '{}' has the wrong type\n  Expecting {}, got {}",
                                i, func_name, type_name(*arg_type), type_name(val_type));
                return err(arg.span().start, ErrorCode::TypeError, msg);
            },
        }
    }

    Ok(ValueRef::new(
        LLVMBuildCall(ctx.builder, func.function, arg_vals.as_mut_ptr(), arg_vals.len() as libc::c_uint, cstr("")),
        true,
        ctx.builder))
}

unsafe fn gen_call(ctx: &mut Context, c: &Call) -> CompileResult<ValueRef>
{
    let mut arg_vals = Vec::with_capacity(c.args.len());
    for arg in &c.args {
        let a = try!(gen_expression(ctx, arg));
        arg_vals.push(a.load());
    }

    let func_name = try!(c.get_function_name());
    let func = try!(ctx
        .get_function(&func_name)
        .ok_or(CompileError::new(c.span.start, ErrorCode::UnknownFunction, format!("Unknown function {}", func_name))));

    gen_call_common(ctx, c, &func, arg_vals)
}

unsafe fn gen_name_ref(ctx: &Context, nr: &NameRef) -> CompileResult<ValueRef>
{
    if let Some(ref v) = ctx.get_variable(&nr.name) {
        Ok(ValueRef::new(
            v.value,
            v.constant,
            ctx.builder,
        ))
    } else {
        err(nr.span.start, ErrorCode::UnknownVariable, format!("Unknown variable {}", nr.name))
    }
}

unsafe fn assign(ctx: &Context, op: Operator, var: ValueRef, val: ValueRef, span: &Span) -> CompileResult<ValueRef>
{
    if op == Operator::Assign {
        return var.store(ctx, val, span.start);
    }

    let var_type = var.get_element_type();

    try!(check_numeric_operands(ctx, op, var_type, val.get_element_type(), span.start));
    let var_val = var.load();
    let new_val = match op
    {
        Operator::AddAssign => {
            if is_floating_point(ctx.context, var_type) {
                LLVMBuildFAdd(ctx.builder, var_val, val.load(), cstr("op"))
            } else {
                LLVMBuildAdd(ctx.builder, var_val, val.load(), cstr("op"))
            }
        },
        Operator::SubAssign => {
            if is_floating_point(ctx.context, var_type) {
                LLVMBuildFSub(ctx.builder, var_val, val.load(), cstr("op"))
            } else {
                LLVMBuildSub(ctx.builder, var_val, val.load(), cstr("op"))
            }
        },
        Operator::MulAssign => {
            if is_floating_point(ctx.context, var_type) {
                LLVMBuildFMul(ctx.builder, var_val, val.load(), cstr("op"))
            } else {
                LLVMBuildMul(ctx.builder, var_val, val.load(), cstr("op"))
            }
        },
        Operator::DivAssign =>  {
            if is_floating_point(ctx.context, var_type) {
                LLVMBuildFDiv(ctx.builder, var_val, val.load(), cstr("op"))
            } else {
                LLVMBuildUDiv(ctx.builder, var_val, val.load(), cstr("op"))
            }
        },
        _ => {
            return err(span.start, ErrorCode::InvalidOperator, format!("'{}' isn't an assigment operator", op));
        }
    };

    let new_val = ValueRef::new(new_val, true, ctx.builder);
    try!(var.store(ctx, new_val.clone(), span.start));
    Ok(new_val) // Return the new value
}

unsafe fn gen_member_var(_ctx: &Context, this: ValueRef, st: &StructType, nr: &NameRef, private_allowed: bool) -> CompileResult<ValueRef>
{
    if let Some((idx, mvar)) = st.get_member(&nr.name)
    {
        if !mvar.public && !private_allowed {
            return err(nr.span.start, ErrorCode::PrivateMemberAccess, format!("Attempting to access private member {}", nr.name.clone()));
        }

        let this_element_type = this.get_element_type();
        if this_element_type == st.typ
        {
            this.get_struct_element(idx as u32, nr.span.start)
        }
        else if this_element_type == LLVMPointerType(st.typ, 0)
        {
            // Dereference this, to get the actual struct ptr, and the get the element
            this.deref(nr.span.start).and_then(|v| v.get_struct_element(idx as u32, nr.span.start))
        }
        else
        {
            return err(nr.span.start, ErrorCode::TypeError, format!("Type mismatch when accessing member variable"));
        }
    }
    else
    {
        err(nr.span.start, ErrorCode::UnknownStructMember, format!("struct {} has no member named {}", st.name, nr.name))
    }
}

unsafe fn gen_nested_member_access_by_name(
    ctx: &mut Context,
    this: ValueRef,
    st: &StructType,
    next: &MemberAccess,
    private_allowed: bool,
    member_name: &str) -> CompileResult<ValueRef>
{
    let (idx, mvar) = try!(st.get_member(member_name).ok_or(
        CompileError::new(next.span.start, ErrorCode::UnknownStructMember, format!("struct {} has no member named {}", st.name, member_name))));
    if !mvar.public && !private_allowed {
        return err(next.span.start, ErrorCode::PrivateMemberAccess,
            format!("Attempting to access private member {}", mvar.name));
    }

    let next_st = try!(ctx.get_struct_type(&mvar.typ, next.span.start));
    let new_this = try!(this.get_struct_element(idx as u32, next.span.start));
    match next.member
    {
        Member::Call(ref c) => gen_member_call(ctx, c, &next_st, new_this, false),
        Member::Var(ref nr) => gen_member_var(ctx, new_this, &next_st, nr, false),
        Member::Nested(ref next_next) => gen_nested_member_access(ctx, new_this, &next_st, next_next, false),
    }
}

unsafe fn gen_nested_member_access(ctx: &mut Context, this: ValueRef, st: &StructType, next: &MemberAccess, private_allowed: bool) -> CompileResult<ValueRef>
{
    match *next.target.deref()
    {
        Expression::NameRef(ref nr) => {
            gen_nested_member_access_by_name(ctx, this, st, next, private_allowed, &nr.name)
        },
        _ => {
            Err(type_error(next.span.start, "Expected name reference".into()))
        },
    }
}

unsafe fn gen_member_access(ctx: &mut Context, a: &MemberAccess) -> CompileResult<ValueRef>
{
    let st = try!(ctx
        .infer_type(&a.target)
        .and_then(|t| ctx.get_struct_type(&t, a.span.start)));
    let target_ptr = try!(gen_target(ctx, &a.target));

    let private_allowed = if let &Expression::NameRef(ref nr) = a.target.deref() {
        nr.name == "self"
    } else {
        false
    };

    match a.member
    {
        Member::Call(ref c) => gen_member_call(ctx, c, &st, target_ptr, private_allowed),
        Member::Var(ref nr) => gen_member_var(ctx, target_ptr, &st, nr, private_allowed),
        Member::Nested(ref next) => gen_nested_member_access(ctx, target_ptr, &st, next, private_allowed),
    }
}

unsafe fn gen_index_operation(ctx: &mut Context, iop: &IndexOperation) -> CompileResult<ValueRef>
{
    let index = try!(gen_expression(ctx, &iop.index_expr)).load();
    if !is_integer(ctx.context, LLVMTypeOf(index)) {
        return Err(type_error(iop.index_expr.span().start, format!("Indexing must be done with an integer expression")));
    }

    let target_type = try!(ctx.infer_type(&iop.target));
    match target_type
    {
        Type::Slice(_) => {
            let slice = try!(gen_target(ctx, &iop.target));
            let slice_data = try!(slice.get_struct_element(1, iop.target.span().start));
            slice_data.get_array_element(ctx, index, iop.span.start)
        },
        Type::Array(_, _) => {
            let array = try!(gen_target(ctx, &iop.target));
            array.get_array_element(ctx, index, iop.span.start)
        },
        _ => Err(type_error(iop.span.start, format!("Indexing not supported on {}", target_type))),
    }
}

unsafe fn gen_target(ctx: &mut Context, target: &Expression) -> CompileResult<ValueRef>
{
    match *target
    {
        Expression::NameRef(ref nr) => {
            gen_name_ref(ctx, nr)
        },

        Expression::MemberAccess(ref ma) => {
            gen_member_access(ctx, ma)
        },

        Expression::IndexOperation(ref iop) => {
            gen_index_operation(ctx, iop)
        },
        _ => err(target.span().start, ErrorCode::TypeError, format!("Invalid left hand side expression")),
    }
}

unsafe fn gen_assignment(ctx: &mut Context, a: &Assignment) -> CompileResult<ValueRef>
{
    let target_ptr = try!(gen_target(ctx, &a.target));
    let target_type = target_ptr.get_element_type();
    let rhs_val = try!(gen_expression(ctx, &a.expression));
    let rhs_type = rhs_val.get_element_type();
    if let Some(cv) = convert(ctx, rhs_val, target_type)
    {
        assign(ctx, a.operator, target_ptr, cv, &a.span)
    }
    else
    {
        let msg = format!("Attempting to assign an expression of type '{}' to a variable of type '{}'",
            type_name(rhs_type), type_name(target_type));
        err(a.span.start, ErrorCode::TypeError, msg)
    }
}

unsafe fn gen_const_object_construction(ctx: &mut Context, oc: &ObjectConstruction) -> CompileResult<ValueRef>
{
    let st: Rc<StructType> = try!(ctx
        .get_complex_type(&oc.object_type.name)
        .ok_or(CompileError::new(oc.span.start, ErrorCode::UnknownType,
            format!("Unknown type {}", oc.object_type.name))));

    if oc.args.len() > st.members.len() {
        return err(oc.span.start, ErrorCode::ArgumentCountMismatch,
            format!("Too many arguments in construction of an object of type '{}', maximum {} allowed",
                st.name, st.members.len()));
    }

    let mut vals = Vec::new();
    for (idx, m) in st.members.iter().enumerate() {
        let v = if let Some(ref e) = oc.args.get(idx) {
            try!(gen_const_expression(ctx, e))
        } else {
            // Use default initializer
            try!(gen_const_expression(ctx, &m.init))
        };

        if v.is_constant_value() {
            vals.push(v.load());
        } else {
            return err(oc.span.start, ErrorCode::ExpectedConstExpr, format!("Global structs must be initialized with constant expressions"));
        }
    }

    Ok(ValueRef::new(
        LLVMConstStructInContext(ctx.context, vals.as_mut_ptr(), vals.len() as u32, 0),
        true,
        ctx.builder
    ))
}

unsafe fn gen_object_construction_store(ctx: &mut Context, oc: &ObjectConstruction, ptr: &ValueRef) -> CompileResult<()>
{
    if ctx.in_global_context()
    {
        LLVMSetInitializer(ptr.get(), try!(gen_const_object_construction(ctx, oc)).get());
    }
    else
    {
        let st: Rc<StructType> = try!(ctx
            .get_complex_type(&oc.object_type.name)
            .ok_or(CompileError::new(oc.span.start, ErrorCode::UnknownType, format!("Unknown type {}", oc.object_type.name))));

        if oc.args.len() > st.members.len() {
            return err(oc.span.start, ErrorCode::ArgumentCountMismatch,
                format!("Too many arguments in construction of an object of type '{}', maximum {} allowed",
                    st.name, st.members.len()));
        }

        for (idx, m) in st.members.iter().enumerate()
        {
            let element = try!(ptr.get_struct_element(idx as u32, oc.span.start));
            if let Some(ref e) = oc.args.get(idx) {
                try!(gen_expression_store(ctx, e, &element))
            } else {
                // Use default initializer
                try!(gen_expression_store(ctx, &m.init, &element))
            }
        }
    }

    Ok(())
}

unsafe fn gen_object_construction(ctx: &mut Context, oc: &ObjectConstruction) -> CompileResult<ValueRef>
{
    let st: Rc<StructType> = try!(ctx
        .get_complex_type(&oc.object_type.name)
        .ok_or(CompileError::new(oc.span.start, ErrorCode::UnknownType,
            format!("Unknown type {}", oc.object_type.name))));

    let ptr = ValueRef::local(ctx.builder, st.typ);
    try!(gen_object_construction_store(ctx, oc, &ptr));
    Ok(ptr)
}

unsafe fn gen_const_array_literal(ctx: &mut Context, a: &ArrayLiteral) -> CompileResult<ValueRef>
{
    let element_type = try!(ctx.infer_array_element_type(a));
    let llvm_type = try!(ctx.resolve_type(&element_type).ok_or(
        type_error(a.span.start, format!("Unknown type {}", element_type))
    ));

    let mut vals = Vec::new();
    for element in &a.elements
    {
        let element_val = try!(gen_const_expression(ctx, element));
        if element_val.is_constant_value() {
            vals.push(element_val.load());
        } else {
            return err(a.span.start, ErrorCode::ExpectedConstExpr,
                format!("Global arrays must be initialized with constant expressions"));
        }
    }

    Ok(ValueRef::new(LLVMConstArray(llvm_type, vals.as_mut_ptr(), vals.len() as u32), true, ctx.builder))
}

unsafe fn gen_array_literal_store(ctx: &mut Context, a: &ArrayLiteral, ptr: &ValueRef) -> CompileResult<()>
{
    if ctx.in_global_context()
    {
        LLVMSetInitializer(ptr.get(), try!(gen_const_array_literal(ctx, a)).load());
    }
    else
    {
        for (idx, element) in a.elements.iter().enumerate()
        {
            let index = const_int(ctx.context, idx as u64);
            try!(ptr
                .get_array_element(ctx, index, a.span.start)
                .and_then(|v| gen_expression_store(ctx, element, &v)));
        }
    }

    Ok(())
}

unsafe fn gen_array_literal(ctx: &mut Context, a: &ArrayLiteral) -> CompileResult<ValueRef>
{
    let element_type = try!(ctx.infer_array_element_type(a));
    let llvm_type = try!(ctx.resolve_type(&element_type)
        .ok_or(type_error(a.span.start, format!("Unknown type '{}'", element_type))));
    let var = ValueRef::local(ctx.builder, LLVMArrayType(llvm_type, a.elements.len() as u32));
    try!(gen_array_literal_store(ctx, a, &var));
    Ok(var)
}

unsafe fn gen_const_array_initializer(ctx: &mut Context, a: &ArrayInitializer) -> CompileResult<ValueRef>
{
    let element_type = try!(ctx.infer_type(&a.init));
    let llvm_type = try!(ctx.resolve_type(&element_type).ok_or(
        type_error(a.span.start, format!("Unknown type {}", element_type))
    ));

    let element_val = try!(gen_const_expression(ctx, &a.init));
    if !element_val.is_constant_value() {
        return err(a.span.start, ErrorCode::ExpectedConstExpr,
            format!("Global arrays must be initialized with constant expressions"));
    }

    let mut vals = Vec::with_capacity(a.times as usize);
    for _ in 0..a.times {
        vals.push(element_val.load());
    }

    Ok(ValueRef::new(LLVMConstArray(llvm_type, vals.as_mut_ptr(), vals.len() as u32), true, ctx.builder))
}

unsafe fn gen_array_initializer_store(ctx: &mut Context, a: &ArrayInitializer, ptr: &ValueRef) -> CompileResult<()>
{
    if ctx.in_global_context()
    {
        LLVMSetInitializer(ptr.get(), try!(gen_const_array_initializer(ctx, a)).get());
    }
    else
    {
        for idx in 0..a.times
        {
            let index = const_int(ctx.context, idx as u64);
            try!(ptr
                .get_array_element(ctx, index, a.span.start)
                .and_then(|v| gen_expression_store(ctx, &a.init, &v)))
        }
    }

    Ok(())
}

unsafe fn gen_array_initializer(ctx: &mut Context, a: &ArrayInitializer) -> CompileResult<ValueRef>
{
    let array_element_type = try!(ctx.infer_type(&a.init));
    let llvm_type = try!(ctx.resolve_type(&array_element_type)
        .ok_or(type_error(a.span.start, format!("Unknown type '{}'", array_element_type))));
    let var = ValueRef::local(ctx.builder, LLVMArrayType(llvm_type, a.times as u32));
    try!(gen_array_initializer_store(ctx, a, &var));
    Ok(var)
}

pub unsafe fn gen_expression(ctx: &mut Context, e: &Expression) -> CompileResult<ValueRef>
{
    match *e
    {
        Expression::IntLiteral(ref span, integer) => gen_integer(ctx, integer, span),
        Expression::FloatLiteral(ref span, ref s) => gen_float(ctx, s, span),
        Expression::StringLiteral(ref span, ref s) => gen_string_literal(ctx, s, span),
        Expression::ArrayLiteral(ref a) => gen_array_literal(ctx, a),
        Expression::ArrayInitializer(ref a) => gen_array_initializer(ctx, a),
        Expression::UnaryOp(ref op) => gen_unary(ctx, op),
        Expression::PostFixUnaryOp(ref op) => gen_pf_unary(ctx, op),
        Expression::BinaryOp(ref op) => gen_binary(ctx, op),
        Expression::Enclosed(ref span, ref e) => gen_enclosed(ctx, e, span),
        Expression::Call(ref c) => gen_call(ctx, c),
        Expression::NameRef(ref nr) => gen_name_ref(ctx, nr),
        Expression::Assignment(ref a) => gen_assignment(ctx, a),
        Expression::MemberAccess(ref ma) => gen_member_access(ctx, ma),
        Expression::IndexOperation(ref iop) => gen_index_operation(ctx, iop),
        Expression::ObjectConstruction(ref oc) => gen_object_construction(ctx, oc),
    }
}

unsafe fn store(ctx: &mut Context, e: &Expression, ptr: &ValueRef) -> CompileResult<()>
{
    let v = try!(gen_expression(ctx, e));
    try!(ptr.store(ctx, v, e.span().start));
    Ok(())
}

pub unsafe fn gen_expression_store(ctx: &mut Context, e: &Expression, ptr: &ValueRef) -> CompileResult<()>
{
    match *e
    {
        Expression::ObjectConstruction(ref oc) => gen_object_construction_store(ctx, oc, ptr),
        Expression::ArrayLiteral(ref a) => gen_array_literal_store(ctx, a, ptr),
        Expression::ArrayInitializer(ref a) => gen_array_initializer_store(ctx, a, ptr),
        _ => store(ctx, e, &ptr),
    }
}

unsafe fn gen_const_expression(ctx: &mut Context, e: &Expression) -> CompileResult<ValueRef>
{
    match *e
    {
        Expression::IntLiteral(ref span, integer) => gen_integer(ctx, integer, span),
        Expression::FloatLiteral(ref span, ref s) => gen_float(ctx, s, span),
        Expression::StringLiteral(_, ref s) => gen_const_string_literal(ctx, s),
        Expression::ArrayLiteral(ref a) => gen_const_array_literal(ctx, a),
        Expression::ObjectConstruction(ref oc) => gen_const_object_construction(ctx, oc),
        _ => {
            let v = try!(gen_expression(ctx, e));
            if !v.is_constant_value() {
                Err(expected_const_expr(e.span().start, format!("Expected a constant expression")))
            } else {
                Ok(v)
            }
        },
    }
}