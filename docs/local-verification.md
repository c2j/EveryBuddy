# 本地启动与插件验证

这份清单用于在本机启动 EvoBuddy，并分别验证 **UI、四个内置插件、插件边界** 和
**自动化审计**。命令都在仓库根目录执行。

## 1. 准备环境

### macOS

```bash
brew install gtk+3 pkg-config
export PKG_CONFIG_PATH="/opt/homebrew/lib/pkgconfig:$PKG_CONFIG_PATH"
cargo run
```

如果只是验证打包后的自包含 app：

```bash
packaging/macos/bundle.sh
open dist/EvoBuddy.app
# 或直接跑二进制，便于看 stdout
dist/EvoBuddy.app/Contents/MacOS/evobuddy
```

未签名 app 首次启动可能需要右键 → Open，或
`xattr -dr com.apple.quarantine dist/EvoBuddy.app`。

### Linux

```bash
sudo apt install libgtk-3-dev
cargo run
```

打包版：

```bash
packaging/linux/package.sh
dist/app/run
# 或
dist/evobuddy-<version>-linux-<arch>.AppImage
```

`package.sh` 需要 Docker；AppImage 只能在原生架构上执行（qemu/Rosetta 会拒绝
`EI_ABIVERSION='AI'`，见 README）。

## 2. 自动化检查（先跑这一组）

> **跑 GUI harness 用临时 `HOME`**：任务收尾会把转录写回 `~/.evobuddy/tasks.json`，
> 真实存档会被短会话覆盖掉。`cargo` 要留在真实 `HOME` 下（rustup 读 `~/.rustup`），
> 所以固定套路是：真实 HOME 下 `cargo build`，再用
> `HOME=/tmp/eb-home-XX ./target/debug/evobuddy` 跑带旗标的二进制。

