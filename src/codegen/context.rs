use std::ptr;
use std::rc::Rc;
use std::os::raw::{c_char};
use std::ffi::{CStr, CString};
use std::fs::DirBuilder;
use std::collections::HashMap;

use llvm::prelude::*;
use llvm::core::*;
use llvm::target_machine::*;

use ast::{Type};
use codegen::{cstr, cstr_mut, type_name};
use compileerror::{Pos, CompileResult, CompileError, ErrorCode, err};
use codegen::symboltable::{VariableInstance, FunctionInstance, SymbolTable};
use codegen::slice::{new_slice_type};

pub struct StackFrame
{
    pub symbols: SymbolTable,
    pub current_function: LLVMValueRef,
}

impl StackFrame
{
    pub fn new(current_function: LLVMValueRef) -> StackFrame
    {
        StackFrame{
            symbols: SymbolTable::new(),
            current_function: current_function,
        }
    }
}

pub struct Context
{
    pub context: LLVMContextRef,
    pub module: LLVMModuleRef,
    pub builder: LLVMBuilderRef,
    name: String,
    stack: Vec<StackFrame>,
    slice_type_cache: HashMap<String, LLVMTypeRef>,
}

impl Context
{
	pub fn new(module_name: &str) -> Context
	{
		unsafe {
            let cname = CString::new(module_name).expect("Invalid module name");
            let context = LLVMContextCreate();
            Context{
                context: context,
                module: LLVMModuleCreateWithNameInContext(cname.as_ptr(), context),
                builder: LLVMCreateBuilderInContext(context),
                name: module_name.into(),
                stack: vec![StackFrame::new(ptr::null_mut())],
                slice_type_cache: HashMap::new(),
            }
        }
	}

    pub fn add_variable(&mut self, var: Rc<VariableInstance>)
    {
        self.stack.last_mut().expect("Stack is empty").symbols.add_variable(var)
    }

    pub fn get_variable(&self, name: &str) -> Option<Rc<VariableInstance>>
    {
        for sf in self.stack.iter().rev()
        {
            let v = sf.symbols.get_variable(name);
            if v.is_some() {
                return v;
            }
        }

        None
    }

    pub fn add_function(&mut self, f: Rc<FunctionInstance>)
    {
        self.stack.last_mut().expect("Stack is empty").symbols.add_function(f)
    }

    pub fn get_function(&self, name: &str) -> Option<Rc<FunctionInstance>>
    {
        for sf in self.stack.iter().rev()
        {
            let func = sf.symbols.get_function(name);
            if func.is_some() {
                return func;
            }
        }

        None
    }

    pub fn push_stack(&mut self, func: LLVMValueRef)
    {
        self.stack.push(StackFrame::new(func));
    }

    pub fn pop_stack(&mut self)
    {
        self.stack.pop();
    }

    pub fn get_current_function(&self) -> LLVMValueRef
    {
        for sf in self.stack.iter().rev()
        {
            if sf.current_function != ptr::null_mut() {
                return sf.current_function;
            }
        }

        panic!("No current function on stack, we should have caught this !");
    }

    pub unsafe fn gen_object_file(&self, build_dir: &str) -> CompileResult<String>
    {
        let target_triple = CStr::from_ptr(LLVMGetDefaultTargetTriple());
        let target_triple_str = target_triple.to_str().expect("Invalid target triple");
        println!("Compiling for {}", target_triple_str);

        let mut target: LLVMTargetRef = ptr::null_mut();
        let mut error_message: *mut c_char = ptr::null_mut();
        if LLVMGetTargetFromTriple(target_triple.as_ptr(), &mut target, &mut error_message) != 0 {
            let msg = CStr::from_ptr(error_message).to_str().expect("Invalid C string");
            let e = format!("Unable to get an LLVM target reference for {}: {}", target_triple_str, msg);
            LLVMDisposeMessage(error_message);
            return err(Pos::zero(), ErrorCode::CodegenError, e);
        }

        let target_machine = LLVMCreateTargetMachine(
            target,
            target_triple.as_ptr(),
            cstr(""),
            cstr(""),
            LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
            LLVMRelocMode::LLVMRelocDefault,
            LLVMCodeModel::LLVMCodeModelDefault,
        );
        if target_machine == ptr::null_mut() {
            let e = format!("Unable to get a LLVM target machine for {}", target_triple_str);
            return err(Pos::zero(), ErrorCode::CodegenError, e);
        }

        try!(DirBuilder::new()
            .recursive(true)
            .create(build_dir)
            .map_err(|e| CompileError::new(
                Pos::zero(),
                ErrorCode::CodegenError,
                format!("Unable to create directory for {}: {}", build_dir, e))));


        let obj_file_name = format!("{}/{}.cobra.o", build_dir, self.name);
        println!("  Building {}", obj_file_name);

        let mut error_message: *mut c_char = ptr::null_mut();
        if LLVMTargetMachineEmitToFile(target_machine, self.module, cstr_mut(&obj_file_name), LLVMCodeGenFileType::LLVMObjectFile, &mut error_message) != 0 {
            let msg = CStr::from_ptr(error_message).to_str().expect("Invalid C string");
            let e = format!("Unable to create object file: {}", msg);
            LLVMDisposeMessage(error_message);
            LLVMDisposeTargetMachine(target_machine);
            return err(Pos::zero(), ErrorCode::CodegenError, e);
        }


        LLVMDisposeTargetMachine(target_machine);
        Ok(obj_file_name)
    }

