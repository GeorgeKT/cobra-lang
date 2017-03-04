use std::rc::{Rc, Weak};
use std::cell::RefCell;
use std::fmt;
use super::ExecutionResult;
use super::value::Value;

#[derive(Debug, Clone)]
pub enum ValueRef
{
    Owner(Rc<RefCell<Value>>),
    Ptr(Weak<RefCell<Value>>),
    Null,
}

impl ValueRef
{
    pub fn new(v: Value) -> ValueRef
    {
        ValueRef::Owner(Rc::new(RefCell::new(v)))
    }

    pub fn clone_value(&self) -> ExecutionResult<Value>
    {
        match *self
        {
            ValueRef::Owner(ref v) => Ok(v.borrow().clone()),
            ValueRef::Ptr(ref v) => {
                if let Some(rv) = v.upgrade() {
                    Ok(rv.borrow().clone())
                } else {
                    Err("Dangling pointer, owner of element pointed to is gone".into())
                }
            }
            ValueRef::Null => {
                Err("Dangling pointer, pointer has been deleted".into())
            }
        }
    }

    pub fn to_ptr(&self) -> ValueRef
    {
        if let ValueRef::Owner(ref v) = *self {
            ValueRef::Ptr(Rc::downgrade(v))
        } else {
            self.clone()
        }
    }

    pub fn apply<Op, R>(&self, op: Op) -> ExecutionResult<R>
        where Op: Fn(&Value) -> ExecutionResult<R>, R: Sized
    {
        match *self
        {
            ValueRef::Owner(ref v) => {
                op(&v.borrow())
            },
            ValueRef::Ptr(ref v) => {
                if let Some(rv) = v.upgrade() {
                    op(&rv.borrow())
                } else {
                    Err("Dangling pointer, owner of element pointed to is gone".into())
                }
            }
            ValueRef::Null => {
                Err("Dangling pointer, pointer has been deleted".into())
            }
        }
    }

    pub fn apply_mut<Op, R>(&mut self, op: Op) -> ExecutionResult<R>
        where Op: FnOnce(&mut Value) -> ExecutionResult<R>, R: Sized
    {
        match *self
        {
            ValueRef::Owner(ref v) => {
                op(&mut v.borrow_mut())
            },
            ValueRef::Ptr(ref v) => {
                if let Some(rv) = v.upgrade() {
                    op(&mut rv.borrow_mut())
                } else {
                    Err("Dangling pointer, owner of element pointed to is gone".into())
                }
            }
            ValueRef::Null => {
                Err("Dangling pointer, pointer has been deleted".into())
            }
        }
    }
}

impl fmt::Display for ValueRef
{
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error>
    {
        match *self
        {
            ValueRef::Owner(ref v) => {
                let inner = v.borrow();
                write!(f, "*({})", *inner)
            },

            ValueRef::Ptr(ref v) => {
                if let Some(rv) = v.upgrade() {
                    let inner = rv.borrow();
                    write!(f, "*({})", *inner)
                } else {
                    println!("Danging pointer access");
                    Err(fmt::Error)
                }
            },

            ValueRef::Null => {
                write!(f, "null")
            },
        }
    }
}