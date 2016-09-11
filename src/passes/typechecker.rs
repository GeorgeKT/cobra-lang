use ast::*;
use compileerror::{CompileResult, CompileError, Pos, ErrorCode, err, unknown_name};
use parser::{Operator};
use passes::{TypeCheckerContext, instantiate_generics, fill_in_generics, resolve_types};


fn invalid_unary_operator<T>(pos: Pos, op: Operator) -> CompileResult<T>
{
    err(pos, ErrorCode::InvalidUnaryOperator, format!("{} is not a valid unary operator", op))
}

fn expected_numeric_operands<T>(pos: Pos, op: Operator) -> CompileResult<T>
{
    err(pos, ErrorCode::TypeError, format!("Operator {} expects two numeric expression as operands", op))
}

fn type_check_unary_op(ctx: &mut TypeCheckerContext, u: &mut UnaryOp) -> CompileResult<Type>
{
    let e_type = try!(type_check_expression(ctx, &mut u.expression, None));
    if e_type.is_generic() {
        u.typ = e_type.clone();
        return Ok(e_type)
    }

    match u.operator
    {
        Operator::Sub => {
            if !e_type.is_numeric() {
                err(u.span.start, ErrorCode::TypeError, format!("Unary operator {} expects a numeric expression", u.operator))
            } else {
                u.typ = e_type.clone();
                Ok(e_type)
            }
        },

        Operator::Not => {
            if !e_type.is_bool() {
                err(u.span.start, ErrorCode::TypeError, format!("Unary operator {} expects a boolean expression", u.operator))
            } else {
                u.typ = Type::Bool;
                Ok(Type::Bool)
            }
        }
        _ => invalid_unary_operator(u.span.start, u.operator),
    }
}

fn type_check_binary_op(ctx: &mut TypeCheckerContext, b: &mut BinaryOp) -> CompileResult<Type>
{
    let left_type = try!(type_check_expression(ctx, &mut b.left, None));
    let right_type = try!(type_check_expression(ctx, &mut b.right, None));

    if left_type.is_generic() || right_type.is_generic() {
        return Ok(left_type);
    }

    match b.operator
    {
        Operator::Add => {
            match addition_type(&left_type, &right_type)
            {
                Some(t) => {
                    b.typ = t.clone();
                    Ok(t)
                },
                None => err(b.span.start, ErrorCode::TypeError,
                    format!("Addition is not supported on operands of type {} and {}", left_type, right_type))
            }
        },

        Operator::Sub |
        Operator::Mul |
        Operator::Div =>
            if !left_type.is_numeric() || !right_type.is_numeric() {
                expected_numeric_operands(b.span.start, b.operator)
            } else if left_type != right_type {
                err(b.span.start, ErrorCode::TypeError, format!("Operator {} expects operands of the same type", b.operator))
            } else {
                b.typ = right_type;
                Ok(left_type)
            },

        Operator::LessThan |
        Operator::GreaterThan |
        Operator::LessThanEquals |
        Operator::GreaterThanEquals =>
            if !left_type.is_numeric() || !right_type.is_numeric() {
                expected_numeric_operands(b.span.start, b.operator)
            } else if left_type != right_type {
                err(b.span.start, ErrorCode::TypeError, format!("Operator {} expects operands of the same type", b.operator))
            } else {
                b.typ = Type::Bool;
                Ok(Type::Bool)
            },

        Operator::Mod =>
            if !left_type.is_integer() || !right_type.is_integer() {
                err(b.span.start, ErrorCode::TypeError, format!("Operator {} expects two integer expressions as operands", b.operator))
            } else {
                b.typ = Type::Int;
                Ok(Type::Int)
            },
        Operator::Equals | Operator::NotEquals =>
            if left_type != right_type {
                err(b.span.start, ErrorCode::TypeError, format!("Operator {} expects two expressions of the same type as operands", b.operator))
            } else {
                Ok(Type::Bool)
            },

        Operator::And | Operator::Or =>
            if !left_type.is_bool() || !right_type.is_bool() {
                err(b.span.start, ErrorCode::TypeError, format!("Operator {} expects two boolean expressions as operands", b.operator))
            } else {
                b.typ = Type::Bool;
                Ok(Type::Bool)
            },
        _ => err(b.span.start, ErrorCode::InvalidBinaryOperator, format!("Operator {} is not a binary operator", b.operator))
    }
}