```bash
# 1) 编译、单元测试（33 步期望表 + 负向断言用例）
cargo test --workspace

# 2) 几何 / 颜色断言；失败退出非零
EB_ASSERT=1 cargo run

# 3) 33 步点击审计断言；失败退出非零
EB_CLICK_ASSERT=1 cargo run

# 4) 欢迎态 / 会话态截图（写到 /tmp）
EB_SELFTEST=1 EB_SHOT=/tmp/evobuddy-welcome.png cargo run
EB_DEMO=1 EB_SELFTEST=1 EB_SHOT=/tmp/evobuddy-chat.png cargo run
# M2.2 内容块：diff / exec / table，默认 752 宽会话列
EB_DEMO_BLOCKS=1 EB_NO_SCROLL=1 EB_SELFTEST=1 EB_SHOT=/tmp/evobuddy-blocks.png cargo run

# 5) 打开模型菜单并截图（检查 popover 内容确实有子控件）
EB_CLICK=1 EB_MENU_SHOT=/tmp/evobuddy-menu.png cargo run

# 6) 查看 widget 可见性/分配，排查“控件在但没显示”
EB_DEBUG=1 EB_SELFTEST=1 cargo run

# 7) fake streaming agent：提交、等流结束、自动退出
EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 cargo run

# 8) fake streaming agent + API 取消（应看到 Failed/Cancelled）
EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_CANCEL_AFTER_MS=300 \
  EB_FAKE_AGENT_CHUNKS=50 cargo run

# 9) fake streaming agent + UI stop 按钮（点击 send 变体）
EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_FAKE_STOP_AFTER_MS=250 \
  EB_FAKE_AGENT_CHUNKS=50 cargo run

# 10) start panic 必须变成 Failed，而不是带崩窗口
EB_FAKE_AGENT=1 EB_FAKE_DEMO=1 EB_DEBUG=1 EB_FAKE_PANIC_START=1 cargo run

# 11) Enter 发送链路
EB_ENTER=1 cargo run

# 12) ACP：fake server 驱动真实 UI（不依赖模型/网络）。
#     路由只认"槽位 + 可执行文件"，用 wrapper 把 fixture 塞进 opencode 槽位；
#     fixture 路径要在创建 wrapper 时展开为绝对路径（agent 的 cwd 是任务 workspace）。
cat > /tmp/fake-acp-wrapper.sh <<EOF
#!/bin/bash
exec python3 "$PWD/crates/plugins/eb-plugin-acp-agent/tests/fixtures/fake_acp.py" "\$@"
EOF
chmod +x /tmp/fake-acp-wrapper.sh
EVOBUDDY_ACP_AGENT=opencode EVOBUDDY_OPENCODE_BIN=/tmp/fake-acp-wrapper.sh \
EVOBUDDY_ACP_AUTO_APPROVE=1 EB_ACP_DEMO=1 EB_NO_SCROLL=1 \
EB_SHOT=/tmp/evobuddy-acp.png EB_SHOT_AFTER_MS=2500 cargo run

# 12b) 同一配方验证"结束条件"两半：EVOBUDDY_ACP_DEBUG=1 会打
#      `acp: session activity working/idle` 与 `acp: tail closed after …（activity=…）`，
#      EVOBUDDY_ACP_LOG 把 agent 发来的每条原始 JSON 落成转写本。
#      换场景只要让 wrapper 导出 FAKE_ACP_SCENARIO（tail / tail_busy / tail_error /
#      past_idle / backlog / retry）。resume 与 idle_between 要同一 session 连发两三轮
#      prompt，一次 EB_ACP_DEMO 跑不出来，由那两个跨轮结束条件用例覆盖：
cat > /tmp/fake-acp-tailbusy.sh <<EOF
#!/bin/bash
FAKE_ACP_SCENARIO=tail_busy exec python3 "$PWD/crates/plugins/eb-plugin-acp-agent/tests/fixtures/fake_acp.py" "\$@"
EOF
chmod +x /tmp/fake-acp-tailbusy.sh
EVOBUDDY_ACP_AGENT=opencode EVOBUDDY_OPENCODE_BIN=/tmp/fake-acp-tailbusy.sh \
EVOBUDDY_ACP_AUTO_APPROVE=1 EB_ACP_DEMO=1 EVOBUDDY_ACP_DEBUG=1 \
EVOBUDDY_ACP_LOG=/tmp/evobuddy-acp.jsonl \
EB_SHOT=/tmp/evobuddy-acp-tail.png EB_SHOT_AFTER_MS=6000 cargo run
#     期望：`working` 撑过 2s 探针 → 两段 research 正文出现在同一轮 → `idle` 后
#     2s 落终态；截图里动作条（复制/赞/踩/重答）已经出现，说明轮真的结束了。

# 13) 鼠标发送路径（等价点击 send，而不是 Enter/直接 Runner API）
EB_SEND_TEST=1 EB_SHOT=/tmp/evobuddy-send.png EB_SHOT_AFTER_MS=2000 cargo run

# 14) ACP：本机 opencode（默认路由就是 ACP·opencode，自动探测 ~/.opencode/bin/opencode；
#     工具调用会弹 allow/reject 弹层；无人值守可加 EVOBUDDY_ACP_AUTO_APPROVE=1 绕过）
cargo run

# 15) 构建返回后的真实交互（不加 selftest；验证 Views 生命周期）
EB_SEND_AFTER_MS=1000 EB_DEBUG=1 cargo run

# 16) dsh 路线（进程内桥，不再走 ACP profile；key 只放本地环境）
EB_AGENT=dsh DEEPSEEK_API_KEY="…" cargo run

# 17) Artifact 侧栏宽度（窗口自适应回归）。EB_WINDOW_WIDTH/HEIGHT 覆盖
#     fit_to_workarea 的钳制，用来模拟宽/窄窗口：
#     1440 基线 sash=860、eb-detail 579x836；宽窗口 sash=width-580、面板恒为 580。
#     窄窗口三段保底（spec §10-31）：1136→sash=804（会话先让），
#     1100→768、1082（窗口有效最小宽，拉不更窄）→750，即面板保住自己的
#     内容最小宽 332。加 EB_DEBUG=1 会打印
#     `layout: width=… open=… pos=… detail_min=… detail_nat=… conv_min=…`，
#     `pos` 必须与消息列 margin 用的 pane_width 同源。
EB_ARTIFACT=1 EB_ASSERT=1 cargo run
EB_WINDOW_WIDTH=1920 EB_WINDOW_HEIGHT=1000 EB_ARTIFACT=1 EB_SELFTEST=1 cargo run
EB_WINDOW_WIDTH=1100 EB_WINDOW_HEIGHT=800 EB_ARTIFACT=1 \
EB_SHOT=/tmp/eb-narrow.png EB_SHOT_AFTER_MS=6000 cargo run
# 保底边界：窗口拉到 900 也只会得到 1082，断言仍应通过
EB_WINDOW_WIDTH=900 EB_WINDOW_HEIGHT=800 EB_ARTIFACT=1 EB_ASSERT=1 EB_DEBUG=1 cargo run

# 18) 多文件标签实时渲染（seed 里 task 0 有 notes.md + demo.py 两个文件）。
#     EB_TAB_CLICK 在截图前 1.5s 真实点击第一个 tab，验证切换后的 paint；
#     EB_TAB_SHOT 在审计的 tab 步骤处补一张离屏快照。
EB_ARTIFACT=1 EB_TAB_CLICK=1 EB_SHOT=/tmp/eb-live-switch.png \
EB_SHOT_AFTER_MS=3000 cargo run
EB_ARTIFACT=1 EB_CLICK_ASSERT=1 EB_TAB_SHOT=/tmp/eb-tabs.png cargo run

# 19) 筛选弹层（spec §5.4 实测样式）。EB_FILTER_SHOT 在审计点击漏斗按钮、
#     popover 弹出的瞬间做离屏快照：应看到 240 宽白卡、灰色分组标题、
#     选中行 #f7f7f7 药丸 + 加粗 + 行尾绿色对勾、灰色 Reset Filters。
EB_CLICK_ASSERT=1 EB_FILTER_SHOT=/tmp/eb-filter.png cargo run

# 20) 顶栏搜索药丸（spec §5.6 实测样式）。EB_SEARCH_SHOT 在审计点击放大镜、
#     药丸展开的瞬间做离屏快照：应看到 300x32 #f2f2f2 圆角 8 药丸（放大镜 +
#     光标 + 尾部 ^ v ×）、右侧 Share 与面板开关仍在；占位灰字在聚焦时不渲染
#     （GTK3 行为，§10-29）。
EB_CLICK_ASSERT=1 EB_SEARCH_SHOT=/tmp/eb-pill.png cargo run

# 21) 后台并发流（bug 修复回归）：任务 A 用 fake agent 流式执行中，250ms 后
#     另起任务 B；A 必须继续在自己的会话里流到完成，不能被标 Cancelled。
#     两条流都终态后打印 `two-task: finished` 并退出；任何回归退出码 1。
EB_TWO_TASK_TEST=1 EB_FAKE_AGENT=1 \
EB_FAKE_AGENT_DELAY_MS=250 EB_FAKE_AGENT_CHUNKS=4 cargo run

# 22) 侧栏在途任务光圈（spec §10-30）。流式中途截屏，应看到运行行行尾
#     时间戳被 16px 转圈替代；两任务并发时两个行各转各的。
EB_FAKE_AGENT=1 EB_FAKE_AGENT_DELAY_MS=300 EB_FAKE_AGENT_CHUNKS=8 \
EB_FAKE_DEMO=1 EB_SHOT=/tmp/eb-spin.png EB_SHOT_AFTER_MS=1000 cargo run
EB_TWO_TASK_TEST=1 EB_FAKE_AGENT=1 EB_FAKE_AGENT_DELAY_MS=300 \
EB_FAKE_AGENT_CHUNKS=8 EB_SHOT=/tmp/eb-spin2.png EB_SHOT_AFTER_MS=1200 cargo run

# 23) 偶发 hang 回归：用一个"接受连接但不回包"的黑洞监听占住桥端口，
#     连发两轮。桥调用必须各吃一次 socket 超时预算（≤1s）而不是永久卡住主线程；
#     第二轮还要看到降级冷却把桥调用直接跳过。20s 看门狗内没收尾就退出码 1。
EVOBUDDY_BRIDGE_ADDR=127.0.0.1:39577 EB_HANG_TEST=1 EB_FAKE_AGENT=1 cargo run

# 24) ACP 槽位探针：只跑 initialize + session/new，不发 prompt、不建窗口，
#     用来回答"这个 agent 到底会不会说 ACP"。四个槽位逐个探。
cargo build
for a in opencode grok jcode pi; do echo "== $a"; EB_ACP_PROBE=$a ./target/debug/evobuddy; echo "  rc=$?"; done

# 25) ACP「响应之后还在干活」端到端：把带 tail 场景的 fake agent 塞进 opencode
#     槽位，走真实宿主链路（不发 prompt 给真实模型，零 token）。
cat > /tmp/fake-acp-tail-wrapper.sh <<EOF
#!/bin/bash
export FAKE_ACP_SCENARIO=tail
exec python3 "$PWD/crates/plugins/eb-plugin-acp-agent/tests/fixtures/fake_acp.py" "\$@"
EOF
chmod +x /tmp/fake-acp-tail-wrapper.sh
EVOBUDDY_ACP_AGENT=opencode EVOBUDDY_OPENCODE_BIN=/tmp/fake-acp-tail-wrapper.sh \
EVOBUDDY_ACP_AUTO_APPROVE=1 EVOBUDDY_ACP_TAIL_QUIET_SECS=8 EVOBUDDY_ACP_DEBUG=1 \
EB_FAKE_DEMO=1 EB_SHOT=/tmp/eb-acp-tail.png EB_SHOT_AFTER_MS=5000 cargo run

# 26) 流式期间不抢滚动：慢速 fake 流跑到一半把消息列表往上甩 300px，之后的
#     delta 必须不再动视图；再把视图放回底部，后续 delta 必须继续跟随。
EB_FOLLOW_TEST=1 EB_FAKE_AGENT=1 EB_FAKE_AGENT_DELAY_MS=120 \
EB_FAKE_AGENT_CHUNKS=40 cargo run

# 27) 多任务并发流式时来回切换（spec §10-34/§10-35）。空状态：harness 自己起三个流式
#     任务；生产体量：先用 docs/make_heavy_state.py 造三个 20 轮的长会话，
#     每次跑之前复制一份（任务收尾会覆盖写状态文件）。
EB_SWITCH_TEST=40 EB_FAKE_AGENT=1 cargo run
mkdir -p /tmp/eb-heavy
python3 docs/make_heavy_state.py /tmp/eb-heavy/tasks.orig.json
cp /tmp/eb-heavy/tasks.orig.json /tmp/eb-heavy/run.json
EB_SWITCH_TEST=30 EB_FAKE_AGENT=1 EB_STATE=/tmp/eb-heavy/run.json EB_DEBUG=1 cargo run
#     出图核对 re-attach 后的渲染（用 EB_SWITCH_SHOT，不要用 EB_SHOT：见下）
EB_SWITCH_TEST=30 EB_FAKE_AGENT=1 EB_STATE=/tmp/eb-heavy/run.json \
EB_SWITCH_SHOT=/tmp/eb-after-switch.png cargo run
#     报障体量：一个任务 120 轮（3×120 共 1.4MB）。这档只有窗口化渲染后才过得了——
#     整段挂载时一次切换是 68ms 全树布局，harness 会报 main loop starved。
python3 docs/make_heavy_state.py /tmp/eb-heavy/many.orig.json 3 120
cp /tmp/eb-heavy/many.orig.json /tmp/eb-heavy/run-many.json
EB_SWITCH_TEST=30 EB_FAKE_AGENT=1 EB_STATE=/tmp/eb-heavy/run-many.json \
EB_SWITCH_SHOT=/tmp/eb-after-switch-many.png EB_DEBUG=1 cargo run

# 28) 长会话滚到顶自动加载更早（spec §10-35）。同一个 120 轮存档：harness 连做两次
#     "甩到顶边"，每次要求恰好多挂 10 轮、容器子控件数等于渲染状态自报的挂载数，
#     并且锚定帧相对视口顶边的位置漂移 ≤4px（第一次冷加载曾是 1580px，因为 reveal
#     之后上方内容还会再长高一次）。EB_LOAD_SHOT 在第二次加载后出图。
EB_LOAD_TEST=1 EB_STATE=/tmp/eb-heavy/run-many.json EB_DEBUG=1 cargo run
EB_LOAD_TEST=1 EB_STATE=/tmp/eb-heavy/run-many.json \
EB_LOAD_SHOT=/tmp/eb-loaded.png cargo run

#     同一张图也用来核对转录竖条的位置（spec §10-37）：条必须贴在 pane 右缘
#     （1440 宽时 `1425–1439`），消息列仍是 `484–1236`（中心与输入卡同轴），
#     条与列之间逐行全白——那条竖条若落在 1018–1032 或 1222–1236，就说明它又回到
#     了消息内 exec/code 卡自己竖条的位置，正是 §10-37 修掉的叠加。

# 29) 窗口缩放后的三栏重排（spec §10-36）。直接把 GdkWindow 拉小/拉大——不经过
#     GtkWindow 的 resize 路径，正是"configure 事件没被默认处理器吃掉"的那类缺陷
#     的复现方式。每个动作先拉到目标宽度、等 frame 落地，再等"pane 与消息列两次
#     读数一致"才测量（缩放是一次请求、布局要跟好几轮）。断言：GTK 缓存的 frame
#     宽 == 实际画的 surface 宽（不等就是"侧栏被拉满整窗"那份报障）、侧栏恒 280、
#     内容栈吃掉余量、消息列宽度为设计值；列宽超过 pane 只打 `note` 不判失败
#     （中文段落折断后的最小宽会粘在上一次的宽度，见 §10-36）。
EB_RESIZE_TEST=1200 cargo run
#     带存档 = 用户报障的那一步（切到某个任务之后再缩放）
EB_RESIZE_TEST=800 EB_RESIZE_TASK=1 EB_RESIZE_SHOT=/tmp/eb-resize cargo run
#     grow 半边要配长会话存档才有意义（`EB_LOAD_TEST` 用的那一份）：不带
#     `EB_STATE` 时 `EB_RESIZE_TASK` 走"自建三个短任务"的分支，转录没有可滚内容。
#     存档每次现造、现拷一份 scratch（harness 会覆盖 `EB_STATE` 指的那个文件）。
mkdir -p /tmp/eb-rz && python3 docs/make_heavy_state.py /tmp/eb-rz/tasks.orig.json 2 12 \
&& cp /tmp/eb-rz/tasks.orig.json /tmp/eb-rz/tasks.json
HOME=/tmp/eb-home-XX EB_STATE=/tmp/eb-rz/tasks.json EB_FAKE_AGENT=1 \
EB_RESIZE_TEST=1440 EB_RESIZE_TASK=1 ./target/debug/evobuddy
#     ⇒ shrink width=1440 … stack=1159 column=752 / grow width=1910 … stack=1629
#     column=752。**harness 机制（spec §10-38）**：`surface.resize()` 只让 GTK *知道*
#     新 frame，把 frame 变成新分配的是 `gtk_window_process_updates`，而它挂在 GDK
#     frame clock 上 —— `cargo run` 的进程不是活动应用，clock 不 tick，树会一直停在
#     旧分配上（假失败：`stack` 死在 1159）。所以每个 tick 比较 `window.size()` 与
#     `window.allocation()`，不一致就自己 `size_allocate()` 一次，等价于真实拖拽的下
#     一帧。`gtk::WindowExt::process_updates` 在 gtk-rs 0.19 / gtk-sys 里没有绑定。
#     走过的弯路别再照抄：`window.resize()`（macOS 干脆不回 configure）、tick 里 pump
#     `MainContext`（没有 pending 源）、只 measure 一次 window（碰运气，时序不稳）。

# 30) 空白处也能滚转录（spec §10-38）。列只有 752 宽、scroller 的输入窗口就到列边，
#     pane 两侧各约 204/189px 空白原本没人接滚轮；输入卡那一段左右两条空白同理。
#     harness 挂存档 → 打开任务 0 → **先断言两个 catcher 的 GdkWindow 掩码**（缺
#     SCROLL/SMOOTH_SCROLL 就直接红，见下）→ 在铺满 pane 的 catcher 上送 -400 上滚、
#     +250 下滚，再在铺满整页的 `page_hits` 上送 -300，每步断言转录那条 `vadjustment`
#     的位移（±8px），中途还断言条已带上 `eb-bar-active`（配方 31）。
mkdir -p /tmp/eb-pad && python3 docs/make_heavy_state.py /tmp/eb-pad/tasks.orig.json 2 12 \
&& cp /tmp/eb-pad/tasks.orig.json /tmp/eb-pad/tasks.json
HOME=/tmp/eb-home-XX EB_STATE=/tmp/eb-pad/tasks.json EB_FAKE_AGENT=1 \
EB_PAD_SCROLL=1 ./target/debug/evobuddy
#     ⇒ 三格位移必须精确：实测 `pad-scroll: ok the pane scrolls 18376 -> 17976 ->
#     18226 -> 17926 of 18980`；同一份存档另一次跑是 `21536 -> 21136 -> 21386 …`，
#     绝对 `upper` 随惰性 attach 当刻挂了多少轮而变，harness 只断言位移。
#     必须配长存档：没有可滚内容时 harness 直接 FAIL（不会退化成空过）——
#     读到 604/604 这类"没东西可滚"就是存档被上一轮写坏了，重新 `make_heavy_state.py`。
#     两条负向对照（都实测过，不是推测）：
#       · 把 `conversation.rs` 里 catcher 的 `connect_scroll_event` 转发换成 `let _ =`
#         ⇒ `pad-scroll: FAIL … moved the transcript 0px, not -400px`；
#       · 把两处 `add_events(SCROLL_EVENTS)` 注释掉 ⇒ `FAIL the pane catcher's window
#         selects EventMask(EXPOSURE | BUTTON_* | ENTER/LEAVE), which is missing
#         SCROLL_MASK/SMOOTH_SCROLL_MASK`。**这条是必需的**：`propagate_event` 绕过
#         GDK 的窗口掩码过滤，光测位移的 harness 会在掩码缺失时照样全绿，而真机滚不动
#         —— 就是这么漏过一次，用户复测"margin 还是滚不动"才抓到。

