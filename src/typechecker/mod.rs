mod typecheck;
mod typecheckercontext;
mod instantiate;
mod instantiategenerics;
mod genericmapper;
mod matchchecker;
mod typeresolver;
#[cfg(test)]
mod tests;

pub use self::typecheck::{type_check_module};
