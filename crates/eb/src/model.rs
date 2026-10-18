//! The application data vocabulary lives in `eb-api`, where the plugins can
//! see it; this re-export keeps the host's `crate::model::` paths working.

pub use eb_api::model::*;