# 31) 右缘转录条是细滑块，不是灰带（spec §10-39）。静止态实测只有 3 列非白。
#     用配方 28 那份 >40 轮的存档（`make_heavy_state.py … 3 24`），2×12 那份不够长，
#     `EB_LOAD_TEST` 会先 FAIL。
HOME=/tmp/eb-home-XX EB_STATE=/tmp/eb-heavy/run.json EB_FAKE_AGENT=1 \
EB_LOAD_TEST=1 EB_LOAD_SHOT=/tmp/eb-pad/bar.png ./target/debug/evobuddy
python3 - <<'PY'
from PIL import Image
im = Image.open('/tmp/eb-pad/bar.png').convert('RGB'); w, h = im.size
row = [x for x in range(w - 20, w) if im.getpixel((x, 260)) != (255, 255, 255)]
print('non-white columns at y=260:', row, [im.getpixel((x, 260)) for x in row])
PY
#     ⇒ `[1433, 1434, 1435]`、值 `(222,222,222)`（= `alpha(#000,.13)` 落在白底上）。
#     回归成 `1426–1439` 整列 `#cecece` 就是槽又实心化了。滑块宽度/灰度若读成 10px 宽、
#     值 127，说明 CSS 又用回了 `> trough > slider` —— GTK 在 `scrollbar` 与 `trough`
#     之间夹了节点，那条写法只匹配得到控件自身，主题色就会盖回来（配方 30 的
#     `eb-bar-active` 断言红也指向同一处）。
```

> **状态文件只在没有任何 selftest 旗标时才加载**（`selftest::requested()` 里列了
> `EB_SHOT`/`EB_ASSERT`/`EB_CLICK_ASSERT`/`EB_DEMO`…）。所以"配一份长会话存档
> 再切"的 harness 不能同时带 `EB_SHOT`——存档不会读进来，harness 会以为状态是空的、
> 自己新建三个短任务。配方 27 的截图因此走 `EB_SWITCH_SHOT`（由 harness 自己在最后
> 一次切换后 300ms 调 `selftest::snapshot`）。同理，跑任何 harness 前请确认临时
> `HOME` 下的 `~/.evobuddy/tasks.json` 是不是上一轮留下的存档：它会改变任务索引，
> 让 harness 走"有存档"的分支。

期望：

- `cargo test --workspace`：`eb-api` 12 个、`eb-plugin-demo-agent` 5 个、
  `eb-plugin-acp-agent` 10 个、`evobuddy` 39 个测试通过，其余 crate 0 测试。
- `EB_ASSERT=1`：最后一行 `selftest: assertions passed`，退出码 0；屏幕小于
  1440×900 时只断言语义不变量（窗口钳到工作区是正常的）。
- `EB_CLICK_ASSERT=1`：最后一行 `click: assertions passed (33 steps)`，退出码 0。
  审计步 `model selector menu` 现在要求 `7 of 7 menu rows visible`——菜单是
  Default + 4 个 ACP 槽位（opencode/Grok Build/jcode/pi）+ dsh + Custom Model，
  加槽位必须同步改 `router::ACP_MENU` 和 `selftest.rs` 里的那句期望。
- 截图后可用 `node docs/ui-spec/pngpx.js /tmp/evobuddy-chat.png` 查看像素；
  侧栏 `#f7f7f7`、路由 `#ffffff` 是规格值。
