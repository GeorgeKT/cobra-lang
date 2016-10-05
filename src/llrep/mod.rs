mod llfunction;
mod llinstruction;

use std::fmt;
use ast::*;
use parser::Operator;
pub use self::llfunction::{LLFunction, LLVar, LLBasicBlockRef};
pub use self::llinstruction::*;


pub struct LLModule
{
    pub name: String,
    pub functions: Vec<LLFunction>,
}

impl fmt::Display for LLModule
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        for func in &self.functions {
            try!(func.fmt(f));
            try!(writeln!(f, " "));
        }
        Ok(())
    }
}

fn add_set(func: &mut LLFunction, expr: LLExpr, dst: &LLVar)
{
    func.add(set_instr(dst, expr));
}

fn make_var(func: &mut LLFunction, expr: LLExpr, typ: Type) -> LLVar
{
    let var = func.new_var(typ);
    add_set(func, expr, &var);
    var
}

fn add_lit(func: &mut LLFunction, lit: LLLiteral, dst: &LLVar)
{
    add_set(func, LLExpr::Literal(lit), dst);
}

fn make_lit(func: &mut LLFunction, lit: LLLiteral, typ: Type) -> LLVar
{
    let var = func.new_var(typ);
    add_set(func, LLExpr::Literal(lit), &var);
    var
}

fn bind(func: &mut LLFunction, name: &str, var: &LLVar)
{
    func.add(bind_instr(name, var))
}

fn call_to_llrep(func: &mut LLFunction, c: &Call) -> LLVar
{
    let dst = get_dst(func, &c.return_type);
    func.push_destination(None);
    let args = c.args.iter().map(|arg| to_llrep(func, arg)).collect();
    func.pop_destination();

    func.add(set_instr(
        &dst,
        LLExpr::Call(
            c.callee.name.clone(),
            args,
        )
    ));
    if dst.typ.allocate_on_heap() {
        func.add_dec_ref_target(&dst);
    }
    dst
}

fn add_binding(func: &mut LLFunction, b: &LetBinding)
{
    let dst = stack_alloc(func, &b.typ, Some(&b.name));
    func.push_destination(Some(dst.clone()));
    expr_to_llrep(func, &b.init);
    func.pop_destination();
}

fn let_to_llrep(func: &mut LLFunction, l: &LetExpression) -> Option<LLVar>
{
    let dst = get_dst(func, &l.typ);
    func.push_scope();
    for b in &l.bindings{
        add_binding(func, b);
    }

    func.push_destination(Some(dst.clone()));
    to_llrep(func, &l.expression);
    func.pop_destination();
    func.pop_scope();
    Some(dst)
}

fn array_lit_to_llrep(func: &mut LLFunction, a: &ArrayLiteral, dst: &LLVar)
{
    let vars = a.elements.iter()
        .map(|e| to_llrep(func, e))
        .collect();

    add_lit(func, LLLiteral::Array(vars), dst);
}

fn struct_initializer_to_llrep(func: &mut LLFunction, si: &StructInitializer, dst: &LLVar)
{
    let init_members = |func: &mut LLFunction, si: &StructInitializer, dst: &LLVar| {
        for (idx, expr) in si.member_initializers.iter().enumerate() {
            let v = to_llrep(func, expr);
            func.add(set_struct_member_instr(&dst, idx, &v));
        }
    };

    if let Type::Sum(ref st) = dst.typ {
        let idx = st.index_of(&si.struct_name).expect("Internal Compiler Error: cannot determine index of sum type case");
        add_set(func, LLExpr::SumTypeCase(idx), dst);
        let struct_ptr = make_var(func, LLExpr::SumTypeStruct(dst.clone(), idx), st.cases[idx].typ.clone());
        init_members(func, si, &struct_ptr);
    } else {
        init_members(func, si, dst);
    }
}

fn add_array_len(func: &mut LLFunction, array: LLVar, dst: &LLVar)
{
    let expr = LLExpr::ArrayProperty(array, ArrayProperty::Len);
    add_set(func, expr, &dst);
}

fn make_array_len(func: &mut LLFunction, array: LLVar) -> LLVar
{
    let var = func.new_var(Type::Int);
    add_array_len(func, array, &var);
    var
}

