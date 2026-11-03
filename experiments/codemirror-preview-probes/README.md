# CodeMirror 预览方案的隐患探针

对照 `docs/codemirror-preview-design.md`。这里只验证，不实现预览迁移。

重跑：

```sh
sh experiments/codemirror-preview-probes/run.sh
```

Linux 部分使用本机已有的 `evobuddy-build-aarch64`（Debian 10 / arm64，和 Linux 包同一套构建镜像），里面临时安装 `xvfb`。结果在 `results/`。

这次跑下来，下面几条被探针打中。没有在这台机器上跑的，列在末尾。

- 预览服务是单线程、一次最多读 2048 字节。先连上但不把请求写完，下一条请求收不到响应。`POST` 的正文如果晚到，循环已经按第一次读到的内容回了包。同一地址第二次 `bind` 失败，生产代码用 `else { return }` 把这个错误丢掉。任意本机 TCP 客户端都能读到响应，没有令牌。
- `packaging/linux/run.sh` 把 `LD_LIBRARY_PATH` 指到包内 `lib/` 再 `exec` 应用。Debian 10 里一个链了系统 GTK 的子进程，只要继承这个变量，动态链接器就会改去加载目录里的假 `libgtk-3.so.0`，打印 `STUB_LIBGTK_LOADED`，然后 `undefined symbol: gtk_init_check`（退出 127）。子进程在 `exec` 前清掉该变量，则加载 `/usr/lib/aarch64-linux-gnu/libgtk-3.so.0` 并正常 `gtk_init`。构建镜像的 `pkg-config webkit2gtk-4.0` 不存在；镜像源里的候选包是 `2.38.6-0+deb10u1`，不是设计写的 2.28.1。
- `GDK_BACKEND` 必须在 `gtk_init` 之前生效。Debian 10 上初始化之后再 `g_setenv(GDK_BACKEND, wayland)`，显示保持 `GdkX11Display`。初始化之前设成 `wayland`，`gtk_init_check` 失败。这台 Mac 上初始化之后设成 `x11`，显示保持 `GdkQuartzDisplay`；初始化之前设成 `x11`，`gtk_init_check` 直接失败。`main.rs` 在 `gtk::Application::new` 之前没有设置这个变量。
- XEmbed：`gtk_plug_get_id` 返回 2097155，和普通顶层窗口的 XID 2097159 不是同一个数。`gtk_socket_add_id` 用 plug id 能嵌进去；用 0 嵌不进。用普通顶层窗口的 XID，GTK 警告 `Can't add non-GtkPlug to GtkSocket`，随后进程在 GDK 的 X 错误陷阱里 abort。
- `tool_preview` 和 `task.artifacts` 不是一摞。写入文件会清掉 Read 预览，但留下已写文件。`begin_task` 换路线时不清除预览字段。`open_task` 和 `new_task` 清预览、不清已写文件。关单个标签只改 `task.artifacts`。
- `theme_view.rs` 标了 validation-only，只 `loadRequest`，会 `activateIgnoringOtherApps(true)`，没有 `size-allocate` 或 `configure-event`。
- wry 0.57.1 的创建入口是 `WebViewBuilder::build_as_child<W: HasWindowHandle>`。`SetParent` 出现在 `WebViewExtWindows::reparent` 内部，不是给应用自己拿 HWND 去挂的公开步骤。`set_bounds` 只对 child webview 有效。默认 features 是 `os-webview` 和 `x11`，会在 Linux target 上打开可选的 `gtk` 0.18 和 `webkit2gtk` 2.0.2。设计把 `wry` 放在 Windows-only 依赖里时，不会把这套 gtk 编进 macOS/Linux 构建。`linux-body` 要求 `webkit2gtk/v2_40`，不能当作 WebKitGTK 2.28 的后端。
- 当前 `objc2-web-kit` 依赖没有关 default features，解析结果里已经包含 `WKUserContentController`。关掉 default features、只留 `WKSnapshotConfiguration` 和 `block2` 时，这个类型才消失。设计把 Rust→JS 写成 `WKUserContentController` 仍然不对：绑定自己的说明是 JavaScript 向原生发消息，而 `theme_view.rs` 没有调用 `evaluateJavaScript`。

没有跑到的部分：Kylin 上的 WebKitGTK 2.28.1、XEmbed 里的键盘/中文输入/HiDPI、WebProcess 崩溃时 helper 仍活着、以及一台 Windows 上的真实 HWND 嵌套。
