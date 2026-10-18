//! EvoBuddy's plugin surface — the only crate a plugin may depend on.
//!
//! The architecture is *data plugins, host rendering*: a plugin runs wherever
//! the host runs it, looks at the data it is handed, and produces plain values —
//! view models, menu items, notice copy. It never touches GTK; the host owns
//! every widget and the rendering policy. That keeps the thread contract to one
//! sentence (plugin code never touches a widget) and keeps the pixel-faithful
//! rendering in one place.
//!
//! Three layers live here:
//!
//! | module     | holds                                                          |
//! |------------|----------------------------------------------------------------|
//! | `model`    | the application data vocabulary and its transitions            |
//! | `contrib`  | what a plugin can contribute: nav rows, menus, notices, chips… |
//! | the traits | [`EbPlugin`] (the mount entry point) and [`AnswerSource`]      |
//!
//! `Ui` is the handle a plugin's `mount` receives: it exposes the rutis context
//! (for providing services) and the contribution registries (for declaring UI
//! content). Plugins depend on this crate and on nothing else; the host must
//! never depend on a plugin crate's internals — the workspace layout enforces
//! both directions.

pub mod contrib;
pub mod model;

pub use rutis::CordisError;
use rutis::Ctx;

/// Everything a plugin needs at mount time.
pub struct Ui<'a> {
    /// The rutis context. Provide services through it (`ctx.provide_as`); what
    /// a plugin provides is visible to the host and to other plugins.
    pub ctx: &'a Ctx,
    /// The contribution registries. Declare UI content through it. The mutex
    /// exists to satisfy the mount futures' `Send` bound; all mounts run on one
    /// thread, so it is never contended.
    pub contribs: &'a std::sync::Mutex<contrib::Contribs>,
}

/// A plugin's mount entry point.
///
/// `mount` declares contributions and provides services; it should not do long
/// work. A failing `mount` fails the plugin's rutis fiber, which rolls the load
/// back rather than leaving a half-registered plugin behind.
pub trait EbPlugin: Send + Sync + 'static {
    fn id(&self) -> &'static str;
    fn mount(&self, ui: &Ui) -> Result<(), CordisError>;
}

/// Who answers a submitted prompt.
///
/// The built-in implementation is the demo agent (fixed placeholder turns that
/// exercise every message block). A real agent replaces it by providing this
/// service — nothing in the host changes.
pub trait AnswerSource: Send + Sync + 'static {
    /// Produce the agent's turn for a submitted prompt, plus the artifact (if
    /// any) the turn generated.
    fn answer(&self, prompt: &str) -> (model::Turn, Option<model::Artifact>);
}

/// Provide the answer source as a rutis service. The host reads it with
/// [`answer_source`]; both sides go through this helper so the type key — the
/// one piece of rutis plumbing a plugin should not have to know — stays here.
pub fn provide_answer(
    ctx: &Ctx,
    source: std::sync::Arc<dyn AnswerSource>,
) -> Result<rutis::Disposer, CordisError> {
    ctx.provide_as::<dyn AnswerSource>(rutis::TypeKey::of::<dyn AnswerSource>(), source)
}

/// Read the answer source, wherever a plugin provided it.
pub fn answer_source(ctx: &Ctx) -> Option<std::sync::Arc<dyn AnswerSource>> {
    ctx.get_as::<dyn AnswerSource>(rutis::TypeKey::of::<dyn AnswerSource>())
}