fn member_access_to_llrep(func: &mut LLFunction, sma: &MemberAccess, dst: &LLVar)
{
    let mut obj = func.get_named_var(&sma.name).expect("Internal Compiler Error: Unknown variable");
    for (access_idx, at) in sma.access_types.iter().enumerate()
    {
        obj = match (&obj.typ, at)
        {
            (&Type::Struct(ref st), &MemberAccessType::StructMember(idx)) => {
                let expr = LLExpr::StructMember(obj.clone(), idx);
                if access_idx == sma.access_types.len() - 1 {
                    add_set(func, expr, dst);
                    return;
                } else {
                    let mtyp = st.members[idx].typ.clone();
                    let mvar = func.new_var(mtyp);
                    add_set(func, expr, &mvar);
                    mvar
                }
            },

            (&Type::Array(_), &MemberAccessType::ArrayProperty(ArrayProperty::Len)) => {
                add_array_len(func, obj.clone(), dst);
                return;
            },

            _ => panic!("Internal Compiler Error: Invalid member access"),
        };
    }
}


fn name_pattern_match_to_llrep(
    func: &mut LLFunction,
    mc: &MatchCase,
    target: &LLVar,
    match_end_bb: LLBasicBlockRef,
    match_case_bb: LLBasicBlockRef,
    next_bb: LLBasicBlockRef,
    nr: &NameRef)
{
    match nr.typ
    {
        Type::Enum(ref et) => {
            let idx = et.index_of(&nr.name).expect("Internal Compiler Error: cannot determine index of sum type case");
            let cv = make_lit(func, LLLiteral::Int(idx as u64), Type::Int);
            let cond = make_var(func, LLExpr::BinaryOp(Operator::Equals, target.clone(), cv), Type::Bool);
            func.add(branch_if_instr(&cond, match_case_bb, next_bb));
        },
        Type::Sum(ref st) => {
            let idx = st.index_of(&nr.name).expect("Internal Compiler Error: cannot determine index of sum type case");
            let cv = make_lit(func, LLLiteral::Int(idx as u64), Type::Int);
            let sum_type_index = make_var(func, LLExpr::SumTypeIndex(target.clone()), Type::Int);
            let cond = make_var(func,  LLExpr::BinaryOp(Operator::Equals, sum_type_index, cv), Type::Bool);
            func.add(branch_if_instr(&cond, match_case_bb, next_bb));
        },
        _ => {
            panic!("Internal Compiler Error: Expression is not a valid match pattern");
        }
    }

    match_case_body_to_llrep(func, mc, match_case_bb, match_end_bb, next_bb);
}


fn match_case_body_to_llrep(
    func: &mut LLFunction,
    mc: &MatchCase,
    match_case_bb: LLBasicBlockRef,
    match_end_bb: LLBasicBlockRef,
    next_bb: LLBasicBlockRef)
{
    func.set_current_bb(match_case_bb);
    expr_to_llrep(func, &mc.to_execute);
    func.add(LLInstruction::Branch(match_end_bb));
    func.set_current_bb(next_bb);
}

fn array_pattern_match_to_llrep(
    func: &mut LLFunction,
    ap: &ArrayPattern,
    seq: &LLVar,
    match_case_bb: LLBasicBlockRef,
    next_bb: LLBasicBlockRef)
{
    let head = make_var(func, LLExpr::ArrayHead(seq.clone()), seq.typ.get_element_type().expect("Invalid array type"));
    bind(func, &ap.head, &head);
    let tail = make_var(func, LLExpr::ArrayTail(seq.clone()), seq.typ.clone());
    bind(func, &ap.tail, &tail);

    let length = make_array_len(func, seq.clone());
    let zero = make_lit(func, LLLiteral::Int(0), Type::Int);
    let cond = make_var(func, LLExpr::BinaryOp(Operator::GreaterThan, length, zero), Type::Bool);
    func.add(branch_if_instr(&cond, match_case_bb, next_bb));
}