- `EB_FAKE_DEMO`：应打印 `fake-demo: finished`；取消/stop 那两次还要能看到
  `Failed { message: "Cancelled" }`；panic 那次应看到
  `agent: start failed: the agent panicked while starting`。
- `EB_ENTER=1`：应打印 `enter: before=welcome` 和 `enter: after=conversation`。
- `EB_ARTIFACT=1`：面板恒占 580（1440 基线 `sash artifact x=860`、
  `eb-detail 579x836`）；`EB_WINDOW_WIDTH=1920` 时若被窗口管理器钳到
  1470，则 `x=890`、detail 仍是 579 宽——面板不吃对话列的余量。
- 窄窗口保底（spec §10-31）：`EB_WINDOW_WIDTH=900 … EB_DEBUG=1` 打印
  `layout: width=1082 … pos=750 detail_min=332`，`sash artifact x=750`、
  `eb-detail 332x800`，`EB_ASSERT=1` 仍 `assertions passed`。截图
  `/tmp/eb-narrow.png` 要看两点：① 用户气泡**完全落在会话列内**，不越过分隔条
  压进面板（消息列取 `min(MESSAGE_WIDTH, 会话列宽)`=470，因此正好贴满会话列；
  居中由 scroller 的 `halign` + 内容宽完成，列自身不带 margin，见 spec §10-36）；
  ② 面板头部标签条与 ×/+/◐/⤢ 仍可见可点，顶栏 ⇧⌘B 收起面板后会话列回到 752。
