//! Opt-in XEmbed host for `EB_PREVIEW=webview` on Linux.
//!
//! The helper is compiled into this binary and extracted on first use. It
//! dlopens the system WebKitGTK and accepts only 2.28. This process does not
//! link libwebkit and does not set `GDK_BACKEND`.

use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;

use gtk::prelude::*;

use crate::preview_optin::{forward_helper_env, helper_im_module, refuse_reason};

struct Host {
    socket: gtk::Socket,
    stdin: Option<ChildStdin>,
    pid: Option<u32>,
    plug_id: Option<i32>,
    adopted: bool,
    failed: bool,
    started: bool,
    visible: bool,
    plugged: bool,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    static FALLBACK: RefCell<Option<String>> = const { RefCell::new(None) };
    static LATE: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static TIMEOUT_ARMED: Cell<bool> = const { Cell::new(false) };
}

const HELPER_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/eb-preview-helper"));

pub fn set_late_fallback(handler: Rc<dyn Fn()>) {
    LATE.with(|slot| *slot.borrow_mut() = Some(handler));
}

pub fn take_fallback() -> Option<String> {
    FALLBACK.with(|slot| slot.borrow_mut().take())
}

pub fn attach(socket: gtk::Socket) {
    socket.connect_plug_added(|_| {
        HOST.with(|slot| {
            if let Some(host) = slot.borrow_mut().as_mut() {
                host.adopted = true;
            }
        });
    });
    socket.connect_plug_removed(|_| {
        request_fallback("preview plug was removed. Showing the native view");
        true
    });
    // Quitting destroys the socket. Stop the helper without treating that as
    // an embed failure, or the exit path prints a fallback line.
    socket.connect_destroy(|_| stop_helper());
    HOST.with(|slot| {
        *slot.borrow_mut() = Some(Host {
            socket,
            stdin: None,
            pid: None,
            plug_id: None,
            adopted: false,
            failed: false,
            started: false,
            visible: false,
            plugged: false,
        });
    });
}

pub fn present() {
    let attached = HOST.with(|slot| slot.borrow().is_some());
    if !attached {
        return;
    }
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.visible = true;
        }
    });
    if failed() || adopted() {
        return;
    }
    if started() {
        adopt_pending();
        return;
    }
    if !display_is_x11() {
        request_fallback(
            "Linux CodeMirror verification needs an X11 display (GDK_BACKEND=x11 EB_PREVIEW=webview). Showing the native view",
        );
        return;
    }
    let Some(url) = crate::preview_web_server::shared_preview_url() else {
        request_fallback("preview server did not start. Showing the native view");
        return;
    };
    if url.contains('\n') || url.contains('\r') {
        request_fallback("preview url was rejected. Showing the native view");
        return;
    }
    match spawn_helper(&url) {
        Ok(()) => arm_timeout(),
        Err(error) => request_fallback(&format!(
            "preview helper did not start ({error}). Showing the native view"
        )),
    }
}

pub fn dismiss() {
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.visible = false;
        }
    });
}

pub fn wake() {
    HOST.with(|slot| {
        let mut hosts = slot.borrow_mut();
        let Some(host) = hosts.as_mut() else {
            return;
        };
        if !host.adopted || !host.visible {
            return;
        }
        if let Some(stdin) = host.stdin.as_mut() {
            let _ = stdin.write_all(b"WAKE\n");
            let _ = stdin.flush();
        }
    });
}

pub fn retry_present() {
    if !visible() || failed() || adopted() {
        return;
    }
    adopt_pending();
}

fn visible() -> bool {
    HOST.with(|slot| slot.borrow().as_ref().is_some_and(|host| host.visible))
}

fn failed() -> bool {
    HOST.with(|slot| slot.borrow().as_ref().is_some_and(|host| host.failed))
}

fn adopted() -> bool {
    HOST.with(|slot| slot.borrow().as_ref().is_some_and(|host| host.adopted))
}

fn started() -> bool {
    HOST.with(|slot| slot.borrow().as_ref().is_some_and(|host| host.started))
}

fn display_is_x11() -> bool {
    gtk::gdk::Display::default()
        .map(|display| display.type_().name() == "GdkX11Display")
        .unwrap_or(false)
}

fn arm_timeout() {
    if TIMEOUT_ARMED.with(|cell| cell.replace(true)) {
        return;
    }
    gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(8), || {
        if started() && !adopted() && !failed() {
            request_fallback("preview helper did not embed within 8s. Showing the native view");
        }
    });
}