fn type_check_array_literal(ctx: &mut TypeCheckerContext, a: &mut ArrayLiteral) -> CompileResult<Type>
{
    if a.elements.is_empty() {
        a.array_type = Type::EmptyArray;
        return Ok(a.array_type.clone());
    }

    let mut array_element_type = Type::Unknown;
    for e in a.elements.iter_mut() {
        let t = try!(type_check_expression(ctx, e, None));
        if array_element_type == Type::Unknown {
            array_element_type = t;
        } else if array_element_type != t {
            return err(e.span().start, ErrorCode::TypeError, format!("Array elements must have the same type"));
        }
    }

    let array_type = array_type(array_element_type);
    if a.array_type == Type::Unknown {
        a.array_type = array_type;
    } else if a.array_type != array_type {
        return err(a.span.start, ErrorCode::TypeError, format!("Array has type {}, but elements have type {}", a.array_type, array_type))
    }

    Ok(a.array_type.clone())
}

fn type_check_array_generator(ctx: &mut TypeCheckerContext, a: &mut ArrayGenerator) -> CompileResult<Type>
{
    ctx.push_stack();

    // At the moment assume iterable is an array, in the future expand to all iterators
    let it_type = try!(type_check_expression(ctx, &mut a.iterable, None));
    let it_element_type = match it_type.get_element_type()
    {
        Some(Type::Unknown) => return err(a.span.start, ErrorCode::TypeError, format!("Extract expression with empty array is pointless")),
        Some(et) => et,
        None => return err(a.span.start, ErrorCode::TypeError, format!("Iterable expression in array generator is not an array")),
    };

    try!(ctx.add(&a.var, it_element_type, a.span.start));

    let element_type = try!(type_check_expression(ctx, &mut a.left, None));
    a.array_type = array_type(element_type);
    ctx.pop_stack();
    Ok(a.array_type.clone())
}


fn resolve_generic_args_in_call(ctx: &mut TypeCheckerContext, ft: &FuncType, c: &mut Call) -> CompileResult<Vec<Type>>
{
    let mut arg_types = Vec::with_capacity(c.args.len());
    let mut count = c.generic_args.len();
    loop
    {
        arg_types.clear();
        for (arg, expected_arg_type) in c.args.iter_mut().zip(ft.args.iter())
        {
            let expected_arg_type = c.generic_args.substitute(&expected_arg_type);
            let arg_type = try!(type_check_expression(ctx, arg, Some(expected_arg_type.clone())));
            let arg_type = c.generic_args.substitute(&arg_type);

            if expected_arg_type.is_generic() {
                try!(fill_in_generics(&arg_type, &expected_arg_type, &mut c.generic_args, arg.span().start));
            }
            arg_types.push(arg_type);
        }

        if c.generic_args.len() == count {
            break;
        }
        count = c.generic_args.len();
    }

    Ok(arg_types)
}


fn type_check_call(ctx: &mut TypeCheckerContext, c: &mut Call) -> CompileResult<Type>
{
    let func_type = try!(ctx.resolve_type(&c.callee.name).ok_or(unknown_name(c.span.start, &c.callee.name)));
    if let Type::Func(ref ft) = func_type
    {
        if ft.args.len() != c.args.len() {
            return err(c.span.start, ErrorCode::TypeError,
                format!("Attempting to call {} with {} arguments, but it needs {}", c.callee.name, c.args.len(), ft.args.len()));
        }

        let arg_types = try!(resolve_generic_args_in_call(ctx, ft, c));
        for idx in 0..c.args.len()
        {
            let expected_arg_type = c.generic_args.substitute(&ft.args[idx]);
            let arg_type = &arg_types[idx];
            if *arg_type == expected_arg_type
            {
                continue
            }
            else
            {
                if let Some(conversion_expr) = expected_arg_type.convert(&arg_type, &c.args[idx])
                {
                    c.args[idx] = conversion_expr;
                }
                else
                {
                    return err(c.args[idx].span().start, ErrorCode::TypeError,
                        format!("Argument {} has the wrong type, function {} expects the type {}, argument provided has type {}",
                            idx, c.callee.name, expected_arg_type, arg_type))
                }
            }
        }

        if ft.return_type.is_generic() {
            c.return_type = c.generic_args.substitute(&ft.return_type);
            return Ok(c.return_type.clone());
        }
        c.return_type = ft.return_type.clone();
        Ok(ft.return_type.clone())
    }
    else
    {
        err(c.span.start, ErrorCode::CallingNonCallable, format!("{} is not callable", c.callee.name))
    }
}