fn struct_pattern_match_to_llrep(
    func: &mut LLFunction,
    mc: &MatchCase,
    target: &LLVar,
    match_end_bb: LLBasicBlockRef,
    match_case_bb: LLBasicBlockRef,
    next_bb: LLBasicBlockRef,
    p: &StructPattern)
{
    func.push_destination(None);

    let add_bindings = |var: &LLVar, func: &mut LLFunction| {
        for (idx, b) in p.bindings.iter().enumerate() {
            if b != "_" {
                let expr = LLExpr::StructMember(var.clone(), idx);
                let member_ptr = make_var(func, expr, p.types[idx].clone());
                bind(func, b, &member_ptr);
            }
        }
    };

    match p.typ
    {
        Type::Struct(_) => {
            func.add(LLInstruction::Branch(match_case_bb));
            func.set_current_bb(match_case_bb);
            add_bindings(target, func);
        },
        Type::Sum(ref st) => {
            let target_sum_type_index = make_var(func, LLExpr::SumTypeIndex(target.clone()), Type::Int);
            let idx = st.index_of(&p.name).expect("Internal Compiler Error: cannot determine index of sum type case");
            let sum_type_index = make_lit(func, LLLiteral::Int(idx as u64), Type::Int);
            let cond = make_var(func, LLExpr::BinaryOp(Operator::Equals, target_sum_type_index, sum_type_index), Type::Bool);
            func.add(branch_if_instr(&cond, match_case_bb, next_bb));

            func.set_current_bb(match_case_bb);
            let struct_ptr = make_var(func, LLExpr::SumTypeStruct(target.clone(), idx), st.cases[idx].typ.clone());
            add_bindings(&struct_ptr, func);
        },
        _ => panic!("Internal Compiler Error: Expression is not a valid match pattern"),
    }

    func.pop_destination();
    match_case_body_to_llrep(func, mc, match_case_bb, match_end_bb, next_bb);
}

fn match_case_to_llrep(func: &mut LLFunction, mc: &MatchCase, target: &LLVar, match_end_bb: LLBasicBlockRef)
{
    let match_case_bb = func.create_basic_block();
    func.add_basic_block(match_case_bb);
    let next_bb = func.create_basic_block();
    func.add_basic_block(next_bb);

    let add_literal_case = |func: &mut LLFunction, lit: LLLiteral, typ: Type| {
        func.push_destination(None);
        let iv = make_lit(func, lit, typ);
        let cond = make_var(func, LLExpr::BinaryOp(Operator::Equals, iv, target.clone()), Type::Bool);
        func.add(branch_if_instr(&cond, match_case_bb, next_bb));
        func.pop_destination();
        match_case_body_to_llrep(func, mc, match_case_bb, match_end_bb, next_bb);
    };

    match mc.pattern
    {
        Pattern::Literal(Literal::Int(_, v)) => {
            add_literal_case(func, LLLiteral::Int(v), Type::Int);
        },

        Pattern::Literal(Literal::Float(_, ref v)) => {
            add_literal_case(func, LLLiteral::Float(v.clone()), Type::Float);
        },

        Pattern::Literal(Literal::Bool(_, v)) => {
            add_literal_case(func, LLLiteral::Bool(v), Type::Bool);
        },

        Pattern::Literal(Literal::Char(_, v)) => {
            add_literal_case(func, LLLiteral::Char(v), Type::Char);
        },

        Pattern::Name(ref nr) => {
            name_pattern_match_to_llrep(func, mc, target, match_end_bb, match_case_bb, next_bb, nr)
        },

        Pattern::Any(_) => {
            func.add(LLInstruction::Branch(match_case_bb));
            match_case_body_to_llrep(func, mc, match_case_bb, match_end_bb, next_bb);
        },

        Pattern::EmptyArray(_) => {
            match target.typ
            {
                Type::Array(_) => {
                    func.push_destination(None);
                    let length = make_array_len(func, target.clone());
                    let zero = make_lit(func, LLLiteral::Int(0), Type::Int);
                    let cond = make_var(func, LLExpr::BinaryOp(Operator::Equals, length, zero), Type::Bool);
                    func.add(branch_if_instr(&cond, match_case_bb, next_bb));
                    func.pop_destination();
                    match_case_body_to_llrep(func, mc, match_case_bb, match_end_bb, next_bb);
                },
                _ => panic!("Internal Compiler Error: Match expression cannot be matched with an array pattern"),
            }
        },

        Pattern::Array(ref ap) => {
            match target.typ
            {
                Type::Array(_) => {
                    func.push_destination(None);
                    array_pattern_match_to_llrep(func, ap, target, match_case_bb, next_bb);
                    func.pop_destination();
                },
                _ => panic!("Internal Compiler Error: Match expression cannot be matched with an array pattern"),
            }

            match_case_body_to_llrep(func, mc, match_case_bb, match_end_bb, next_bb);
        },

        Pattern::Literal(Literal::Array(ref a)) => {
            func.push_destination(None);
            let arr = func.new_var(a.array_type.clone());
            array_lit_to_llrep(func, a, &arr);
            let cond = make_var(func, LLExpr::BinaryOp(Operator::Equals, arr, target.clone()), Type::Bool);
            func.add(branch_if_instr(&cond, match_case_bb, next_bb));
            func.pop_destination();
            match_case_body_to_llrep(func, mc, match_case_bb, match_end_bb, next_bb);
        },

        Pattern::Literal(Literal::String(_, ref s)) => {
            func.push_destination(None);
            let arr = make_lit(func, LLLiteral::String(s.clone()), string_type());
            let cond = make_var(func, LLExpr::BinaryOp(Operator::Equals, arr, target.clone()), Type::Bool);
            func.add(branch_if_instr(&cond, match_case_bb, next_bb));
            func.pop_destination();
            match_case_body_to_llrep(func, mc, match_case_bb, match_end_bb, next_bb);
        },

        Pattern::Struct(ref p) => {
            struct_pattern_match_to_llrep(func, mc, target, match_end_bb, match_case_bb, next_bb, p);
        }
    }
}