fn spawn_helper(url: &str) -> std::io::Result<()> {
    let path = helper_path()?;
    let mut child = Command::new(&path)
        .env_clear()
        .envs(helper_env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let pid = child.id();
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        return Err(std::io::Error::other("helper stdin was missing"));
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err(std::io::Error::other("helper stdout was missing"));
    };
    if stdin
        .write_all(format!("PLUG_URL {url}\n").as_bytes())
        .and_then(|()| stdin.flush())
        .is_err()
    {
        let _ = child.kill();
        return Err(std::io::Error::other(
            "helper did not accept the preview url",
        ));
    }
    HOST.with(|slot| {
        if let Some(host) = slot.borrow_mut().as_mut() {
            host.stdin = Some(stdin);
            host.pid = Some(pid);
            host.started = true;
        }
    });
    std::thread::spawn(move || read_helper(stdout, child));
    Ok(())
}

fn helper_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(key, _)| forward_helper_env(key))
        .collect();
    env.push(("GDK_BACKEND".to_string(), "x11".to_string()));
    if let Some(module) = helper_im_module(std::env::var("GTK_IM_MODULE").ok().as_deref()) {
        env.push(("GTK_IM_MODULE".to_string(), module.to_string()));
    }
    env
}

fn helper_path() -> std::io::Result<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let dir = std::env::temp_dir().join(format!("evobuddy-preview-{uid}"));
    std::fs::create_dir_all(&dir)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    let path = dir.join("eb-preview-helper");
    if std::fs::read(&path).unwrap_or_default().as_slice() != HELPER_BYTES {
        let tmp = dir.join("eb-preview-helper.tmp");
        std::fs::write(&tmp, HELPER_BYTES)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700))?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(path)
}

fn read_helper(stdout: std::process::ChildStdout, mut child: Child) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        gtk::glib::idle_add_once(move || on_helper_line(&line));
    }
    let status = child.wait();
    gtk::glib::idle_add_once(move || on_helper_exit(status));
}

fn on_helper_line(line: &str) {
    if let Some(id) = line.strip_prefix("PLUG ") {
        match id.trim().parse::<i32>() {
            Ok(id) if id > 0 => {
                HOST.with(|slot| {
                    if let Some(host) = slot.borrow_mut().as_mut() {
                        host.plug_id = Some(id);
                    }
                });
                adopt_pending();
            }
            _ => {
                request_fallback("preview helper reported a plug id of 0. Showing the native view")
            }
        }
        return;
    }
    if line.starts_with("REFUSE") {
        request_fallback(&refuse_reason(line));
        return;
    }
    if line == "CRASHED" {
        request_fallback("preview web process crashed. Showing the native view");
    }
}

fn on_helper_exit(status: std::io::Result<std::process::ExitStatus>) {
    if failed() {
        return;
    }
    if adopted() {
        let code = status.ok().and_then(|status| status.code());
        if code == Some(0) {
            return;
        }
        request_fallback("preview helper exited. Showing the native view");
        return;
    }
    request_fallback("preview helper exited before it embedded. Showing the native view");
}

fn adopt_pending() {
    if failed() || adopted() {
        return;
    }
    let (socket, id) = HOST.with(|slot| {
        let host = slot.borrow();
        let Some(host) = host.as_ref() else {
            return (None, None);
        };
        (Some(host.socket.clone()), host.plug_id)
    });
    let (Some(socket), Some(id)) = (socket, id) else {
        return;
    };
    if socket.window().is_none() {
        socket.realize();
    }
    if socket.window().is_none() {
        return;
    }
    let plugged = HOST.with(|slot| slot.borrow().as_ref().is_some_and(|host| host.plugged));
    if !plugged {
        gtk::gdk::error_trap_push();
        socket.add_id(id as gtk::xlib::Window);
        gtk::gdk::flush();
        let trap = gtk::gdk::error_trap_pop();
        HOST.with(|slot| {
            if let Some(host) = slot.borrow_mut().as_mut() {
                host.plugged = true;
            }
        });
        if trap != 0 {
            request_fallback("preview socket rejected the plug. Showing the native view");
            return;
        }
    }
    if socket.plug_window().is_some() {
        HOST.with(|slot| {
            if let Some(host) = slot.borrow_mut().as_mut() {
                host.adopted = true;
            }
        });
    }
}

fn stop_helper() {
    let _ = mark_failed_and_kill();
}

fn mark_failed_and_kill() -> bool {
    HOST.with(|slot| {
        let mut hosts = slot.borrow_mut();
        let Some(host) = hosts.as_mut() else {
            return true;
        };
        if host.failed {
            return false;
        }
        host.failed = true;
        if let Some(pid) = host.pid.take() {
            let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
        true
    })
}

fn request_fallback(message: &str) {
    if !mark_failed_and_kill() {
        return;
    }
    FALLBACK.with(|slot| {
        if slot.borrow().is_none() {
            *slot.borrow_mut() = Some(message.to_string());
        }
    });
    gtk::glib::idle_add_once(|| {
        LATE.with(|slot| {
            if let Some(handler) = slot.borrow().as_ref().cloned() {
                handler();
            }
        });
    });
}