fn type_check_function(ctx: &mut TypeCheckerContext, fun: &mut Function) -> CompileResult<Type>
{
    ctx.push_stack();
    for arg in fun.sig.args.iter_mut()
    {
        if arg.typ.pass_by_ptr() {
            arg.passing_mode = ArgumentPassingMode::ByPtr;
        }
        try!(ctx.add(&arg.name, arg.typ.clone(), arg.span.start));
    }

    let et = try!(type_check_expression(ctx, &mut fun.expression, None));
    ctx.pop_stack();
    if et != fun.sig.return_type {
        return err(fun.span.start, ErrorCode::TypeError, format!("Function {} has return type {}, but it is returning an expression of type {}",
            fun.sig.name, fun.sig.return_type, et));
    }

    fun.type_checked = true;
    Ok(fun.sig.typ.clone())
}

fn type_check_match(ctx: &mut TypeCheckerContext, m: &mut MatchExpression) -> CompileResult<Type>
{
    let target_type = try!(type_check_expression(ctx, &mut m.target, None));
    let mut return_type = Type::Unknown;

    for c in &mut m.cases
    {
        let infer_case_type = |ctx: &mut TypeCheckerContext, e: &mut Expression, return_type: &Type| {
            let tt = try!(type_check_expression(ctx, e, None));
            if *return_type != Type::Unknown && *return_type != tt {
                return err(e.span().start, ErrorCode::TypeError, format!("Expressions in match statements must return the same type"));
            } else {
                Ok(tt)
            }
        };

        let match_pos = c.match_expr.span().start;
        let case_type = match c.match_expr
        {
            Expression::EmptyArrayPattern(ref ap) => {
                if !target_type.is_sequence() {
                    return err(ap.span.start, ErrorCode::TypeError, format!("Attempting to pattern match an expression of type {}, with an empty array", target_type));
                }
                try!(infer_case_type(ctx, &mut c.to_execute, &return_type))
            },

            Expression::ArrayPattern(ref ap) => {
                if !target_type.is_sequence() {
                    return err(ap.span.start, ErrorCode::TypeError, format!("Attempting to pattern match an expression of type {}, with an array", target_type));
                }

                let element_type = target_type.get_element_type().expect("target_type is not an array type");

                ctx.push_stack();
                try!(ctx.add(&ap.head, element_type.clone(), ap.span.start));
                try!(ctx.add(&ap.tail, array_type(element_type.clone()), ap.span.start));
                let ct = try!(infer_case_type(ctx, &mut c.to_execute, &return_type));
                ctx.pop_stack();
                ct
            },

            Expression::NameRef(ref mut nr) => {
                try!(type_check_name(ctx, nr, Some(target_type.clone())));
                if nr.name != "_" && nr.typ != target_type {
                    return err(match_pos, ErrorCode::TypeError,
                        format!("Cannot pattern match an expression of type {} with an expression of type {}",
                            target_type, nr.typ));
                }

                match nr.typ
                {
                    Type::Sum(ref st) => {
                        let idx = st.index_of(&nr.name).expect("Internal Compiler Error: cannot determine index of sum type case");
                        let ref case = st.cases[idx];
                        if case.typ == Type::Int {
                            try!(infer_case_type(ctx, &mut c.to_execute, &return_type))
                        } else {
                            return err(match_pos, ErrorCode::TypeError, format!("Invalid pattern match, match should be with an empty sum case"));
                        }
                    },
                    Type::Enum(_) => {
                        try!(infer_case_type(ctx, &mut c.to_execute, &return_type))
                    },
                    _ => {
                        if nr.name != "_"
                        {
                            return err(match_pos, ErrorCode::TypeError, format!("Invalid pattern match"));
                        }
                        else
                        {
                            try!(infer_case_type(ctx, &mut c.to_execute, &return_type))
                        }
                    }
                }
            },

            Expression::ArrayLiteral(_) |
            Expression::IntLiteral(_, _) |
            Expression::BoolLiteral(_, _) |
            Expression::FloatLiteral(_, _) |
            Expression::StringLiteral(_, _) => {
                let m_type = try!(type_check_expression(ctx, &mut c.match_expr, None));
                if !target_type.is_matchable(&m_type) {
                    return err(c.match_expr.span().start, ErrorCode::TypeError, format!("Pattern match of type {}, cannot match with an expression of type {}",
                        m_type, target_type));
                }

                try!(infer_case_type(ctx, &mut c.to_execute, &return_type))
            },

            Expression::StructPattern(ref mut p) => {
                try!(type_check_struct_pattern(ctx, p));
                if p.typ != target_type {
                    return err(match_pos, ErrorCode::TypeError,
                        format!("Cannot pattern match an expression of type {} with an expression of type {}",
                            target_type, p.typ));
                }

                ctx.push_stack();

                for (binding, typ) in p.bindings.iter().zip(p.types.iter()) {
                    if binding != "_" {
                        try!(ctx.add(binding, typ.clone(), p.span.start));
                    }
                }

                let ct = try!(infer_case_type(ctx, &mut c.to_execute, &return_type));
                ctx.pop_stack();
                ct
            },

            _ => {
                return err(c.match_expr.span().start, ErrorCode::TypeError, format!("Invalid pattern match"));
            }
        };

        if return_type == Type::Unknown {
            return_type = case_type;
        } else if return_type != case_type {
            return err(m.span.start, ErrorCode::TypeError, format!("Cases of match statements must return the same type"));
        }
    }

    m.typ = return_type.clone();
    Ok(return_type)
}

