use ast::{StructDeclaration, SumTypeDeclaration, TypeDeclaration, Function, Module, Type, func_type,
    struct_type, sum_type, sum_type_case, enum_type};
use passes::TypeCheckerContext;
use compileerror::{CompileResult, unknown_name};

#[derive(Eq, PartialEq, Debug)]
enum TypeResolved
{
    Yes,
    No,
}

#[derive(Eq, PartialEq, Copy, Clone, Debug)]
enum ResolveMode
{
    Lazy,
    Forced,
}

fn resolve_type(ctx: &TypeCheckerContext, typ: &mut Type) -> TypeResolved
{
    let rt = if let Type::Unresolved(ref ut) = *typ {
        ctx.resolve_type(&ut.name)
    } else {
        return TypeResolved::Yes;
    };

    match rt
    {
        Some(t) => {
            *typ = t;
            TypeResolved::Yes
        },
        _ => {
            TypeResolved::No
        },
    }
}

pub fn resolve_function_args_and_ret_type(ctx: &mut TypeCheckerContext, fun: &mut Function) -> CompileResult<()>
{
    // We cannot resolve generics, until we instantiate them, so threat them as resolved
    if fun.is_generic() || fun.sig.typ != Type::Unknown {
        return Ok(());
    }

    if resolve_type(ctx, &mut fun.sig.return_type) == TypeResolved::No {
        return Err(unknown_name(fun.sig.span.start, &format!("{}", fun.sig.return_type)));
    }

    let mut args = Vec::with_capacity(fun.sig.args.len());
    for ref mut arg in &mut fun.sig.args {
        if resolve_type(ctx, &mut arg.typ) == TypeResolved::No {
            return Err(unknown_name(arg.span.start, &format!("{}", arg.typ)));
        }

        args.push(arg.typ.clone());
    }

    fun.sig.typ = func_type(args, fun.sig.return_type.clone());
    try!(ctx.add(&fun.sig.name, fun.sig.typ.clone(), fun.sig.span.start));
    Ok(())
}

fn resolve_struct_member_types(ctx: &mut TypeCheckerContext, sd: &mut StructDeclaration, mode: ResolveMode) -> CompileResult<TypeResolved>
{
    if sd.typ != Type::Unknown {
        return Ok(TypeResolved::Yes);
    }

    let mut member_types = Vec::with_capacity(sd.members.len());
    for m in &mut sd.members
    {
        if resolve_type(ctx, &mut m.typ) == TypeResolved::No {
            if mode == ResolveMode::Lazy {
                return Ok(TypeResolved::No);
            } else {
                return Err(unknown_name(m.span.start, &format!("{}", m.typ)));
            }
        }

        member_types.push(m.clone());
    }

    sd.typ = struct_type(member_types);
    Ok(TypeResolved::Yes)
}

fn resolve_sum_case_types(ctx: &mut TypeCheckerContext, st: &mut SumTypeDeclaration, mode: ResolveMode) -> CompileResult<TypeResolved>
{
    if st.typ != Type::Unknown {
        return Ok(TypeResolved::Yes);
    }

    let mut case_types = Vec::with_capacity(st.cases.len());
    for c in st.cases.iter_mut()
    {
        if let Some(ref mut sd) = c.data
        {
            if try!(resolve_struct_member_types(ctx, sd, mode)) == TypeResolved::No
            {
                return Ok(TypeResolved::No);
            }
            else
            {
                case_types.push(sum_type_case(&c.name, sd.typ.clone()));
            }
        }
        else
        {
            case_types.push(sum_type_case(&c.name, Type::Int)); // Use integer type for cases without structs
        }
    }

    if case_types.iter().all(|ct| ct.typ == Type::Int)
    {
        let case_names: Vec<String> = st.cases.iter().map(|c| c.name.clone()).collect();
        st.typ = enum_type(case_names.clone(), None);
        try!(ctx.add(&st.name, st.typ.clone(), st.span.start));
        for (idx, c) in st.cases.iter().enumerate()
        {
            let et = enum_type(case_names.clone(), Some(idx));
            try!(ctx.add(&c.name, et, c.span.start));
        }
    }
    else
    {
        st.typ = sum_type(case_types.clone(), None);
        try!(ctx.add(&st.name, st.typ.clone(), st.span.start));
        for (idx, c) in st.cases.iter_mut().enumerate()
        {
            c.typ = sum_type(case_types.clone(), Some(idx));
            try!(ctx.add(&c.name, c.typ.clone(), c.span.start));
        }
    }

    Ok(TypeResolved::Yes)
}

fn resolve_all_types(ctx: &mut TypeCheckerContext, module: &mut Module, mode: ResolveMode) -> CompileResult<usize>
{
    let mut num_resolved = 0;
    for typ in module.types.values_mut()
    {
        match *typ
        {
            TypeDeclaration::Struct(ref mut s) => {
                if try!(resolve_struct_member_types(ctx, s, mode)) == TypeResolved::Yes
                {
                    try!(ctx.add(&s.name, s.typ.clone(), s.span.start));
                    num_resolved += 1;
                }
            },
            TypeDeclaration::Sum(ref mut s) => {
                if try!(resolve_sum_case_types(ctx, s, mode)) == TypeResolved::Yes {
                    num_resolved += 1;
                }
            },
            TypeDeclaration::Alias(ref mut _a) => {
                panic!("NYI");
            }
        }
    }

    Ok(num_resolved)
}

pub fn resolve_types(ctx: &mut TypeCheckerContext, module: &mut Module) -> CompileResult<()>
{
    let mut num_resolved = 0;
    loop
    {
        let already_resolved = num_resolved;
        num_resolved = try!(resolve_all_types(ctx, module, ResolveMode::Lazy));

        if num_resolved == module.types.len() {
            break;
        } else if already_resolved == num_resolved {
            // We weren't able to resolve any in this pass, so something is missing
            try!(resolve_all_types(ctx, module, ResolveMode::Forced));
            break;
        }
    }


    for ref mut f in module.functions.values_mut() {
        try!(resolve_function_args_and_ret_type(ctx, f));
    }

    Ok(())
}
