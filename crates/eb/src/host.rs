//! The host side of the plugin system: the rutis runtime, the mount of the
//! built-in plugins, and the validated contribution set the renderers read.
//!
//! Threading: plugin *mounting* and contribution reads happen on the GTK main
//! thread. Agent answers run on [`AgentRuntime`]'s dedicated thread; the host
//! drains their events on the main loop. Plugin code never touches a widget.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use rutis::{BoxFuture, Ctx, CordisError, Effect};

use eb_api::contrib::Contribs;
use eb_api::{AnswerSource, EbPlugin, Ui};
use crate::agent::AgentRuntime;

static ACP_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn acp_enabled() -> bool {
    ACP_ACTIVE.load(Ordering::SeqCst)
}

fn acp_command(program: std::path::PathBuf, args: &[&str], name: &str) -> Option<eb_plugin_acp_agent::AcpAgent> {
    if !program.is_file() {
        return None;
    }
    let auto_approve = std::env::var("EVOBUDDY_ACP_AUTO_APPROVE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    Some(eb_plugin_acp_agent::AcpAgent::new(eb_plugin_acp_agent::AcpConfig {
        program: program.to_string_lossy().to_string(),
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
        cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        auto_approve,
        name: name.into(),
        debug: std::env::var_os("EVOBUDDY_ACP_DEBUG").is_some(),
        env: crate::agents::external_env(),
        auth_method: std::env::var("EVOBUDDY_ACP_AUTH_METHOD").ok(),
    }))
}

/// Locally installed ACP agents. Explicit `EVOBUDDY_ACP_AGENT` wins; otherwise
/// opencode remains the default and Grok Build is the fallback.
fn default_acp_agent() -> Option<eb_plugin_acp_agent::AcpAgent> {
    let selected = crate::agents::selected();
    let build = |agent: crate::agents::AcpAgent| {
        acp_command(agent.program(), &agent.args(), agent.label())
    };
    match selected {
        crate::agents::AcpAgent::Grok => build(selected),
        crate::agents::AcpAgent::Opencode => build(selected).or_else(|| {
            acp_command(
                crate::agents::AcpAgent::Grok.program(),
                &crate::agents::AcpAgent::Grok.args(),
                "Grok build",
            )
        }),
    }
}

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

        // One router is the answer source for the process lifetime. It
        // resolves the current default on every turn, so Settings can switch
        // between ACP, dsh, and a custom model without remounting the host.
        let router = Arc::new(crate::router::AgentRouter::new(crate::router::load()));
        crate::router::install(router.clone());
        // The fake-agent harness is a test double, not a routable provider:
        // keep it as the sole answer source so route defaults cannot bypass it.
        if std::env::var_os("EB_FAKE_AGENT").is_some() {
            eb_api::provide_answer(&ctx, Arc::new(eb_plugin_demo_agent::FakeAgent::from_env()))
                .expect("provide the fake agent");
        } else {
            eb_api::provide_answer(&ctx, router.clone())
                .expect("provide the agent router");
        }
        ACP_ACTIVE.store(
            crate::router::load().default == crate::router::DefaultRoute::Acp,
            Ordering::SeqCst,
        );
        let _ = crate::dsh_agent::enabled();
        let builtins: Vec<Arc<dyn EbPlugin>> = vec![
            Arc::new(eb_plugin_nav::NavPlugin),
            Arc::new(eb_plugin_notices::NoticesPlugin),
            Arc::new(eb_plugin_composer::ComposerPlugin),
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

        let mut contribs = contribs.lock().unwrap().clone();
        if let Some((_, items)) = contribs.menus.iter_mut().find(|(name, _)| *name == "model") {
            *items = crate::router::menu_labels();
        }
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