fn type_check_lambda_body(ctx: &mut TypeCheckerContext, m: &mut Lambda) -> CompileResult<Type>
{
    ctx.push_stack();
    for arg in &mut m.sig.args {
        if arg.typ.pass_by_ptr() {
            arg.passing_mode = ArgumentPassingMode::ByPtr;
        }
        try!(ctx.add(&arg.name, arg.typ.clone(), arg.span.start));
    }

    let return_type = try!(type_check_expression(ctx, &mut m.expr, None));
    ctx.pop_stack();
    m.set_return_type(return_type);
    Ok(m.sig.typ.clone())
}

fn type_check_lambda(ctx: &mut TypeCheckerContext, m: &mut Lambda, type_hint: Option<Type>) -> CompileResult<Type>
{
    match type_hint
    {
        Some(typ) => {
            use uuid::{Uuid};
            m.sig.name = format!("lambda-{}", Uuid::new_v4()); // Add a uuid, so we don't get name clashes
            try!(m.apply_type(&typ));
            let infered_type = try!(type_check_lambda_body(ctx, m));
            if infered_type != typ {
                return err(m.span.start, ErrorCode::TypeError, format!("Lambda body has the wrong type, expecting {}, got {}", typ, infered_type));
            }

            Ok(infered_type)
        },
        None => {
            if m.is_generic() {
                return Ok(Type::Unknown);
            }
            type_check_lambda_body(ctx, m)
        },
    }
}

fn is_instantiation_of(concrete_type: &Type, generic_type: &Type) -> bool
{
    if !generic_type.is_generic() {
        return *concrete_type == *generic_type;
    }

    match (concrete_type, generic_type)
    {
        (&Type::Array(ref a), &Type::Array(ref b)) => is_instantiation_of(&a.element_type, &b.element_type),
        (_, &Type::Generic(_)) => true,
        (&Type::Struct(ref a), &Type::Struct(ref b)) => {
            a.members.len() == b.members.len() &&
            a.members.iter()
                .zip(b.members.iter())
                .all(|(ma, mb)| is_instantiation_of(&ma.typ, &mb.typ))
        },
        (&Type::Func(ref a), &Type::Func(ref b)) => {
            is_instantiation_of(&a.return_type, &b.return_type) &&
            a.args.iter()
                .zip(b.args.iter())
                .all(|(ma, mb)| is_instantiation_of(ma, mb))
        }
        (&Type::Sum(ref a), &Type::Sum(ref b)) => {
            a.cases.iter()
                .zip(b.cases.iter())
                .all(|(ma, mb)| is_instantiation_of(&ma.typ, &mb.typ))
        }
        _ => false,
    }
}