fn match_to_llrep(func: &mut LLFunction, m: &MatchExpression) -> LLVar
{
    func.push_destination(None);
    let target_var = to_llrep(func, &m.target);
    func.pop_destination();
    let match_end_bb = func.create_basic_block();

    let dst = get_dst(func, &m.typ);
    func.push_scope();
    func.push_destination(Some(dst.clone()));
    for mc in &m.cases {
        match_case_to_llrep(func, mc, &target_var, match_end_bb);
    }
    func.pop_destination();

    func.add(LLInstruction::Branch(match_end_bb));
    func.add_basic_block(match_end_bb);
    func.set_current_bb(match_end_bb);
    func.pop_scope();
    dst
}


fn name_ref_to_llrep(func: &mut LLFunction, nr: &NameRef) -> Option<LLVar>
{
    let add_name_ref = |func: &mut LLFunction, nr: &NameRef| {
        let v = LLVar::named(&nr.name, nr.typ.clone());
        match func.get_destination()
        {
            Some(var) => {
                if var.typ.allocate_on_heap() {
                    func.add(LLInstruction::IncRef(v.clone()));
                }
                add_set(func, LLExpr::Ref(v), &var);
                Some(var)
            },
            None => Some(v),
        }
    };

    match nr.typ
    {
        Type::Sum(ref st) => {
            if let Some(idx) = st.index_of(&nr.name) {
                let dst = get_dst(func, &nr.typ);
                func.add(set_instr(&dst, LLExpr::HeapAlloc(dst.typ.clone())));
                func.add_dec_ref_target(&dst);
                add_set(func, LLExpr::SumTypeCase(idx), &dst);
                Some(dst)
            } else {
                add_name_ref(func, nr)
            }
        },
        Type::Enum(ref et) => {
            if let Some(idx) = et.index_of(&nr.name) {
                // enums are integers
                let dst = get_dst(func, &nr.typ);
                add_lit(func, LLLiteral::Int(idx as u64), &dst);
                Some(dst)
            } else {
                add_name_ref(func, nr)
            }
        },
        _ => {
            add_name_ref(func, nr)
        }
    }
}

fn to_llrep(func: &mut LLFunction, expr: &Expression) -> LLVar
{
    expr_to_llrep(func, expr).expect("Expression must return a value")
}

