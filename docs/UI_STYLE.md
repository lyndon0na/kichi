# UI_STYLE.md — Kichi 界面风格规范（KDE / Breeze）

> 本文件是界面外观与交互的**单一规范**。新页面 / 新控件 / 改动既有界面时先读这里；
> 与代码实况冲突时以代码为准，并顺手修正本文件。工程约束见 `AGENTS.md` 第 8 条。

## 1. 总原则

Kichi 是 Linux 桌面客户端，外观目标是与 KDE Plasma / Breeze（Dolphin、System Settings）**融为一体的原生观感**：

1. **扁平、克制**：不堆装饰。控件与菜单项**常态无边框**，不要「框里套框」；只有悬停 / 选中 / 聚焦才出现反馈。
2. **信息密度高**：间距紧凑，列表行矮、留白少，一屏信息量优先于「呼吸感」。
3. **层次靠底色不靠描边**：用 `bg / panel / card` 三档明度差表达层次，而不是到处画 border。
4. **跟随系统**：能跟随 KDE 系统配色的地方一律跟随（`kde.rs` → `theme.rs::Theme::from_kde`），不写死颜色。
5. **一致优先于个性**：同类控件在同一处、不同处长得一样；不在局部发明新样式。

## 2. 颜色（`crates/kichi-gui/src/theme.rs::Theme`）

所有颜色从 `Theme` 实例取（`th.xxx`），不要直接写 `Color32::from_rgb(...)`（`theme.rs` 内的兜底配色除外）。

| 字段 | 用途 |
|:--|:--|
| `accent` / `on_accent` | 品牌强调色 / 强调色上的文字（选中、主按钮） |
| `bg` / `panel` / `card` | 页面底 / 侧栏面板 / 视图区（列表、卡片） |
| `hover` | 行 / 项**悬浮打底** |
| `border` | 分隔线、少量必要描边 |
| `text` / `text_weak` / `text_faint` | 主 / 次 / 最弱（占位、辅助说明）文字 |
| `ok` / `warn` / `danger` | 成功 / 警告 / 危险 |

- 强调色的**淡染底**统一用 `th.accent_soft()`（选中标签、搜索指示等），不要自己 `mix` 一套。
- 需要新的中间色时用 `theme::mix(base, fg, alpha)`，不要在业务代码里手搓 `from_rgb`。
- 深浅色由 `th.dark` 分支控制透明度系数；两套都要能看。

## 3. 圆角与间距

圆角一律经 `th.cr(n)` 换算：Breeze 下**控件 4px、卡片/面板 6px**（`n<=3` 保留原值，`3<n<=10` → 4，其余 → 6）；非 Breeze 沿用原值。

间距由 `theme.rs::configure` 统一下发，**不要在业务代码另设一套**：

| 项 | 默认 | Breeze |
|:--|:--|:--|
| `item_spacing` | (10, 8) | **(8, 6)** |
| `button_padding` | (12, 6) | **(10, 5)** |
| `interact_size.y` | 30 | 30 |
| `menu_margin` | 8 | 8 |
| 滚动条 | 默认 | 细：宽 8 / 手柄 24 / 内 2 / 外 2 / 非浮动 |

常用尺寸基准（与现有一致，新增沿用）：

| 元素 | 尺寸 |
|:--|:--|
| 侧栏宽度 | 默认 228，可调 196–320（`app/sidebar.rs::app_shell`） |
| 侧栏导航项高 | 36（`app/sidebar.rs::nav_item`） |
| 文件页顶栏高 | 44（`app/files/mod.rs::FilesPage::HEAD_H`） |
| 文件列表行高 | 40（`app/files/row.rs::file_row`） |
| 回收站行高 | 52（`app/trash.rs`） |
| 网格卡片边长 | 80–160，Ctrl+滚轮缩放（`app/files/grid.rs::GRID_CARD_MIN/MAX`） |
| 图标按钮 | 28–30 见方（如上传按钮 30） |

## 4. 字体与文案

- 中文字体经 `app/helpers.rs` 从系统加载（Proportional / Monospace 双族回退），不内嵌字体文件。
- 字号梯度（对照现有取值，新文案就近取档）：
  - **19** 页面标题（strong，如「设置」「回收站」）
  - **14–15** 区块标题 / 导航 / 主按钮
  - **13–13.5** 正文、文件名校、输入框
  - **12–12.5** 次级信息、计数、说明
  - **11–11.5** 最弱辅助、时间戳、标签
- 层级用 `text / text_weak / text_faint` + 字号表达，不靠加粗堆叠。
- 文案一律中文（见 `AGENTS.md` 第 1 条）；按钮用动宾短语（「上传文件」「移入回收站」）。

## 5. 图标

- 属 `crates/kichi-gui/src/icons.rs` 的**自绘矢量图标**（`Glyph` + `icons::paint`），16×16 规范坐标等比缩放；**不引入字体图标 / 位图图标**。
- 新增图标：在 `Glyph` 加变体，在 `icons::paint` 的 `match` 加分支并实现同一坐标系的绘制函数。
- 图标颜色传 `th.text_weak`（常态）/ `th.accent`（选中）/ `th.text_faint`（禁用、装饰）。
- 图标按钮 = 透明底 + 悬停 `th.hover` 圆角底 + 图标（参考 `app/files/toolbar.rs::top_bar` 的刷新 / 上传按钮）。
- 文件类型图标 / 预览 / 系统打开 / 只下载共用 `filetypes.rs` 单点表，不要另起一套。

## 6. 布局骨架