fn type_check_name(ctx: &mut TypeCheckerContext, nr: &mut NameRef, type_hint: Option<Type>) -> CompileResult<Type>
{
    if nr.name == "_" {
        return Ok(Type::Unknown);
    }

    if !nr.typ.is_unknown() && !nr.typ.is_generic() {
        return Ok(nr.typ.clone()); // We have already determined the type
    }

    let resolved_type = try!(ctx.resolve_type(&nr.name).ok_or(unknown_name(nr.span.start, &nr.name)));

    if let Some(typ) = type_hint {
        if resolved_type == Type::Unknown {
            return err(nr.span.start, ErrorCode::UnknownType(nr.name.clone(), typ), format!("{} has unknown type", nr.name))
        }

        if resolved_type == typ {
            nr.typ = resolved_type;
            return Ok(nr.typ.clone());
        }

        if !resolved_type.is_generic() && !typ.is_generic() && !resolved_type.is_convertible(&typ) {
            return err(nr.span.start, ErrorCode::TypeError, format!("Type mismatch: expecting {}, but {} has type {}", typ, nr.name, resolved_type));
        }

        if resolved_type.is_generic() && !typ.is_generic() {
            if !is_instantiation_of(&typ, &resolved_type) {
                err(nr.span.start, ErrorCode::TypeError, format!("Type mismatch: {} is not a valid instantiation of {}", typ, resolved_type))
            } else {
                nr.typ = typ;
                Ok(nr.typ.clone())
            }
        } else {
            nr.typ = resolved_type;
            Ok(nr.typ.clone())
        }
    } else {
        nr.typ = resolved_type;
        Ok(nr.typ.clone())
    }
}

fn type_check_let(ctx: &mut TypeCheckerContext, l: &mut LetExpression) -> CompileResult<Type>
{
    ctx.push_stack();
    for b in &mut l.bindings
    {
        b.typ = try!(type_check_expression(ctx, &mut b.init, None));
        try!(ctx.add(&b.name, b.typ.clone(), b.span.start));
    }

    match type_check_expression(ctx, &mut l.expression, None)
    {
        Err(ref cr) => {
            if let ErrorCode::UnknownType(ref name, ref expected_type) = cr.error {
                let mut handled = false;
                for b in &mut l.bindings
                {
                    if b.name == *name
                    {
                        // It's one we know, so lets try again with a proper type hint
                        b.typ = try!(type_check_expression(ctx, &mut b.init, Some(expected_type.clone())));
                        ctx.update(&b.name, b.typ.clone());
                        l.typ = try!(type_check_expression(ctx, &mut l.expression, None));
                        handled = true;
                        break;
                    }
                }

                if !handled {
                    return Err(cr.clone());
                }
            } else {
                return Err(cr.clone());
            }
        },
        Ok(typ) => {
            l.typ = typ;
        }
    }

    ctx.pop_stack();
    Ok(l.typ.clone())
}

fn type_check_if(ctx: &mut TypeCheckerContext, i: &mut IfExpression) -> CompileResult<Type>
{
    let cond_type = try!(type_check_expression(ctx, &mut i.condition, Some(Type::Bool)));
    if cond_type != Type::Bool {
        return err(i.condition.span().start, ErrorCode::TypeError, format!("Condition of an if expression needs to be a boolean expression"));
    }

    let on_true_type = try!(type_check_expression(ctx, &mut i.on_true, None));
    let on_false_type = try!(type_check_expression(ctx, &mut i.on_false, None));
    if on_true_type != on_false_type {
        return err(i.condition.span().start, ErrorCode::TypeError,
            format!("then and else expression of an if expression need to be of the same type, then has type {}, else has type {}", on_true_type, on_false_type)
        );
    }

    i.typ = on_true_type;
    Ok(on_false_type)
}