- `EB_RESIZE_TEST`（配方 29，spec §10-36）：应打印两行 `resize-test: shrink …` /
  `resize-test: grow …`、退出码 0。每行都要 `toplevel == surface`（不等就是那份
  "侧栏拉满整窗"的报障）、`sidebar=280`、`stack` 吃掉余量。2026-10-27 实测
  1200→1670：`column=752` 双向一致；800→1270（带 `EB_RESIZE_TASK=1` 的存档）：
  `toplevel=800 surface=800 stack=534 column=751`，并附一行
  `resize-test: note shrink: …`（列比 pane 宽，是中文段落折断粘住的已知残留，
  不是失败）。回归验证方式：把 `shell.rs` 的 configure 处理器改回 `true`（吞掉
  事件），同一 harness 以 1 退出并报 `FAIL grow: the content stack got 1159px of a
  1670px window`——控件树停在旧 frame 的布局上；用户看到的那一档更极端，服务器把
  280px 的旧 surface 铺满整个新 frame，于是"只剩侧栏"。
- 多文件标签：`EB_TAB_CLICK=1` 的 `EB_SHOT` 截图应显示切到 `notes.md` 后的
  真实渲染（带行号 gutter 的正文）；`EB_TAB_SHOT` 是审计步骤处的离屏快照，
  标签条正确但代码体可能空白（离屏捕获的已知伪影，以实时截图为准）。
- Artifact 文件卡（spec §5.5）：审计 `artifact card` 步在种子下打印
  `cards=2` 且 `file="notes.md"`（点第一张卡把面板从最新的 `demo.py` 切回
  `notes.md`）；无种子时面板保持 `tabs=1`。截图验证：
  `EB_ARTIFACT=1 EB_SHOT=/tmp/eb-cards.png EB_SHOT_AFTER_MS=2500 cargo run`
  应看到完成回答下两张灰底圆角卡（蓝色徽章 + 文件名 + 字节数）；超过 2 个
  文件时才有 `View all artifacts (N) ›` 链接。
