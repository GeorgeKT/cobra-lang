use std::io::prelude::*;
use shrust::{Shell, ShellIO, ExecResult, ExecError};
use bytecode::*;
use super::*;
use super::value::Value;


#[derive(Debug, Clone)]
pub struct ByteCodeIndex
{
    pub function: String,
    pub basic_block: BasicBlockRef,      // current basic block
    pub instruction: usize,     // current instruction in the basic block
}

impl ByteCodeIndex
{
    pub fn new<S: Into<String>>(function: S, bb_ref: BasicBlockRef, instruction: usize) -> ByteCodeIndex
    {
        ByteCodeIndex{
            function: function.into(),
            basic_block: bb_ref,
            instruction: instruction,
        }
    }

    pub fn next(&self) -> ByteCodeIndex
    {
        ByteCodeIndex{
            function: self.function.clone(),
            basic_block: self.basic_block,
            instruction: self.instruction + 1,
        }
    }

    pub fn jump(&self, destination_block: BasicBlockRef) -> ByteCodeIndex
    {
        ByteCodeIndex{
            function: self.function.clone(),
            basic_block: destination_block,
            instruction: 0,
        }
    }


    pub fn print(&self, module: &ByteCodeModule)
    {
        if module.get_function(&self.function)
            .and_then(|function| function.blocks.get(&self.basic_block))
            .and_then(|bb| bb.instructions.get(self.instruction))
            .map(|inst| print!("{}", inst))
            .is_none()
        {
            panic!("Invalid Location (function: {},  bb: {}, instruction: {})", self.function, self.basic_block, self.instruction);
        }
    }

}

struct DebuggerContext<'a>
{
    interpreter: Interpreter,
    index: ByteCodeIndex,
    module: &'a ByteCodeModule
}

fn help(io: &mut ShellIO, _dc: &mut DebuggerContext) -> ExecResult
{
    writeln!(io, r#"
Cobra interpreter debugger commands:
    help, h:                print this help message
    quit, q:                quit the debugger
    print <var>, p <var>:   print a variable
    step, s:                Step one instruction
    continue, c:            Continue running
    "#)?;
    Ok(())
}

fn quit(io: &mut ShellIO, _dc: &mut DebuggerContext) -> ExecResult
{
    writeln!(io, "Quiting ...")?;
    Err(ExecError::Quit)
}

fn step(io: &mut ShellIO, dc: &mut DebuggerContext) -> ExecResult
{
    dc.index = match dc.interpreter.step(&dc.index, dc.module)
    {
        Ok(StepResult::Continue(new_index)) => {
            new_index.print(dc.module);
            new_index
        },

        Ok(StepResult::Exit(return_value)) => {
            writeln!(io, "Program exited with return value {}", return_value)?;
            return Err(ExecError::Quit)
        },

        Err(msg) => {
            writeln!(io, "Execution error: {}", msg)?;
            return Err(ExecError::Quit)
        }
    };

    Ok(())
}

fn print(io: &mut ShellIO, dc: &mut DebuggerContext, args: &[&str]) -> ExecResult
{
    match dc.interpreter.get_variable(args[0])
    {
        Ok(ref v) => {
            writeln!(io, "{} = {}", args[0], v)?;
            Ok(())
        },

        Err(ref e) => {
            writeln!(io, "{}", e)?;
            Ok(())
        }
    }
}

fn cont(io: &mut ShellIO, dc: &mut DebuggerContext) -> ExecResult
{
    loop {
        step(io, dc)?;
    }
}

pub fn debug_byte_code(module: &ByteCodeModule) -> ExecutionResult<Value>
{
    let mut interpreter = Interpreter::new(true);
    let function = module.main_function_name();
    let index = interpreter.start(&function, module)?;
    index.print(module);
    let mut shell = Shell::new(DebuggerContext{
        interpreter: interpreter,
        index: index,
        module: module,
    });
    shell.set_prompt("dbg>".into());
    shell.new_command_noargs("help", "Print help", help);
    shell.new_command_noargs("h", "Print help", help);
    shell.new_command_noargs("quit", "Quit", quit);
    shell.new_command_noargs("q", "Quit", quit);
    shell.new_command_noargs("step", "Step", step);
    shell.new_command_noargs("s", "Step", step);
    shell.new_command_noargs("cont", "Continue", cont);
    shell.new_command_noargs("c", "Continue", cont);
    shell.new_command("print", "Print", 1, print);
    shell.new_command("p", "Print", 1, print);
    shell.run_loop(&mut ShellIO::default());
    Ok(Value::Int(5))
}