fn type_check_struct_members_in_initializer(ctx: &mut TypeCheckerContext, members: &Vec<StructMember>, si: &mut StructInitializer) -> CompileResult<Type>
{
    if members.len() != si.member_initializers.len() {
        return err(si.span.start, ErrorCode::WrongArgumentCount,
            format!("Type {} has {} members, but attempting to initialize {} members", si.struct_name, members.len(), si.member_initializers.len()));
    }

    let mut new_members: Vec<StructMember> = Vec::with_capacity(members.len());

    for (idx, (member, mi)) in members.iter().zip(si.member_initializers.iter_mut()).enumerate()
    {
        let t = try!(type_check_expression(ctx, mi, Some(member.typ.clone())));
        let expected_type = if member.typ.is_generic() {
            try!(fill_in_generics(&t, &member.typ, &mut si.generic_args, mi.span().start))
        } else {
            member.typ.clone()
        };

        if t != expected_type
        {
            return err(mi.span().start, ErrorCode::TypeError,
                format!("Attempting to initialize member {} with type '{}', expecting an expression of type '{}'",
                    idx, t, expected_type));
        }

        new_members.push(struct_member(&member.name, t, member.span));
    }

    Ok(struct_type(new_members))
}

fn type_check_struct_initializer(ctx: &mut TypeCheckerContext, si: &mut StructInitializer) -> CompileResult<Type>
{
    let typ = try!(ctx.resolve_type(&si.struct_name).ok_or(unknown_name(si.span.start, &si.struct_name)));
    match typ
    {
        Type::Struct(st) => {
            si.typ = try!(type_check_struct_members_in_initializer(ctx, &st.members, si));
            Ok(si.typ.clone())
        },
        Type::Sum(st) => {
            let idx = st.index_of(&si.struct_name).expect("Internal Compiler Error: cannot determine index of sum type case");
            let mut sum_type_cases = Vec::with_capacity(st.cases.len());
            for (i, case) in st.cases.iter().enumerate()
            {
                let typ = if i == idx
                {
                    match case.typ
                    {
                        Type::Struct(ref s) => try!(type_check_struct_members_in_initializer(ctx, &s.members, si)),
                        Type::Int => Type::Int,
                        _ => return err(si.span.start, ErrorCode::TypeError, format!("Invalid sum type case")),
                    }
                } else {
                    case.typ.clone()
                };
                sum_type_cases.push(sum_type_case(&case.name, typ))
            }

            si.typ = sum_type(sum_type_cases);
            Ok(si.typ.clone())
        },
        _ => err(si.span.start, ErrorCode::TypeError, format!("{} is not a struct", si.struct_name)),
    }
}



fn find_member_type(members: &Vec<StructMember>, member_name: &str, pos: Pos) -> CompileResult<(usize, Type)>
{
    members.iter()
        .enumerate()
        .find(|&(_, m)| m.name == member_name)
        .map(|(idx, m)| (idx, m.typ.clone()))
        .ok_or(CompileError::new(pos, ErrorCode::UnknownStructMember, format!("Unknown struct member {}", member_name)))
}

fn type_check_struct_member_access(ctx: &mut TypeCheckerContext, sma: &mut StructMemberAccess) -> CompileResult<Type>
{
    let mut st = try!(ctx.resolve_type(&sma.name).ok_or(unknown_name(sma.span.start, &sma.name)));
    sma.indices.clear();
    for ref member_name in &sma.members
    {
        st = match st
        {
            Type::Struct(ref st) => {
                let (member_idx, member_type) = try!(find_member_type(&st.members, &member_name, sma.span.start));
                sma.indices.push(member_idx);
                member_type
            },
            _ => {
                if let Some(prop_type) = st.get_property_type(member_name) {
                    prop_type
                } else {
                    return err(sma.span.start, ErrorCode::TypeError, format!("Type '{}' has no property named '{}'", st, member_name));
                }
            },
        };
    }

    sma.typ = st.clone();
    Ok(st)
}

