use std::fmt;
use std::collections::{BTreeMap, HashMap};
use itertools::free::join;
use ast::{Type, FunctionSignature};
use llrep::llinstruction::LLInstruction;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct LLVar
{
    pub name: String,
    pub typ: Type,
}

impl LLVar
{
    pub fn new(idx: usize, typ: Type) -> LLVar
    {
        LLVar{
            name: format!("$var{}", idx),
            typ: typ,
        }
    }

    pub fn named(name: &str, typ: Type) -> LLVar
    {
        LLVar{
            name: name.into(),
            typ: typ,
        }
    }
}

impl fmt::Display for LLVar
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        write!(f, "({}: {})", self.name, self.typ)
    }
}

#[derive(Debug)]
pub struct Scope
{
    named_vars: HashMap<String, LLVar>,
    to_dec_ref: Vec<LLVar>,
}

impl Scope
{
    pub fn new() -> Scope
    {
        Scope{
            named_vars: HashMap::new(),
            to_dec_ref: Vec::new(),
        }
    }

    pub fn add_named_var(&mut self, var: LLVar)
    {
        self.named_vars.insert(var.name.clone(), var);
    }

    pub fn get_named_var(&self, var: &str) -> Option<LLVar>
    {
        self.named_vars.get(var).map(|v| v.clone())
    }

    pub fn add_dec_ref_target(&mut self, v: &LLVar) -> bool
    {
        if self.named_vars.get(&v.name).is_none() {
            false
        } else {
            self.to_dec_ref.push(v.clone());
            true
        }
    }

    pub fn remove_dec_ref_target(&mut self, v: &LLVar) -> bool
    {
        let len = self.to_dec_ref.len();
        self.to_dec_ref.retain(|e| e != v);
        self.to_dec_ref.len() < len
    }

    pub fn cleanup(&self, func: &mut LLFunction)
    {
        // Cleanup in reverse construction order
        for v in self.to_dec_ref.iter().rev() {
            func.add(LLInstruction::DecRef(v.clone()));
        }
    }
}


pub type LLBasicBlockRef = usize;

pub fn bb_name(bb: LLBasicBlockRef) -> String
{
    if bb == 0 {
        "entry".into()
    } else {
        format!("block{}", bb)
    }
}

#[derive(Debug)]
pub struct LLBasicBlock
{
    pub name: String,
    pub instructions: Vec<LLInstruction>
}

impl LLBasicBlock
{
    pub fn new(name: String) -> LLBasicBlock
    {
        LLBasicBlock{
            name: name,
            instructions: Vec::new(),
        }
    }
}



#[derive(Debug)]
pub struct LLFunction
{
    pub sig: FunctionSignature,
    pub blocks: BTreeMap<LLBasicBlockRef, LLBasicBlock>,
    pub block_order: Vec<LLBasicBlockRef>,
    pub lambdas: Vec<LLFunction>,
    current_bb: usize,
    bb_counter: usize,
    var_counter: usize,
    scopes: Vec<Scope>,
    destinations: Vec<Option<LLVar>>,
}


impl LLFunction
{
    pub fn new(sig: &FunctionSignature) -> LLFunction
    {
        let mut f = LLFunction{
            sig: sig.clone(),
            blocks: BTreeMap::new(),
            block_order: Vec::new(),
            lambdas: Vec::new(),
            current_bb: 0,
            bb_counter: 0,
            var_counter: 0,
            scopes: vec![Scope::new()],
            destinations: Vec::new(),
        };

        let entry = f.create_basic_block();
        f.add_basic_block(entry);

        for arg in &sig.args {
            f.add_named_var(LLVar::named(&arg.name, arg.typ.clone()));
        }
        f
    }

    pub fn is_empty(&self) -> bool
    {
        self.blocks.get(&0).map(|bb| bb.instructions.is_empty()).unwrap_or(false)
    }

    pub fn add(&mut self, inst: LLInstruction)
    {
        // Pop final scope before returning
        match inst
        {
            LLInstruction::Return(_) => self.pop_scope(),
            _ => (),
        }

        let idx = self.current_bb;
        self.blocks.get_mut(&idx).map(|bb| bb.instructions.push(inst));
    }

    pub fn create_basic_block(&mut self) -> LLBasicBlockRef
    {
        let bb_ref = self.bb_counter;
        self.bb_counter += 1;
        let name = bb_name(bb_ref);
        self.blocks.insert(bb_ref, LLBasicBlock::new(name));
        bb_ref
    }

    pub fn add_basic_block(&mut self, bb_ref: LLBasicBlockRef)
    {
        self.block_order.push(bb_ref);
    }

    pub fn set_current_bb(&mut self, bb_ref: LLBasicBlockRef)
    {
        assert!(bb_ref < self.blocks.len());
        self.current_bb = bb_ref;
    }

    pub fn new_var(&mut self, typ: Type) -> LLVar
    {
        let idx = self.var_counter;
        self.var_counter += 1;
        let v = LLVar::new(idx, typ);
        self.add_named_var(v.clone());
        v
    }

    pub fn push_scope(&mut self)
    {
        self.scopes.push(Scope::new());
        self.add(LLInstruction::StartScope);
    }

    pub fn pop_scope(&mut self)
    {
        let s = self.scopes.pop().expect("Empty Scope Stack");
        s.cleanup(self);
        if !self.scopes.is_empty() {
            // Add an endscope instruction, but not at function exit
            self.add(LLInstruction::EndScope);
        }
    }

    pub fn push_destination(&mut self, var: Option<LLVar>)
    {
        self.destinations.push(var);
    }

    pub fn pop_destination(&mut self)
    {
        let _ = self.destinations.pop();
    }

    pub fn get_destination(&self) -> Option<LLVar>
    {
        match self.destinations.last() {
            Some(&Some(ref var)) => Some(var.clone()),
            _ => None
        }
    }

    pub fn add_named_var(&mut self, var: LLVar)
    {
        let scope = self.scopes.last_mut().expect("Empty Scope Stack");
        scope.add_named_var(var);
    }

    pub fn get_named_var(&self, var: &str) -> Option<LLVar>
    {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get_named_var(var) {
                return Some(v)
            }
        }

        None
    }

    pub fn add_dec_ref_target(&mut self, v: &LLVar)
    {
        for scope in self.scopes.iter_mut().rev() {
            if scope.add_dec_ref_target(v) {
                break;
            }
        }
    }

    pub fn remove_dec_ref_target(&mut self, v: &LLVar) -> bool
    {
        let scope = self.scopes.last_mut().expect("Empty Scope Stack");
        scope.remove_dec_ref_target(v)
    }
}

impl fmt::Display for LLFunction
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        for lambda in &self.lambdas {
            try!(lambda.fmt(f));
            try!(writeln!(f, ""));
        }

        try!(writeln!(f, "{}({}) -> {}:",
            self.sig.name,
            join(self.sig.args.iter().map(|arg| format!("{}: {}", arg.name, arg.typ)), ", "),
            self.sig.return_type)
        );
        for bb_ref in &self.block_order {
            let bb = self.blocks.get(bb_ref).expect("Unknown basic block");
            try!(writeln!(f, " {}:", bb.name));
            for inst in &bb.instructions {
                try!(inst.fmt(f));
            }
        }

        Ok(())
    }
}