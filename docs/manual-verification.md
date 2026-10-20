# 人工验证清单（ACP / 多屏 / 输入）

这份清单只列自动化不容易覆盖、但几分钟能确认的项目。按顺序做，失败时保留终端日志。

## 1. ACP 文本对话（opencode）

```bash
pkill -f 'target/debug/evobuddy' 2>/dev/null || true
EVOBUDDY_ACP_COMMAND="$HOME/.opencode/bin/opencode" \
EVOBUDDY_ACP_ARGS=acp \
EVOBUDDY_ACP_CWD="$PWD" \
EVOBUDDY_ACP_DEBUG=1 EB_DEBUG=1 cargo run
```

- [ ] 窗口出现在**笔记本主屏**（不是上方的外接屏）。
- [ ] 输入一句 `hi`，点发送或 Enter；先看到自己的消息和 `Starting agent…`/`Thinking`，随后出现 opencode 文本。
- [ ] 终端有 `runner: refresh_now` 且紧跟 `refresh: enter` / `refresh: begin tasks=1`。
- [ ] 得到回答后发送按钮从黑色 stop 恢复为箭头。

## 2. ACP 工具权限（opencode）

发送一个会读文件/跑命令的请求，例如“读一下 Cargo.toml 的第一行”。

- [ ] 输入框上方出现 `Tool permission` 弹层，按钮来自 agent（通常是 Allow / Reject）。
- [ ] 点 **Allow**：agent 继续，出现 tool/exec 行。
- [ ] 再发一次，点 **Reject**：工具被取消，对话继续出现拒绝/错误说明。
- [ ] 不点弹层、按 stop：不应崩溃；重新发送后仍可用。

无人值守复现（不点弹层）：

```bash
# 自动点 Allow / Reject
EB_PERMISSION_CLICK=allow ...
# 或直接按策略回答
EB_PERMISSION_AUTO=allow ...
```

## 3. dsh ACP

```bash
EVOBUDDY_ACP_COMMAND="$DSH/node_modules/.bin/pnpm" \
EVOBUDDY_ACP_ARGS="dsh --profile acp" \
EVOBUDDY_ACP_CWD="$DSH" \
DEEPSEEK_API_KEY="…" \
EVOBUDDY_ACP_DEBUG=1 cargo run
```

- [ ] 有 `DEEPSEEK_API_KEY` 时出现 dsh 文本；没有时窗口出现可换行的 `Failed: ... no API key ...`，而不是撑宽窗口。
- [ ] dsh 的 stderr 只在 `EVOBUDDY_ACP_DEBUG=1` 时出现。

## 4. 多显示器 / 窗口位置

- [ ] 启动后窗口在主屏可见；如果仍在别的屏，终端 `monitor` 行会打印 GDK 的显示器布局。
- [ ] 拖动窗口到另一块屏后，下一次启动仍回到主屏原点（当前策略）。

## 5. 中文输入（macOS 本机）

- [ ] 点击输入框，用中文输入法输入一句中文，能正常组合上屏。
- [ ] 组合态按 Enter 不会误发送；确认上屏后再 Enter 才发送。
- [ ] `IMKCFRunLoopWakeUpReliable` / `TSM AdjustCapsLockLED` 只是 macOS 输入法日志，不必然是故障。

## 6. 失败时收集

```bash
# 只保留关键行
... 2>&1 | tee /tmp/evobuddy.log
grep -E 'submit:|runner:|refresh:|acp:|permission:|agent: task' /tmp/evobuddy.log
```

把 `submit:` 之后的日志和窗口截图一起提供；现在 `refresh: begin tasks=1` 是判断 UI 是否真正重绘的关键行。