fn type_check_struct_pattern(ctx: &mut TypeCheckerContext, p: &mut StructPattern) -> CompileResult<Type>
{
    if !p.typ.is_unknown() {
        return Ok(p.typ.clone());
    }

    let typ = try!(ctx.resolve_type(&p.name).ok_or(unknown_name(p.span.start, &p.name)));
    match typ
    {
        Type::Sum(ref st) => {
            let idx = st.index_of(&p.name).expect("Internal Compiler Error: cannot determine index of sum type case");
            let ref case = st.cases[idx];
            match case.typ
            {
                Type::Struct(ref s) => {
                    if s.members.len() != p.bindings.len() {
                        err(p.span.start, ErrorCode::TypeError, format!("Not enough bindings in pattern match"))
                    } else {
                        p.types = s.members.iter().map(|sm| sm.typ.clone()).collect();
                        p.typ = Type::Sum(st.clone());
                        Ok(Type::Unknown)
                    }
                },
                _ => err(p.span.start, ErrorCode::TypeError, format!("Attempting to pattern match a normal sum type case with a struct")),
            }
        },

        Type::Struct(ref st) => {
            p.types = st.members.iter().map(|sm| sm.typ.clone()).collect();
            p.typ = Type::Struct(st.clone());
            Ok(Type::Unknown)
        },
        _ => err(p.span.start, ErrorCode::TypeError, format!("Struct pattern is only allowed for structs and sum types containing structs"))
    }
}

fn type_check_block(ctx: &mut TypeCheckerContext, b: &mut Block, type_hint: Option<Type>) -> CompileResult<Type>
{
    let num =  b.expressions.len();
    for (idx, e) in b.expressions.iter_mut().enumerate()
    {
        let typ = try!(type_check_expression(ctx, e, type_hint.clone()));
        if idx == num - 1 {
            b.typ = typ;
        }
    }

    Ok(b.typ.clone())
}

pub fn type_check_expression(ctx: &mut TypeCheckerContext, e: &mut Expression, type_hint: Option<Type>) -> CompileResult<Type>
{
    match *e
    {
        Expression::UnaryOp(ref mut op) => type_check_unary_op(ctx, op),
        Expression::BinaryOp(ref mut op) => type_check_binary_op(ctx, op),
        Expression::ArrayLiteral(ref mut a) => type_check_array_literal(ctx, a),
        Expression::ArrayPattern(_) => Ok(Type::Unknown), // Doesn't really have a type
        Expression::EmptyArrayPattern(_) => Ok(Type::Unknown), // Doesn't really have a type
        Expression::StructPattern(ref mut p) => type_check_struct_pattern(ctx, p),
        Expression::ArrayGenerator(ref mut a) => type_check_array_generator(ctx, a),
        Expression::Call(ref mut c) => type_check_call(ctx, c),
        Expression::NameRef(ref mut nr) => type_check_name(ctx, nr, type_hint),
        Expression::Match(ref mut m) => type_check_match(ctx, m),
        Expression::Lambda(ref mut l) => type_check_lambda(ctx, l, type_hint),
        Expression::Let(ref mut l) => type_check_let(ctx, l),
        Expression::If(ref mut i) => type_check_if(ctx, i),
        Expression::Block(ref mut b) => type_check_block(ctx, b, type_hint),
        Expression::IntLiteral(_, _) => Ok(Type::Int),
        Expression::FloatLiteral(_, _) => Ok(Type::Float),
        Expression::StringLiteral(_, _)  => Ok(string_type()),
        Expression::BoolLiteral(_, _) => Ok(Type::Bool),
        Expression::StructInitializer(ref mut si) => type_check_struct_initializer(ctx, si),
        Expression::StructMemberAccess(ref mut sma) => type_check_struct_member_access(ctx, sma),
    }
}

fn set_arg_passing_modes(fun: &mut ExternalFunction)
{
    for arg in fun.sig.args.iter_mut()
    {
        if arg.typ.pass_by_ptr() {
            arg.passing_mode = ArgumentPassingMode::ByPtr;
        }
    }
}

/*
    Type check and infer all the unkown types
*/
pub fn type_check_module(module: &mut Module) -> CompileResult<()>
{
    loop {
        let mut ctx = TypeCheckerContext::new();
        try!(resolve_types(&mut ctx, module));

        for ref mut f in module.functions.values_mut() {
            if !f.type_checked {
                try!(type_check_function(&mut ctx, f));
            }
        }

        let count = module.functions.len();
        try!(instantiate_generics(module));
        // As long as we are adding new generic functions, we need to type check the module again
        if count == module.functions.len() {
            break;
        }
    }

    for ref mut f in module.externals.values_mut() {
        set_arg_passing_modes(f);
    }

/*
    use ast::TreePrinter;
    module.print(0);
*/
    Ok(())
}