```text
SidePanel(left, 侧栏)  +  CentralPanel(当前页)
                         └─ 页内可再叠 TopBottomPanel(顶部栏) / 内容区
```

- 侧栏 `app/sidebar.rs::app_shell`：`Frame` 只填 `th.panel` + 内边距，**不描整面板边框**（右侧分隔线交给 `SidePanel` 自带 separator）。
- 内容页顶栏用 `TopBottomPanel::top`，`Frame` 填 `th.bg` / `th.card`；列表 / 网格**直接平铺**，不要在外层再套一层卡片与边框（Dolphin 风格，见 `app/files/mod.rs::show` 的注释）。
- 弹窗 / 菜单的**外框保留一层**（`Frame::popup` / window），但里面**不要再套第二层框**。

## 7. 控件规范

### 按钮
- 主操作：`accent` 实底 + `on_accent` 文字（如「下载到本地」「登 录」）。
- 次级操作：透明底 + `text_weak`，悬停才打底；**不要常态描边**。
- 危险操作：`danger` 文字 + 淡描边或实底，明显区别于普通项（如「移入回收站」）。
- 纯图标按钮用 `icons::paint` 自绘 + 悬停 `th.hover` 底，可加 `on_hover_text` 提示。

### 菜单 / 下拉（KDE 观感）
- 统一走 `app/files/toolbar.rs::popup_menu`：
  - 菜单项**常态无边框**，仅悬停时整行填充高亮（`th.hover`）、按下 `th.accent_soft()`；
  - 宽度按最长项自适应（可设上限后截断），不做固定大宽度；
  - 弹层锚定触发控件（`egui::popup_below_widget` + `PopupCloseBehavior::CloseOnClick`）。
- 不要用 `egui::Button` 直接当菜单项——全局按钮视觉带的 1px 边框正是「框里套框」的来源。

### 列表 / 表格
- 行：透明底，悬停 `th.hover`；选中项用强调色淡染（`accent_soft`）或复选框表示，不要描边。
- 表头与行用 1px `th.border` 分隔线即可（`app/files/list.rs`）。

### 网格卡片
- 卡片无边框，选中 / 悬停用打底与轻微描边；缩略图保持宽高比居中，尺寸随卡片等比缩放（`app/files/grid.rs`）。

### 面包屑
- 扁平文字 + `ChevronRight` 分隔；当前级 `th.text`，上级 `th.text_weak`；悬停上级打底。
- 宽度不足时从左侧省略中间层级，保留当前目录（必要时截断）；省略出的「…」可点击，弹出被省略层级供跳转（`app/files/toolbar.rs::breadcrumbs`）。
- 面包屑宽度估算必须计入 egui 的 `item_spacing`（`allocate_exact_size` 每项都会追加），或临时置零，避免溢出顶到右侧控件。

### 对话框 / 弹窗
- `egui::Window` / `Frame::popup`，圆角 `th.cr(14)`（window）/ `th.cr(8)`（menu）；标题 14–15，正文 12–13。
- 危险确认（删除 / 清空）用 `danger` 色，按钮文案明确动宾。

### 提示条 / Toast
- 成功 `ok`、失败 `danger`、警告 `warn`，短句 (不超过一行)；走 `Global::toast_*`，不自己弹窗。
- 长等待（预览解析 / 唤起外部软件）用**常驻提示**（`Global::toast_sticky`）：显示到结果到达、被结果提示替换，不设自动超时；在途状态被整体作废时用 `Global::clear_sticky_toast` 收回。

### 进度与加载
- 进度条 / Spinner 用 `text_weak` / `accent`；卡片内展示「已完成 / 全部」「速率 / 剩余时间」等次级信息（`app/transfers/model.rs` 提供聚合）。

## 8. 交互反馈

- 三态一律**用填充深浅**表达，不用边框：常态透明 → 悬停 `th.hover` → 按下 / 选中 `accent_soft` 或 `accent`。
- 悬停可交互元素给出 `on_hover_text`（纯图标按钮必给）。
- 禁用态：降透明度 / 用 `text_faint`，不隐藏（保留布局稳定，避免跳动）。

## 9. 新建 / 改动界面时的检查清单

- [ ] 颜色全部来自 `Theme`，无硬编码 `from_rgb`（`theme.rs` 兜底除外）。
- [ ] 圆角走 `th.cr(n)`，间距沿用全局 `spacing`，未另设一套。
- [ ] 控件 / 菜单项常态**无边框**；反馈只用填充。
- [ ] 没有「框里套框」：弹层外框只保留一层，列表 / 内容区不套多余卡片。
- [ ] 菜单用 `popup_menu` 或同风格扁平行，不用裸 `Button` 当菜单项。
- [ ] 图标用 `icons.rs` 自绘 `Glyph`，未引入字体 / 位图图标。
- [ ] 深浅色两套都可读，长文本会截断 / 省略而非溢出。
- [ ] 过闸门：`cargo fmt --all` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace`。
- [ ] **实机冒烟**（GUI 布局与交互单测覆盖不到），确认无重叠、无错位、悬停反馈正确。

## 10. 与既有代码的取舍

- 全局按钮视觉（`theme.rs::configure`）目前给按钮保留 1px 边框，这是常规表单按钮的既有观感；**新做菜单 / 行式交互时不要复用它**，改用扁平行。
- 与本文冲突的既有实现，向 KDE 观感收敛；改动范围大时按 `AGENTS.md` 第 5 条拆小步、逐个实机确认。
- 规范拿不准时：**先按 KDE 观感做，再让维护者实机定稿**，不要自己发明第三种风格。
