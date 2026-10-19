//! The host side of the plugin system: the rutis runtime, the mount of the
//! built-in plugins, and the validated contribution set the renderers read.
//!
//! Threading: plugin *mounting* and contribution reads happen on the GTK main
//! thread. Agent answers run on [`AgentRuntime`]'s dedicated thread; the host
//! drains their events on the main loop. Plugin code never touches a widget.

use std::sync::{Arc, Mutex};

use rutis::{BoxFuture, Ctx, CordisError, Effect};

use eb_api::contrib::Contribs;
use eb_api::{AnswerSource, EbPlugin, Ui};
use crate::agent::AgentRuntime;

/// The mounted plugin system.
pub struct Host {
    /// Lives as long as the host: dropping the runtime shuts the driver down —
    /// the ctx's handle does not keep it alive. Used only for the mount.
    #[allow(dead_code)]
    pub mount_runtime: tokio::runtime::Runtime,
    pub ctx: Ctx,
    pub contribs: Contribs,
    /// Owns the dedicated agent thread and its tokio runtime.
    pub agent_runtime: Arc<AgentRuntime>,
}

impl Host {
    /// Mount the built-in plugins. A plugin that fails to mount aborts the
    /// startup loudly — a missing contribution would render as a silently
    /// wrong UI, and rutis has already rolled the failed plugin back.
    pub fn mount() -> Host {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("plugin runtime");
        let ctx = Ctx::root_with(runtime.handle().clone());
        let contribs = Arc::new(Mutex::new(Contribs::default()));

        // One answer source, selected at mount time. An ACP command means a
        // real external agent; otherwise the built-in demo keeps the UI
        // runnable with no configuration.
        let answer: Arc<dyn EbPlugin> = if std::env::var("EB_AGENT").as_deref() == Ok("dsh") {
            Arc::new(crate::dsh_agent::DshAgent)
        } else {
            match eb_plugin_acp_agent::AcpAgent::from_env() {
                Some(agent) => Arc::new(agent),
                None => Arc::new(eb_plugin_demo_agent::DemoAgent::new()),
            }
        };
        let builtins: Vec<Arc<dyn EbPlugin>> = vec![
            Arc::new(eb_plugin_nav::NavPlugin),
            Arc::new(eb_plugin_notices::NoticesPlugin),
            Arc::new(eb_plugin_composer::ComposerPlugin),
            answer,
        ];

        let mut fibers = Vec::new();
        for plugin in &builtins {
            let fiber = ctx.plugin(PluginAdapter {
                id: plugin.id(),
                plugin: plugin.clone(),
                contribs: contribs.clone(),
            });
            fibers.push((plugin.id(), fiber));
        }
        runtime.block_on(async {
            for (id, fiber) in &fibers {
                if let Err(error) = fiber.await {
                    panic!("plugin '{id}' failed to mount: {error}");
                }
            }
        });

        let contribs = contribs.lock().unwrap().clone();
        let agent_runtime = Arc::new(AgentRuntime::new());
        let host = Host {
            mount_runtime: runtime,
            ctx,
            contribs,
            agent_runtime,
        };
        host.validate();
        host
    }

    /// The answer source behind the composer's send path.
    pub fn answer(&self) -> Arc<dyn AnswerSource> {
        eb_api::answer_source(&self.ctx).expect("no plugin provided an answer source")
    }

    /// The contributions the renderers cannot do without. Listed here rather
    /// than sprinkled through the renderers, so a plugin regression names the
    /// missing piece instead of drawing a silently wrong window.
    fn validate(&self) {
        assert!(
            !self.contribs.nav.is_empty(),
            "no plugin contributed nav rows"
        );
        for id in [
            "sidebar.search",
            "sidebar.mini_programs",
            "detail.new_tab",
            "composer.code_assist",
        ] {
            self.contribs.notice(id);
        }
        for name in ["workspace", "permission", "model", "add", "filter", "account"] {
            self.contribs.menu(name);
        }
        self.contribs.composer_copy();
        assert!(
            !self.contribs.chips.is_empty(),
            "no plugin contributed scenario chips"
        );
        self.answer();
    }
}

/// Adapts an [`EbPlugin`] to rutis's async plugin trait. The built-ins mount
/// synchronously; this is where that is made to look like a fiber.
struct PluginAdapter {
    id: &'static str,
    plugin: Arc<dyn EbPlugin>,
    contribs: Arc<Mutex<Contribs>>,
}

impl rutis::Plugin for PluginAdapter {
    fn name(&self) -> &str {
        self.id
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let ui = Ui { ctx, contribs: &self.contribs };
            self.plugin.mount(&ui)?;
            Ok(Effect::Done)
        })
    }
}