- 顶栏三件套（spec §5.6）：默认态截图右端应看到 放大镜 / 弯曲分享箭头 /
  圆角矩形+实心右导轨 三枚 32px 按钮（4px 间距）；`EB_SEARCH_SHOT` 离屏快照
  应看到药丸展开（`^` `v` `×` 三枚 24px 步进、Share/面板开关右移跟随）；
  审计 `chat search`→`search bar visible=true`、`chat search close`→`false`、
  `panel toggle (close)`→`detail visible=false`。
- `EB_SEND_AFTER_MS`：在 `build_ui` 返回后模拟发送，应出现 `submit:`、
  `runner: refresh_now` 和 `refresh: begin tasks=1`；这是 Views 生命周期回归。
- `EB_CHAT_SEND_AFTER_MS`：在对话里再发一条，`Tasks (1)` 必须保持不变；
  只有欢迎页/New Task 的发送才新增任务。
- `EB_TWO_TASK_TEST=1`（配 `EB_FAKE_AGENT`）：应打印 `two-task: finished`、
  退出码 0；两个任务各自的最后一轮都是非流式、无 `Cancelled/Failed`、
  有正文块。若第二个提交把第一个任务打断，会打印 `two-task: FAIL …` 并以 1 退出。
- 侧栏光圈：配方 22 的两张截屏里，流式中的任务行行尾应看到 16px 灰弧
  （时间戳被替代），`...` 菜单钮仍在最右；两任务并发时各自行都带弧；
  任务终态后的刷新里弧消失、时间戳回来（离屏快照抓到的是时钟相位的任意角）。
- `EB_HANG_TEST=1`（配方 23，配 `EB_FAKE_AGENT`）：应打印
  `hang-test: recovered first=… second=…`、退出码 0。`first` 是"桥只超时一次"
  的那轮（约 1s 桥预算 + 流式时长，断言 <5s），`second` 必须比 `first` 小且
  <3s——降级冷却生效的证据。若把 `bridge.rs` 的 socket 超时去掉，20s 看门狗会
  打印 `hang-test: FAIL the app never recovered …` 并以 1 退出。端口被真实桥占住
  时换 `EVOBUDDY_BRIDGE_ADDR` 到空闲端口即可，不需要停掉真实桥。
- ACP 探针（配方 24）：每个槽位先打印 `acp-probe: <id> -> <program> <args>`
  （pi 那行应是 `npx --yes pi-acp@0.0.34`，除非 `EVOBUDDY_PI_ACP_BIN` 或全局
  `pi-acp` 存在），成功再打印 `ok name=…` 与 `capabilities=…`、退出码 0；
  agent 拒绝或可执行文件缺失是 1，槽位名打错是 2 并列出可选 id。2026-10-26 本机
  四个槽位（配好登录的真实环境）都拿到 rc 0；未登录时 grok 会以 1 退出并报
  `no auth method id provided`——**探针必须用真实 `HOME` 跑**，grok/opencode 的登录态
  都在 `~/.grok`、`~/.opencode` 下，套上临时 `HOME` 的 harness 前缀会让它们"看起来
  没登录"。capability 明细见 [`docs/acp-agent.md`](acp-agent.md)
  的探针表。**探针不发 prompt，不花 token。**
- ACP 尾流（配方 25）：调试日志按顺序出现 `acp: prompt response`、
  `acp: agent kept working after the response`，随后是响应之后那条
  `session/request_permission` 被宿主 answered（`EVOBUDDY_ACP_AUTO_APPROVE=1`
  或显式批准都行）。截图 `/tmp/eb-acp-tail.png` 里状态行是
  `Agent is still working`；`EVOBUDDY_ACP_TAIL_QUIET_SECS=8` +
  `EB_SHOT_AFTER_MS=5000` 的延时截图应看到正文
  `research: part one. research: part two. research: part three.` 追加在**同一条**回答里
  （不是新一轮），最后一行仍是 `fake-demo: finished`、退出码 0。把 `prompt()` 里那句
  `drain_tail` 去掉，正文只剩响应前的部分——这就是「没完成任务就结束」的回归。
  把 wrapper 里改成 `export FAKE_ACP_SCENARIO=retry` 再跑一次：这一轮不会再有续流，
  2s 探测窗后正常收尾（`fake-demo: finished`、退出码 0）。两条 `Retrying 1/15`
  （Grok 的私有 `_x.ai/session_notification` 信封）与 `Retrying 2/15`（spec 信封）
  只在退避期间挂在状态行上，响应之后就被后续状态顶掉，所以**别拿截图当断言**——
  双信封的可见性由单测 `provider_retries_reach_the_status_line` 把关。
- 滚动跟随（配方 26）：应打印 `follow-test: finished`、退出码 0。它断言三件事：
  滚上去之后 `pinned=false` 且离底 ≥100px；观察窗内**确实有 delta 到达**（正文字符数
  增长）而视图 `value` 一动没动（>1px 即 FAIL）；把视图放回底部后一轮检查时仍贴底
  （>24px 即 FAIL）。把 `follow_output()` 的 `!self.pinned.get()` 判定临时旁路再跑同一
  配方，就会复现用户报的原症状：`follow-test: FAIL a delta yanked the view +348px while
  89 chars streamed in`（2026-10-27 实测）。harness 自己发一条足够长的 prompt 把转录
  撑高（否则视口没有 300px 可滚，它会退化成一个跟状态存档内容挂钩的偶发测试），
  并按 `painted_task` 读正在流式的那个任务。
