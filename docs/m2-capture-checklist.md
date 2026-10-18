# M2.1 产品采集清单

> 目的:为 M2.2 的工具调用展开卡、diff、exec、表格、status chips 与用量环取得
> **可复现的几何/样式证据**,而不是从类名猜。采集只读,产物只进临时目录;仓库里
> 只留数值、结构与**自有**占位文案。

## 1. 环境与版本

采集前先记录:

- 产品版本 / 构建号、采集日期、操作系统与桌面缩放。
- 窗口 `office` 的客户区尺寸(目标 1440×868)与 DPR(规格书为 2)。
- 采集方式:`docs/ui-spec/capture_dom.js` / `watch_dom.js` 的只读 CDP,端口
  9333;不修改产品安装目录。
- 账号/工作区只用于触发真实工具调用;不记录凭据、token、个人路径或对话内容。

## 2. 触发一次真实工具调用

需要一条能产生以下内容的会话(至少各一次):

1. 一次写入/编辑文件 → `cr-tool-write__*`、`cr-tool-diff`。
2. 一次命令执行 → `cr-tool-exec__title` / `__command` / `__output`。
3. 一次表格结果 → `cr-table-block`。
4. 一次带状态/上下文的回答 → `cr-status-chips`、`cr-context-usage`。
5. 一次可折叠的调用组 → `cr-collapse` 的 collapsed / expanded / completed /
   summary 四态。

如果产品当前版本无法稳定触发某一类,记录“未采集”,不要用推测值补齐。

## 3. 每个块要采集什么

对每一类、每一种状态:

| 项 | 内容 |
|---|---|
| DOM | 该块的最小子树(outerHTML 或关键节点 + class),去掉无关祖先 |
| 几何 | 外层/头部/正文/滚动的 `getBoundingClientRect`,CSS px |
| 计算样式 | 背景、文字色、边框、圆角、padding/margin、gap、字号/行高、字体族 |
| 截图 | 该块裁剪图,DPR 2;展开/折叠各一张 |
| 内容 | 行数/列数、最长行、是否滚动、是否换行;**用自有占位文本记录** |
| 状态 | 流式中 / 完成 / 失败 / 折叠 / 展开 / hover(如适用) |

### 3.1 展开卡 `cr-collapse`

- 四态:`--collapsed`、展开、`--completed`、`--summary`。
- 头部高度、箭头/图标尺寸、标题与副标题的基线、`#e7f9ef` 背景的实际取值。
- 展开后内容区的 padding、与相邻消息的间距。
- 折叠时是否保留 summary 行、summary 的字号/颜色。

### 3.2 diff `cr-tool-diff`

- `__line`、`--add` / `--del`(产品类名可能是 `__line--add`)、`__sign` 的颜色。
- 行高、gutter 宽、`+42/−0` 数字的位置与字体。
- `--all-add` / `--first-add` / `--last-add` 的圆角与分隔线。
- 长行是横向滚动还是换行;滚动区 `hadjustment.upper/page_size`。

### 3.3 命令输出 `cr-tool-exec`

- 标题、命令行、输出区三段的层级与间距。
- 输出区最大高度、滚动策略、等宽字体与行高。
- 多行/超长行/空输出三种情况。

### 3.4 表格 `cr-table-block`

- 外层圆角、表头背景、分隔线、单元格 padding。
- 列宽策略(内容宽/均分/固定),长单元格是否省略。
- `__scroller` 的横向滚动阈值与 `upper/page_size`。

### 3.5 状态 chips `cr-status-chips`

- chip 高度、圆角、间距、图标尺寸、颜色(成功/运行/失败)。
- 多 chip 换行规则。

### 3.6 用量环 `cr-context-usage`

- 环直径、线宽、track/progress 颜色、中心文字。
- 0% / 50% / 100% 三个值,以及 `--clickable` 的 hover 态。
- 旁边是否显示数字/百分比。

## 4. 产物与落库

临时目录(不入仓库):

```
/tmp/evobuddy-capture/
  meta.json              # 版本、日期、窗口、DPR
  dom/                   # 每块的最小 DOM
  styles/                # 每块的 computed-style 子集
  shots/                 # DPR2 裁剪图
  notes.md               # 未采集项与原因
```

仓库里只提交:

- `docs/ui-spec/ui-spec.md` 的 §5(组件)/§4(几何)新增表格:数值、类名、状态。
- `docs/ui-spec/fixtures/blocks/*.json`:**自有占位文本**的块数据,供渲染器与
  `EB_SELFTEST`/截图使用;不包含产品文案或个人内容。

## 5. 验收

- 每类块至少一个展开态、一个折叠态;diff/exec/table 各有一个长内容用例。
- spec 里能查到:外层尺寸、内边距、行高、颜色、滚动阈值。
- fixture 能驱动 `ConversationBlock` 渲染,并在 Linux/macOS 截图;
  `EB_ASSERT`/`EB_CLICK_ASSERT` 覆盖新增块。
- 未采集的部分明确写“未验证”,不阻塞 M2.2 的其他块。

## 6. 如果环境不可用

不要用产品 CSS/类名硬猜数值。按优先级:

1. 先做不依赖新采集的块(已有的 text/tool/table)。
2. 把 diff/exec/chips/usage 标成“未验证”,用自有占位内容实现近似布局。
3. 等真实环境可用时补齐 spec 与 fixture,再逐块收紧断言。

法务边界仍按 spec §9:只记录可观察的数值/结构,不复制产品 CSS/JS、品牌资产或
文案。