fn expr_to_llrep(func: &mut LLFunction, expr: &Expression) -> Option<LLVar>
{
    match *expr
    {
        Expression::NameRef(ref nr) => {
            name_ref_to_llrep(func, nr)
        },

        Expression::UnaryOp(ref u) => {
            func.push_destination(None);
            let v = to_llrep(func, &u.expression);
            func.pop_destination();
            let dst = get_dst(func, &u.typ);
            func.add(set_instr(&dst, LLExpr::UnaryOp(u.operator, v)));
            Some(dst)
        },

        Expression::BinaryOp(ref op) => {
            func.push_destination(None);
            let l = to_llrep(func, &op.left);
            let r = to_llrep(func, &op.right);
            func.pop_destination();
            let dst = get_dst(func, &op.typ);
            func.add(set_instr(&dst, LLExpr::BinaryOp(op.operator, l, r)));
            if dst.typ.allocate_on_heap() {
                func.add_dec_ref_target(&dst);
            }
            Some(dst)
        },

        Expression::Literal(Literal::Int(_, v)) => {
            let dst = get_dst(func, &Type::Int);
            add_lit(func, LLLiteral::Int(v), &dst);
            Some(dst)
        },

        Expression::Literal(Literal::Float(_, ref v_str)) => {
            let dst = get_dst(func, &Type::Float);
            add_lit(func, LLLiteral::Float(v_str.clone()), &dst);
            Some(dst)
        },

        Expression::Literal(Literal::String(_, ref s))  => {
            let dst = get_dst(func, &string_type());
            func.add(set_instr(&dst, LLExpr::HeapAlloc(dst.typ.clone())));
            func.add_dec_ref_target(&dst);
            add_lit(func, LLLiteral::String(s.clone()), &dst);
            Some(dst)
        },

        Expression::Literal(Literal::Bool(_, v)) => {
            let dst = get_dst(func, &Type::Bool);
            add_lit(func, LLLiteral::Bool(v), &dst);
            Some(dst)
        },

        Expression::Literal(Literal::Char(_, v)) => {
            let dst = get_dst(func, &Type::Char);
            add_lit(func, LLLiteral::Char(v), &dst);
            Some(dst)
        },

        Expression::Literal(Literal::Array(ref a)) => {
            let dst = get_dst(func, &a.array_type);
            func.add(set_instr(&dst, LLExpr::HeapAlloc(dst.typ.clone())));
            func.add_dec_ref_target(&dst);
            func.push_destination(None);
            array_lit_to_llrep(func, a, &dst);
            func.pop_destination();
            Some(dst)
        },

        Expression::Call(ref c) => {
            Some(call_to_llrep(func, c))
        },

        Expression::Let(ref l) => {
            let_to_llrep(func, l)
        },

        Expression::LetBindings(ref l) => {
            for b in &l.bindings {
                add_binding(func, b);
            }
            None
        },

        Expression::StructInitializer(ref si) => {
            let dst = get_dst(func, &si.typ);
            func.add(set_instr(&dst, LLExpr::HeapAlloc(dst.typ.clone())));
            func.add_dec_ref_target(&dst);
            func.push_destination(None);
            struct_initializer_to_llrep(func, si, &dst);
            func.pop_destination();
            Some(dst)
        },

        Expression::MemberAccess(ref sma) => {
            let dst = get_dst(func, &sma.typ);
            member_access_to_llrep(func, sma, &dst);
            Some(dst)
        },

        Expression::Match(ref m) => {
            Some(match_to_llrep(func, m))
        },

        Expression::If(ref i) => {
            let match_expr = i.to_match();
            Some(match_to_llrep(func, &match_expr))
        },

        Expression::Block(ref b) => {
            let dst = get_dst(func, &b.typ);
            func.push_destination(Some(dst.clone()));
            for (idx, e) in b.expressions.iter().enumerate() {
                if idx == b.expressions.len() - 1 {
                    expr_to_llrep(func, e);
                } else {
                    func.push_destination(None);
                    expr_to_llrep(func, e);
                    func.pop_destination();
                }
            }
            func.pop_destination();
            Some(dst)
        },

        Expression::Lambda(ref l) => {
            let lambda = func_to_llrep(&l.sig, &l.expr);
            func.lambdas.push(lambda);
            let dst = get_dst(func, &l.sig.get_type());
            add_set(func, LLExpr::Func(l.sig.name.clone()), &dst);
            Some(dst)
        },

        /*
        Expression::ArrayGenerator(ref _a) => panic!("NYI"),
        */

        _ => None,
    }
}

fn stack_alloc(func: &mut LLFunction, typ: &Type, name: Option<&str>) -> LLVar
{
    match name
    {
        Some(n) => {
            let var = LLVar::named(n, typ.clone());
            func.add_named_var(var.clone());
            func.add(LLInstruction::Alloc(var.clone()));
            var
        },
        None => {
            let var = func.new_var(typ.clone());
            func.add(LLInstruction::Alloc(var.clone()));
            var
        }
    }
}

fn get_dst(func: &mut LLFunction, typ: &Type) -> LLVar
{
    assert!(*typ != Type::Unknown);
    if let Some(dst) = func.get_destination() {
        assert!(dst.typ == *typ);
        return dst;
    }

    stack_alloc(func, typ, None)
}

fn func_to_llrep(sig: &FunctionSignature, expression: &Expression) -> LLFunction
{
    let mut llfunc = LLFunction::new(&sig);
    let var = to_llrep(&mut llfunc, &expression);
    if var.typ.allocate_on_heap() {
        llfunc.remove_dec_ref_target(&var);
    }
    llfunc.add(ret_instr(&var));
    llfunc
}

pub fn compile_to_llrep(md: &Module) -> LLModule
{
    let mut ll_mod = LLModule{
        name: md.name.clone(),
        functions: Vec::new(),
    };

    for func in md.externals.values() {
        ll_mod.functions.push(LLFunction::new(&func.sig));
    }

    for func in md.functions.values() {
        if !func.is_generic() {
            ll_mod.functions.push(func_to_llrep(&func.sig, &func.expression));
        }
    }

    ll_mod
}