- 任务切换不卡（配方 27，spec §10-34/§10-35）：应打印
  `switch-test: finished switches=40 worst_late=0ms worst_switch=0ms tree=… frames=2 window=(0, 2)`、
  退出码 0。三条不变量：消息列表的帧数**恰好等于**渲染状态自报的挂载数、且窗口不越过
  所画任务的轮数边界（切换漏帧会每轮变大）、每次切换都在 120ms 节拍内
  （`worst_late`/`worst_switch` 任一 >600ms 即 FAIL）、最后 re-attach 的帧全部可见且有非零分配。
  挂上长会话存档跑生产体量：`EB_DEBUG=1` 会打印 `fill_messages: re-attached N frames for task i`
  （2026-10-27 实测 30 次切换里 29 次走 re-attach，只有冷缓存的第一次全量重排；`conv` 均值 8.5ms、峰值
  41.6ms，修复前每次切换都全量重排、均值 23.3ms）。
  修复前的同一配方报 `switch-test: FAIL the main loop starved by 1233ms during 40 switches`，
  且迟到量随切换次数单调增长。截图 `/tmp/eb-after-switch.png` 人工核对：切换后的会话
  应有完整 markdown 正文、exec/diff 卡与每轮工具条（复制/点赞/点踩/重新生成），
  没有空白轮次或重复帧。
  120 轮那档（`run-many.json`）是报障体量：应打印 `window=(100, 22)`（122 轮里只挂 22 帧）、
  `tree=1087->1139`、`worst_late=0ms worst_switch=0ms`。整段挂载时它是
  `switch-test: FAIL the main loop starved by 3334ms during 20 switches`——一次切换 68ms 的
  全树布局（6289 个控件），刷新内外合计约 150ms。
- 滚到顶加载更早（配方 28，spec §10-35）：应打印
  `load-test: finished task=0 window=90+30->80+40 drift=0px,0px`、退出码 0。两次"甩到顶"
  各要求：窗口下界恰好前移 `ATTACH_STEP=10`、新增帧数等于前移轮数、容器子控件数等于挂载数、
  锚定帧相对视口顶边的位置漂移 ≤4px。`reanchor: pass N …px -> …px` 是补偿在事后修正的现场
  （把那句 `connect_changed` 去掉，第一次冷加载会留下 1580px 漂移：reveal 那次协商用的还是
  Pango 的临时测量，上方内容之后又长高一截）。`/tmp/eb-loaded.png` 人工核对：加载出来的
  较早轮次（截图里是 question 45 起）正文、工具行与状态都画全，且读者停在**自己刚才那一行**。
  注意 harness 不能用 `upper` 的增量当不变量，也不能用 `translate_coordinates`（它不随滚动值
  移动）；有效的度量是 `frame.allocation().y - adjustment.value()`，即"视口顶边下那个内容像素"。

CI 里的同一条路径：Linux 跑
`packaging/linux/verify.sh`（裸 Debian 10 + `EB_ASSERT` + `EB_CLICK_ASSERT` +
fake streaming/cancel），macOS 跑 `packaging/macos/verify.sh`（藏 Homebrew +
两个断言 + fake streaming/cancel）。

## 3. 手工 UI 检查（重点看插件贡献）

| 区域 | 操作 | 期望 | 来自 |
|---|---|---|---|
| 侧栏 | 看五行导航 | `New Task`、`Assistant BETA`、`Experts · Skills · Connectors`、`Scheduled`、`Library` | nav 插件 |
| 侧栏 | 点 `Assistant` / `Experts…` / `Scheduled` / `Library` | 进入 “not in this build” 路由；点 `New Task` 回欢迎页 | 宿主按 nav 的 `unbuilt_route` 渲染 |
| 欢迎页 | 看六个 chip | `Development`、`Websites`、`Mini Programs`、`Agents`、`Skills`、`Tech Docs` | composer 插件 |
| 欢迎页 | 逐个点 chip | 输入框被填入对应的整句 prompt（不是发送） | composer 插件 |
| 欢迎页 | 点 `Select Workspace` | 菜单 `No workspace` / `Add workspace…` | composer 插件 |
| 欢迎页 | 点 `Default Permissions` | `Default Permissions` / `Auto-approve` / `Read-only` | composer 插件 |
| 欢迎页 | 点 `Auto` | `Auto` / `Fast` / `Deep` | composer 插件 |
| 欢迎页 | 点 `+` | `Add file` / `Add folder` / `Add screenshot` / `Add web page` | composer 插件 |
| 侧栏 | 点搜索 / 小程序 / 筛选 | 搜索与小程序弹 notice；筛选卡 11 行（6 状态 + 4 时间 + Reset），选中行灰底加粗 + 行尾绿勾（spec §5.4） | notices / composer |
| 侧栏 | 任务流式执行中看任务行（含工作区嵌套行） | 行尾时间戳位置出现 16px 转动灰圈，终态后变回时间戳；多任务并发各转各的（spec §10-30） | 宿主 `widgets::spinner` |
| 侧栏 | 折叠 / 展开、Tasks 分区折叠 | 可见性与 `▶` 字形切换；任务行隐藏/恢复 | 宿主交互 |
| 会话 | 发送（按钮或 Enter） | 进入会话；第一条 demo 回答含文本 + 工具行 + 表格，并打开 artifact | demo-agent |
| 会话 | 再发送一次（当前实现会新建任务） | 第二条回答是另一种占位文案，且新任务没有 artifact，面板关闭（demo-agent 交替） | demo-agent |
| 会话 | `EB_FAKE_AGENT=1 cargo run` 后点 chip / Enter | 每 120ms 到达一个 `chunk N`，底部显示 streaming footer；流式期间侧栏仍可折叠/切换 | fake agent |
| 会话 | 点 `✎`（Code assist） | `Code assist is not part of this UI-only build.` | notices 插件 |
| 会话 | 顶栏搜索 | 原地展开 300×32 药丸（标题仍可见）；输入即跳转命中轮，`^` `v` 前后步进，`×`/Esc 收起；⌘F 等价展开 |
| 会话 | 顶栏分享 | notice popover |
| 会话 | 顶栏面板开关 | 开→关 / 关→开右面板；tooltip 随状态在 `Open/Close right panel ⇧⌘B` 间切换；⇧⌘B 等价点击；无文件任务上点击无操作 |
| 会话 | 消息复制 | tooltip/反馈出现 `Copied` | 宿主 |
| 会话 | 侧栏用户行 | `Settings` / `Keyboard shortcuts` / `Send feedback` / `Sign out` | composer 插件 |
| Artifact | 展开 / 恢复 / 关闭 / 任务行重开 | 面板宽度与可见性变化；重开恢复面板 | 宿主 |
| Artifact | `×` 关闭标签、`+` 新建标签 | 多文件时 `×` 只关那一个标签（关到空才收起面板）；`+` 弹 notice | 宿主 / notices |
| 会话 | 点回答下的文件卡 / `View all artifacts` | 面板展开并选中该文件的标签；View all 选最新文件；流式期间无卡片 | 宿主 |
| Connectors | 在 Experts · Skills · Connectors 里点某张卡的 `Test` | 卡片立刻显示 `testing…` 且窗口照常重绘（握手最长 10s，跑在工作线程），结果回来后才替换成 `MCP initialize ok` / `failed: …` | 宿主 |