    pub fn optimize(&self) -> CompileResult<()>
    {
        unsafe{
            use llvm::transforms::pass_manager_builder::*;

            let pmb = LLVMPassManagerBuilderCreate();
            let pm = LLVMCreateFunctionPassManagerForModule(self.module);
            LLVMInitializeFunctionPassManager(pm);

            LLVMPassManagerBuilderSetOptLevel(pmb, 2);
            LLVMPassManagerBuilderPopulateFunctionPassManager(pmb, pm);

            let mut func = LLVMGetFirstFunction(self.module);
            while func != ptr::null_mut() {
                LLVMRunFunctionPassManager(pm, func);
                func = LLVMGetNextFunction(func);
            }

            LLVMDisposePassManager(pm);
            LLVMPassManagerBuilderDispose(pmb);
        }
        Ok(())
    }

    pub fn verify(&self) -> CompileResult<()>
    {
        use llvm::analysis::*;
        unsafe {
            let mut error_message: *mut c_char = ptr::null_mut();
            if LLVMVerifyModule(self.module, LLVMVerifierFailureAction::LLVMReturnStatusAction, &mut error_message) != 0 {
                let msg = CStr::from_ptr(error_message).to_str().expect("Invalid C string");
                let e = format!("Module verification error: {}", msg);
                LLVMDisposeMessage(error_message);
                err(Pos::zero(), ErrorCode::CodegenError, e)
            } else {
                Ok(())
            }
        }
    }

    #[cfg(test)]
    pub fn take_module_ref(&mut self) -> LLVMModuleRef
    {
        use std::mem;
        mem::replace(&mut self.module, ptr::null_mut())
    }

    pub unsafe fn get_slice_type(&mut self, element_type: LLVMTypeRef) -> LLVMTypeRef
    {
        let name = format!("slice-{}", type_name(element_type));
        if let Some(t) = self.slice_type_cache.get(&name) {
            return *t;
        }

        let slice_type = new_slice_type(self.context, element_type);
        self.slice_type_cache.insert(name, slice_type);
        slice_type
    }

    pub unsafe fn resolve_type(&mut self, typ: &Type) -> Option<LLVMTypeRef>
    {
        match *typ
        {
            Type::Void => Some(LLVMVoidTypeInContext(self.context)),
            Type::Int => Some(LLVMInt64TypeInContext(self.context)),
            Type::Bool => Some(LLVMInt1TypeInContext(self.context)),
            Type::Float => Some(LLVMDoubleTypeInContext(self.context)),
            Type::Array(ref et, len) => {
                self.resolve_type(et).map(|et| LLVMArrayType(et, len as u32))
            },
            Type::Slice(ref et) => {
                self.resolve_type(et).map(|et| self.get_slice_type(et))
            },
            _ => None,
        }
    }

    pub fn in_global_context(&self) -> bool
    {
        self.stack.len() == 1
    }

}


impl Drop for Context
{
    fn drop(&mut self)
    {
        unsafe {
            LLVMDisposeBuilder(self.builder);
            if self.module != ptr::null_mut() {
                LLVMDisposeModule(self.module);
            }
            LLVMContextDispose(self.context);
        }
    }
}