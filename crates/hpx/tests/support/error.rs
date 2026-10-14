#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]
use std::error::Error as StdError;

#[allow(unused)]
pub(crate) fn inspect<E>(err: E) -> Vec<String>
where
    E: Into<Box<dyn StdError + Send + Sync>>,
{
    let berr = err.into();
    let mut err = Some(&*berr as &(dyn StdError + 'static));
    let mut errs = Vec::new();
    while let Some(e) = err {
        errs.push(e.to_string());
        err = e.source();
    }
    errs
}