注意：`Code assist` 与 workspace/permission 菜单**不在** 33 步自动审计里，手工
检查时重点补这几个。

## 4. 插件边界与运行时验证

### 4.1 依赖边界（不会启动 UI）

```bash
cargo tree -p eb-plugin-nav --edges normal
cargo tree -p eb-plugin-composer --edges normal
cargo tree -p eb-plugin-demo-agent --edges normal
cargo tree -p eb-plugin-acp-agent --edges normal
```

每个插件应只有 `eb-api`（再往下是 vendored `rutis`/tokio）；ACP 插件额外有
纯数据依赖 `serde_json`，这是协议允许的例外。任何插件都**不应出现 `evobuddy`
或 `gtk`**。宿主 `evobuddy` 的依赖里才会同时出现五个插件和 `gtk`。

```bash
cargo metadata --no-deps --format-version 1 |
  python3 -c 'import json,sys; d=json.load(sys.stdin); [print(p["name"], "->", sorted(x["name"] for x in p["dependencies"])) for p in d["packages"] if p["name"].startswith("eb-") or p["name"]=="evobuddy"]'
```

### 4.2 插件不碰 widget

```bash
grep -RIn "gtk" crates/plugins || echo "no gtk references in crates/plugins"
```

应没有任何输出。插件产出的都是 `eb-api` 的纯数据（nav 行、菜单项、notice 文案、
chip、`AnswerSource`）。

### 4.3 贡献缺失会启动失败（可选）

`crates/eb/src/host.rs` 的 `validate()` 会检查 nav、6 个菜单、4 个 notice、
composer copy、chips 和 answer source。想验证这一点，可以临时把
`eb-plugin-notices` 从 `Host::mount()` 的内置插件列表里去掉，或改掉某个
notice 的 id，启动时应直接 panic 并报出缺失的贡献；测试完用
`git checkout -- crates/eb/src/host.rs`（以及对应插件文件）恢复。
**不要在有未提交改动时直接 `git checkout` 整个目录。**

### 4.4 改一条贡献看 UI（可选）

在 `crates/plugins/eb-plugin-nav/src/lib.rs` 临时加一行 `NavRow`，重新
`cargo run`，侧栏应多一行；或在 `eb-plugin-composer` 改一个 chip 的 `prompt`，
点击后输入框内容应跟着变。这能直接证明“插件产出数据、宿主渲染”的边界。
改完用 `git diff` 确认，再按需恢复。

## 5. 自动化审计没有覆盖的部分

以下仍需手工或真实环境：

- `Code assist`、workspace / permission 菜单（见上表）。
- 真实活跃窗口下的 popover 焦点、点击外部关闭（无窗管环境只能看到 inactive）。
- IME 组合态下的 Enter（需要真实输入法）。
- Linux 自带栈的 ibus/fcitx5 输入（缺口 22）。
- 超长 artifact 标签省略、overlay 滚动条观感。
- Windows 产物（无验证环境，仍标“未验证”）。

### M2.0a/b 当前限制

- UI stop 已可用；`EB_FAKE_STOP_AFTER_MS` 点击的就是用户点的那颗按钮。
- `Usage` 事件已定义但模型还没有用量环，暂时忽略。
- `AgentSink` 是容量 64 的有界队列；`AgentRuntime` 已有专用线程，
  fake agent 自己起 worker 线程模拟异步生产者。
- 每个任务各自支持一个在途回答；同一任务里的新提交会把旧回答标成 `Cancelled`。
  不同任务互不打断：另起新任务或在别的会话发言时，后台任务的流继续写入自己
  （`EB_TWO_TASK_TEST` 回归）。注意 ACP 的真实 agent 共享一条子进程 session，
  两个任务同时提问时第二个会等第一个的 prompt 结束（进程内串行，属正常）。
- `Contribs` 仍是挂载期快照，运行期贡献热更新未做。

## 6. 常见问题

- **`cargo run` 报 GTK 找不到**：确认 `pkg-config --modversion gtk+-3.0` 有输出；
  macOS 设 `PKG_CONFIG_PATH`，Linux 装 `libgtk-3-dev`。
- **命令一闪而过**：带 `EB_*` 标志时应用在报告后自动退出，这是设计行为；想交互
  就裸跑 `cargo run`。
- **点击审计 `window active=false`**：终端启动没有窗管激活窗口，审计会尝试
  `present()`；这不影响断言结果。
- **macOS 自包含 app 打不开**：未签名，右键 Open 或清 quarantine；不要用
  `open` 看日志，直接跑 `Contents/MacOS/evobuddy`。
- **AppImage 执行报 `Exec format error`**：当前在模拟层（qemu/Rosetta），
  AppImage runtime 不能运行；换原生架构。
