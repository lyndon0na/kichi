# Kichi 开发任务清单

> [!NOTE]
> 本清单独立于 [`PROJECT_PLAN.md`](./PROJECT_PLAN.md)：`PROJECT_PLAN` 记录**已完成的里程碑与实现笔记**，这里只收敛**待办与改进项**，按优先级排序。
> 文件定位以「文件 + 关键符号」标注（不写行号，避免格式化后漂移）。每项完成后请同步回 `PROJECT_PLAN.md` 的「未实现 / 已知边界」与「路线图」。

## 进度总览

| 优先级    | 主题        | 项数  |
|:------ |:--------- |:--- |
| **P0** | 用户可见的功能缺口 | 0   |
| **P1** | 正确性与健壮性   | 0   |
| **P2** | 可维护性与工程   | 5   |
| **P3** | 分发与发布     | 0   |

> 上表只统计主线队列（P0–P3）；另有一节独立的「P2 · 结构优化」队列（第一批 P2-9 ~ P2-14 **全部完成** —— P2-9 / P2-10 / P2-11 / P2-12 / P2-13 已落地，P2-14 已由 P2-10 / P2-11 / P2-12 覆盖；第二批收尾 P2-15 ~ P2-17 已立项待做），见下方专节。

---

## P0 · 用户可见的功能缺口

> ✅ 全部完成

## P1 · 正确性与健壮性

> ✅ 全部完成

## P2 · 可维护性与工程

> ✅ 全部完成

## P3 · 分发与发布

> ✅ 全部完成

---

## P2 · 结构优化：巨型文件与 god object（第一批 P2-9 ~ P2-14 ✅ 已完成 · 第二批 P2-15 ~ P2-17 待做）

> 目标不是「把文件切小」，而是**结构优化**：按域建子目录 + 让页面各自持有状态。
> 判据（怎么切才算对）：
> ① 职责能一句话命名；文件能一口气读完。
> ② **目录 = 域的边界，文件 = 域内的角色**（文件名通常取里面的核心类型名）。
> ③ **一起变的东西放一起**：同一处业务改动若总要同时改 3 个文件，说明切错了。
> ④ 停止信号：出现 `utils.rs` / `common.rs`、叫不出某个文件的名字、一个类型的 `impl` 散落多文件（手写 trait impl 单独成文件是正常例外）、为「每文件 < 500 行」这类教条继续切。
> 参照（本机 `~/.cargo` 源码实测，`wc -l`）：egui 114 文件 / 中位 226 行，但 `context.rs` **4420 行**、`style.rs` 3163；eframe 28 文件 / 最大 1666；image 71 文件 / 最大 2476（`codecs/` 一个目录 39 文件，每种格式一个）；rustls 107 文件 / 最大 `msgs/handshake.rs` **3266 行**（协议消息表，形态类似 `worker::handle`）—— **大文件本身不是问题，一个文件里混着多个概念才是**。
> 因此本仓库**不追「一个函数一个文件」**；`App` 这种 god object（100+ 方法与字段）光拆文件治不了，页面结构化才是正解。

### 路线（2026-09-23 拍板）

**已定 A**：拆域时**顺手 struct 化** —— `app/files/` 里直接放 `FilesPage`（自身状态）+ `show()`，`App` 持 `files: FilesPage`；即 P2-11 / P2-12 与 P2-14 合并。下表为决策时的三选项，留作备查（B 要再搬一轮、C 让 god object 期间继续长，均未取）。

**P2-10 切法（2026-09-24 细化）**：域 = 结构体 + 方法 + `drain` 一行转调（沿用 P2-13 的形态）；新增极小的共用句柄 `app/global.rs::Global`（`tx` / `toast` / KDE 配色与主题轮询），域方法签名 `fn on_x(&mut self, g: &mut Global, …)` —— App 侧写 `self.trash.on_x(&mut self.global, …)` 是不相交字段借用，不引入 `Rc` / `RefCell`；`App::send` / `theme` / `toast_*` 保留转发壳，页面调用点零改动；跨域副作用（目录缓存作废、配额刷新、页面路由）留在 `drain` 臂里做 2–4 行编排。对原条目的两处修正：① `app/browse/` 并入 P2-11 的 `FilesPage`（目录数据就是文件页的状态，先搬会再收编一次）；② tasks 域纳入 P2-10。`app/system.rs` / `app/dnd.rs` 取消（避免夹缝模块）：`poll_file_picker` / `upload_pick` / `last_dir` 归上传域（P2-12），`poll_pending_open` 归预览域，`persist_settings` / `chrono_now` / `choose_download_dir` 跟全局设置走。

| 选项       | 做法                                                                                             | 代价                                                     |
|:-------- |:---------------------------------------------------------------------------------------------- |:------------------------------------------------------ |
| **A（推荐）** | 拆域时**顺手 struct 化**：`app/files/` 里直接放 `FilesPage`（自身状态）+ `show()`，`App` 持 `files: FilesPage`；即 P2-11 / P2-12 与 P2-14 合并 | 一次到位，不用同一批代码搬两次家；但触及所有 `self.xxx` 引用，diff 大，必须逐页 UI 冒烟 |
| B        | 先只按职责切文件、不动结构                                                                                  | diff 小、好回滚；之后还要再搬一轮（P2-14 不能省）                          |
| C        | 先做 `worker/`（管线边界最清晰、无 UI 状态纠缠），页面结构化留到最后                                                       | 业务风险最低；god object 留到最后，`app/mod.rs` 期间仍在长                   |

### 队列

| 编号    | 任务                                | 主要路径                                                                                       | 说明 / 验收                                                                                                                                      |
|:------ |:--------------------------------- |:------------------------------------------------------------------------------------------ |:--------------------------------------------------------------------------------------------------------------------------------------------- |
| P2-9   | `app/mod.rs` 纯逻辑下沉（第 1 步，建议先做） | 新模块（现为 `app/transfers/model.rs`）                                                        | **✅ 已完成**（`e85211b` 搬移 + `caa89db` 补测 + `c9b57a3` 抽 `restore_req_id` / `start_session`）。`dl_record_status` / `aggregate_children` / `compute_dir_counts` / `sample_speed` 四个无 UI 依赖函数连同 5 项相关单测外移到新模块，另补 2 项（此前零覆盖：状态映射兜底、短间隔不取样防速率爆炸）→ 新模块共 7 项单测；`App::new` 只留装配与字体告警。`app/mod.rs` 3264 → 3085 行。零行为变更（函数体 / 文档注释逐字比对、抽取语句 `app.` → `self.` 逐行一致），`fmt` 零差异 / clippy 0 告警 / **93 单测**全绿（计划时写的「8 项单测」「89 项」为估算，按实况修正）；实机冒烟已确认（维护者，2026-09-24） |
| P2-10  | `app/` 按域 struct 化（含拆 `drain`）      | `app/{global,trash,shares,preview,search,thumbs,tasks}.rs`                                  | **✅ 已完成**（D0 `935d3dd` · D1 `83f5ddd` · D2+D3 `72b670a` · D4 `7a6493b` · D5 `9e4b919` · D6 `d0be9fc` · D7 `73edc3e`）。七个域结构体 `Global` / `TrashPage` / `SharesPage`（我的分享 + 转存分享）/ `PreviewPage` / `SearchPage` / `ThumbsPage` / `TasksPage` 各持自身状态与 `on_x` 方法，`App` 侧写 `self.trash.on_x(&mut self.global, …)`（不相交字段借用，无 `Rc` / `RefCell`）；`drain` 的 60 个 `Msg` 臂全部一行转调、跨域副作用留 2–4 行编排，`app/mod.rs` 3085 → 2127 行（`drain` 由约 840 行降至 548 行）。每步纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / **93 项单测**全绿（core 33 + GUI 60，各步不减），并逐个用归一化脚本比对旧新函数体（逐段 `ALL_MATCH`）；`App::new` 的 310 行字面量随各域 `Default` 自然消掉。实机冒烟已确认（维护者，2026-09-24） |
| P2-11  | `files_page.rs` → `app/files/`      | `app/files/{mod,list,grid,row,toolbar}.rs`                                                  | **✅ 已完成**（F1 `37c70ab` · F2 `93a769c` · F3 `60c00cc` · F4a `d86d268` · F4b `1ee33b1`）。`FilesPage` 落地在 `app/files/mod.rs`，一并**吸收 P2-10 原列的 `browse/`**（目录缓存 SWR / 导航栈 / 文件列表即文件页状态：`send_list` / `fetch_dir` / `revalidate_dir` / `reload_dir` / `on_files` / `evict_dir_cache` / `goto_folder` / `current_parent`），不再单独建 `app/browse/`；渲染按角色分家：`list.rs`（`col_layout` / `file_list_header` / `list_rows`）、`grid.rs`（`grid_cards` + 原有 `thumb_row_range` / `thumb_max_edge` / 卡片尺寸常量）、`row.rs`（`file_row` / `play_menu` / `preview_menu_items`）、`toolbar.rs`（`top_bar` + `breadcrumbs`）。渲染与动作分离：`FilesPage::show(...) -> Vec<FilesAction>` 只读写自身状态，15 个跨域动作（弹窗 / 预览 / 播放 / 下载 / 分享 / 上传 / 搜索）由 `App::apply_files_action` 在本帧渲染后按序执行；顶部栏与两个视图各收一个上下文结构（`TopBar` / `ListCtx` / `GridCtx`）避免参数过长。`app/files/mod.rs` 1891 → 1098 行（`show` 约 440 行 = 状态机 + 入口）。每步纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / **93 项单测**全绿，逐词归一化脚本比对旧新函数体（`files_page=show` 38 处差异全为 `req.` 前缀与动作外提；两个视图分支差异仅有 `cx.` 前缀、`&mut` 重借与格式化换行）。唯一有意的微差异：旧实现「用户取消选下载目录」时会提前 `return` 漏画一帧列表，现在列表照画。实机冒烟已确认（维护者，2026-09-24） |
| P2-12  | `transfers_page.rs` → `app/transfers/` | `app/transfers/{mod,download,upload}.rs`                                                    | **✅ 已完成**（T0 `6550268` · T1 `ccf255c` · T2 `54f0636` · T3 `be44c61` · T4 `a81afa9` · T5 `cc20d19`）。`transfers_page.rs`（1951 行）与 `transfers_model.rs` 归并为 `app/transfers/`：`TransfersPage` 持传输域全部状态（下载 / 上传两条任务表、选中集、筛选、展开、上传选择框）与生命周期方法（入队 / 重试 / 移除 / 历史记录 / 目录扫描收敛 / 进度与速率），`drain` 的 11 条传输消息臂改为一行转调（仅 `UlFinished` 留 4 行跨域编排：作废文件页目录缓存 + 刷新配额）；渲染入口收成 `TransfersPage::show(...) -> Vec<TransfersAction>`，4 类跨域动作（打开本地路径 / 打开下载目录 / 文件选择框 / 跳转「我的文件」）由 `App::apply_transfers_action` 在本帧渲染后执行；两个分栏分家：`upload.rs`（`upload_tab` / `upload_action_bar` / `ul_card` / `upload_status_line`）、`download.rs`（`download_tab` / `download_action_bar` / `dl_card` / `dl_dir_node` / `dl_file_node` / `status_line` / `paint_rails` / `paint_disclosure` / `node_h` / `tree_block_height`），无 UI 依赖的纯逻辑留在 `model.rs`；`last_dir` / `download_dir` 等持久化设置与下载目录选择仍留 `App`。前置 T0 把 `req_id` 分配器从 `FilesPage` 搬到 `Global`：worker 的 `cancel_task` 只认一张按 req_id 索引的取消登记表，下载 / 上传 / 预览必须共用同一命名空间（原先下载走 `App`、上传走 `FilesPage`，双分配器迟早撞号）。`app/mod.rs` 1679 → 968 行；旧 1951 行的 `transfers_page.rs` 变为 `mod.rs` 1216（状态 + 生命周期 + 页壳）/ `download.rs` 1020 / `upload.rs` 591 / `model.rs` 253。每步纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / **93 项单测**全绿（core 33 + GUI 60，各步不减），逐词归一化脚本比对旧新代码（`download.rs` 的旧代码整体按序包含于新文件，差异只有 `pub(super)` 可见性、动作外提与 rustfmt 换行）。实机冒烟已确认（维护者，2026-09-24） |
| P2-13  | `worker.rs` → `worker/`            | `worker/{mod,gate,cache,auth,files,tasks,shares,download,upload,preview,thumbs}.rs`         | **✅ 已完成**（`9da37dd` 起 19 个提交：W1–W9 = `3d5e23c` `8a8861e` `2b5901c` `815e4a7` `6b26981` `1d888aa` `08eb956` `fcd8259` `f4532b7`；W10 收尾 = `2e3e122` `4721eb6` `2437def` + `320aa98`（补漏掉的 `.await`）`5ee58d9` `f2b96c4` `f618e6a` `174d34f` `5997c77`）。`worker.rs` 2460 行 → 12 个文件、入口 `worker/mod.rs` 367 行，`handle` 的 43 个 `Cmd` 分支全部一行转调；纯搬移零行为变更，每步 fmt 零差异 / clippy 0 告警 / 91 单测全绿。实机冒烟（登录 / 上传 / 下载 / 预览 / 转存）已确认 |
| P2-14  | `App` 字段按页面分组为子结构                 | `app/mod.rs` + 各页面                                                                            | **已并入 P2-10 / P2-11 / P2-12**（目标形态各有归属：`FilesPage` = P2-11、`TransfersPage` = P2-12、`TasksPage` / `SharesPage` / `TrashPage` / `PreviewState` / `ThumbsState` = P2-10），本条不再单独做，行内不再计为待办 |

### 第二批（2026-09-24 立项，待做）

> 第一批（P2-9 ~ P2-14）收尾后的复核结论：主线队列已清空，但按判据 ② / ③ 还剩三处同类尾巴。三条互不依赖、可各自独立回滚，建议顺序 P2-15 → P2-16 → P2-17（P2-17 收益最低，可后置）。外部结构建议（`worker/handlers/` 与 `app/state/` 桶目录）经复核**不采纳**：其目标与本轮一致，但目录形态是夹缝层（会把下载 / 上传两条管线捆回一个文件、把文件页与预览的状态从各自域里再切出去），违背判据 ② / ③ —— 完整理由见 `PROJECT_PLAN.md` 第五节 11)。

| 编号    | 任务                                             | 主要路径                                            | 说明 / 验收                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
|:------ |:---------------------------------------------- |:----------------------------------------------- |:------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P2-15  | tasks 域收口（唯一还挂 `impl App` 的带状态页面）              | `app/tasks.rs` + `app/tasks_page.rs` → `app/tasks/` | 现状：状态 / 生命周期在 `app/tasks.rs`（317 行，`TasksPage`），渲染在 `app/tasks_page.rs`（643 行，`impl App`：`task_card` / `task_tab_button` / `tasks_page`；`self.tasks` 46 处、跨域 `self.download_single` 1 处）—— 正是 P2-11 / P2-12 之前的形态。目标：`app/tasks/`（`mod.rs::TasksPage` + `show(…) -> Vec<TasksAction>`，跨域动作由 `App::apply_tasks_action` 在本帧渲染后执行；渲染按角色分文件：状态页签与列表 / 任务卡片 / 「保存到」选择器），与 `app/files/` / `app/transfers/` 同形；自适应轮询触发的落点实现时定（域内或 `App::update`）。验收：沿用下方固定验收 + 实机冒烟（页签切换 / 分页加载更多 / 多选批量重试与删除 / 「保存到」目录选择 / 下载到本地）                                                                                                     |
| P2-16  | `app/types.rs` 类型随域归位                          | `app/types.rs` → 各域文件 + `app/global.rs`         | 现状 506 行：跨域类型与各域类型混放一处，改传输任务类型要同时开 `app/transfers/*` 与 `app/types.rs`，撞判据 ③。归位：`Dl*`（`DlJob` / `DlNode` / `DlStatus` / `DlOp` / `DlRow` / `DlFilter` / `DlSel`）→ `app/transfers/` 下载侧；`Ul*` / `UploadPick` → 上传侧；`DirEntry` / `RowAction` / `ViewMode` / `ClipKind` / `Clipboard` / `ColDrag` / `Crumb` / `SortBy` → `app/files/`；`OfflineTab` / `TaskOp` / `TaskSel` → tasks 域（P2-15 之后）；`QualityReady` / `QualityMenuState` / `PendingOpen` / `PreviewConfirm` / `PreviewProgress` → `app/preview.rs`；`ShareResult` → shares 域；仅 `Page` / `TransferTab` / `CacheUsage` 跨域共享，留 `global.rs`（`app/types.rs` 清空后删除）。验收：纯机械搬移零行为变更（`fmt` 零差异 / clippy 0 告警 / 93 单测不减），可见性（`pub(crate)` / `pub(super)`）以实际引用面为准                  |
| P2-17  | `app/shares.rs` 渲染按角色分文件（可后置）                  | `app/shares.rs` → `app/shares/{mod,mine,restore}.rs` | 现状 1599 行（GUI 最大）：状态与生命周期约 400 行 + `draw`（我的分享列表 + 转存入口）+ `draw_dialogs`（创建分享设置框 / 分享结果 / 取消分享确认）+ `draw_save_dialogs`（转存分享弹窗 / 目标目录选择器 / 自动移动失败重试）+ 单测。已是 `&mut Global` 形态（非 `impl App`），**不动结构与 `App` 字段**，只做分文件：`mod.rs`（`SharesPage` + 生命周期 + 页壳）/ `mine.rs`（我的分享 + 创建分享）/ `restore.rs`（转存：解析 / 浏览 / 保存）。验收：沿用固定验收 + 实机冒烟（转存链接解析 / 分享目录浏览 / 目标目录选择 / 保存后移动与重试 / 我的分享列表与取消分享）                                                                                                                              |

### 每步的固定验收

- `cargo fmt --all` → `cargo clippy --workspace --all-targets -- -D warnings`（0 告警）→ `cargo test --workspace` 全绿，`#[test]` 数不减（当前基线 **93 项**：core 33 + GUI 60）。
- 一次一个域、独立提交（`refactor(gui): …`），**纯搬移、零行为变更**；`worker/` 与上传 / 下载管线额外实机验证。
- 路径变了就必须同步 `README.md` 目录树与 `AGENTS.md` 代码地图，并在 `PROJECT_PLAN.md` 第五节 11) 追加进度。

---

## 已完成（本轮）

- [x] 文档失真修正：离线任务翻页说明、README 目录树补 `lib.rs` / `logging.rs`（`9c66bfa`）

- [x] 工程杂项：`LICENSE`、`CHANGELOG.md`、`rustfmt.toml`，移除占位 `repository`，版本号去硬编码，`.gitignore` 扩充，全仓库 `cargo fmt`（`d6044a3`）

- [x] P0-1 离线任务「加载更多」分页；顺带把离线任务 / 配额的自动轮询改为自适应节拍（`worker.rs` / `tasks_page.rs` / `app/mod.rs` / `msg.rs`）

- [x] P0-2 整目录递归下载：`kichi-core::walk_folder` 递归遍历 + 本地按云端层级建目录；传输任务页聚合为单张目录卡片（显示「文件 已完成/全部」），展开后按层级显示目录树、子目录可单独折叠；重试仅重下失败子文件（`client.rs` / `worker.rs` / `app/mod.rs` / `files_page.rs` / `transfers_page.rs` / `settings.rs` / `msg.rs`）

- [x] P0-3 真实缩略图：网格视图加载 `thumbnail_link` 预签名直链，worker 下载 + 磁盘缓存（`~/.cache/kichi/thumbnails/`）+ `image` crate 解码为 RGBA 后上传 GPU 纹理；保持宽高比显示；Ctrl + 滚轮缩放网格大小（80–160px），图标 / 文字 / 缩略图随卡片等比缩放（`client.rs` / `worker.rs` / `msg.rs` / `app/mod.rs` / `files_page.rs`）

- [x] P0-4 全局搜索：PikPak 无服务端搜索 API，采用客户端递归遍历所有目录并按文件名模糊匹配；搜索框按 Enter 触发搜索，支持分页加载更多；搜索模式下显示搜索结果指示器，导航/面包屑点击自动退出搜索模式（`client.rs` / `msg.rs` / `worker.rs` / `app/mod.rs` / `files_page.rs`）

- [x] P1-1 分享转存目标目录改为持久化 ID：新增 `settings::load_pack_folder_id` / `save_pack_folder_id`（独立文件 `~/.config/kichi/pack_folder_id`，避免与 GUI 线程覆写 settings.json 竞争）；`worker.rs` 新增 `find_pack_folder` 统一按持久化 ID 定位「转存自分享」暂存目录，ID 失效时回退名称匹配并刷新缓存，`snapshot_pack_folder` / `move_new_files` / `Cmd::RetryMoveShare` 全部改用该入口（`worker.rs` / `settings.rs`）

- [x] P1-2 分享链接解析增强：`extract_share_id` 改为 `parse_share_input`，支持带查询参数 / 片段 / 复制链接附带前后文字的形态，ID 截到首个非法字符为止，并顺带从 `password`/`pass_code` 回填提取码；无法识别（缺 `/s/` 的其它链接、非法字符、空）时返回 `None`，解析对话框给出错误提示而非当成裸 ID；补 `parse_share_id_from_url_forms` 单测（`mod.rs` / `dialogs.rs`）

- [x] P1-3「打开下载目录」空路径修复：系统下载目录取不到时不再回退成空路径导致按钮无反应，改为回退到已记住的下载目录，两者都无效时给出 toast 提示前往设置选择（`transfers_page.rs`）

- [x] P1-4 人机验证流程：`captcha_init` 取不到 `captcha_token` 时改为返回新错误变体 `Error::CaptchaReview`（携带从响应里递归提取的验证页链接 `data.url`/`*url`）；`Msg::LoginFailed` 增加 `verify_url` 字段并透传到 `App::auth_captcha_url`；登录页在需要验证时显示「打开验证页面」按钮（`helpers::open_url`）+ 完成验证后重试的引导，无链接时给出「稍后重试 / 换网络 / 用官方客户端验证」提示；补 `extract_verify_url` 单测（`error.rs` / `client.rs` / `msg.rs` / `worker.rs` / `app/mod.rs` / `login.rs`）

- [~] P2-1 上传跨重启续传 —— **调查后判定不可行，代码已回退**（详见根目录 `UPLOAD_RESUME_NOTES.md`）。曾实现一版（`upload_resume.json` 持久化 upload_id / OSS 位置 / 已传分片 ETag，重启后重刷 STS 凭证 + `oss_list_parts` 对账续传），实机验证发现 PikPak 的 STS 凭证**按对象 key 授权**、而每次 `upload_create` 都换 key（key = `upload_tmp/<GCID>_<时间戳>`），用新票凭证访问旧 upload_id 直接 `403 AccessDenied: Access denied by authorizer's policy`，续传被服务端 policy 挡死。已将整套续传代码回退到「重启即全量重传」的干净状态（保留同票同凭证的**会话内**退避重试不受影响）。若日后要真做，唯一方向是持久化并在有效期内**免取票复用凭证本身**对旧 key 续传，但有安全与 STS 寿命窗口限制、且完成落库环节仍有未验证风险。

- [x] P2-1b 上传/下载速率显示异常修复（随本次一并保留）：`drain()` 单帧内一次性消费积压的多条进度消息时，逐条按 `Instant::now()` 取样会因 `dt≈0` 让瞬时速率爆炸；新增 `sample_speed` 按 0.25s 节流取样，并补 `has_active_uploads` 使纯上传时也走快轮询，消除消息堆积（`app/mod.rs`）

- [x] P2-2 并发 / 重试参数可配置：设置页新增「传输」卡片（下载并发 1–8 / 上传并发 1–4 / 上传分片并发 1–10 / 单任务重试次数 1–10），改动即持久化并推送 worker；并发经 `Cmd::SetTransferLimits` 即时生效（扩容立刻放行排队任务、缩容不打断在传任务），重试与分片并发对新启动任务生效。原编译期常量（`DL/UL_CONCURRENCY`、`DL/UL_MAX_ATTEMPTS`）删除；`OSS_UPLOAD_CONCURRENCY` 改为 `KichiClient` 的 `part_concurrency` 原子字段 + setter。tokio `Semaphore::set_capacity` 未稳定、`forget_permits` 缩容会被释放许可回填，故自研动态并发闸 `Gate`（`Notify::enable` 先注册后判定避免丢唤醒），并补扩容 / 缩容语义单测（`settings.rs` / `worker.rs` / `msg.rs` / `client.rs` / `app/mod.rs` / `app/settings_page.rs`）

- [x] P2-3 日志轮转 + 磁盘缓存淘汰：`logging.rs` 的 `Mutex<File>` 换成 `LogSink`（按大小轮转，1 MiB × 3 备份，**只在行尾切**以免把一条 event 劈到两个文件；启动时兜底轮转；`KICHI_LOG_MAX_MB` / `KICHI_LOG_FILES` 可覆盖；轮转或写失败降级为只写 stderr）。新增 `cache.rs`：预览与缩略图共用一套按 mtime 的 LRU（预览 512 MiB / 200 条，缩略图 128 MiB / 1000 条，**双上限**同时生效），命中缓存时 `touch` 刷新 mtime，**上限是软上限**（豁免窗口内 / 正在使用 / 自身超限的条目不删，淘汰扫一轮即止，杜绝「预览完自删、下次又重下」）；目录条目的「最近使用」取子树最新 mtime，避免命中缓存时目录 mtime 不变导致误判；worker 在启动时后台扫一次、写入后按节流（≥60s 或新增 ≥8 MiB）触发，预览 / 字幕 / 缩略图下载期间登记 `in_use`（含 `.part`）防止被并发淘汰；设置页新增「缓存」卡片展示占用并可清空（`Cmd::MaintainCache` / `Msg::CacheUsage`）。新增 11 项单测（日志轮转 3 + 缓存淘汰 8）（`logging.rs` / `cache.rs` / `main.rs` / `worker.rs` / `msg.rs` / `app/mod.rs` / `app/types.rs` / `app/sidebar.rs` / `app/settings_page.rs`）

- [x] P2-7 缩略图调度重构：`Cmd::LoadThumbnail` 从「在命令主循环里 `await`」改为 `tokio::spawn` + 独立并发闸（4；与用户可配的下载并发解耦 —— 缩略图不该排在大文件下载后面，也不该占用户为下载预留的槽位），命令循环不再被占住。请求点由整目录收窄为「可见行 ± 一屏」（抽出 `thumb_row_range` 自由函数并补 4 项边界单测）。新增目录代数 `thumb_gen`：目录切换 / 刷新 / 新搜索时自增，旧任务在下载前 / 解码前 / 回包前三个检查点自行放弃。失败不再静默：下载按 600ms 递增退避重试 3 次，终态回 `Msg::ThumbnailFailed`，UI 结束在途登记并记入 `thumbnail_failed`（本会话不再重复请求，刷新目录 / 重新搜索时清空重试）。`download_thumbnail` 改 `.part` + rename 原子落盘并加 8 MiB 响应上限，堵住「截断的缓存文件被 `exists()` 当成命中 → 解码失败 → 永不重下」的第二个空洞来源（`worker.rs` / `msg.rs` / `app/mod.rs` / `app/files_page.rs` / `client.rs`）

- [x] P2-6 预览入口范围分层 + 健壮性（4 提交：`4d85185` / `a118b52` / `f5cf109` / `5d25f44`）：
  
  - **类型分类单点**：新建 `filetypes.rs`，判定顺序为「强 mime 优先 → 通用 mime（`octet-stream` 等）视为无信息 → 唯一扩展名表」；图标与预览判定共用同一份表，消除 `m2ts` / `mpg` / `mpeg` 图标显示视频却走整份下载的不一致（`files_page.rs` / `trash_page.rs` / `helpers.rs` 的重复扩展名表删除）。
  - **入口分层（最终取「排除名单」口径）**：只有压缩包 / 镜像 / 可执行 / 种子不给「打开」（双击等兜底路径在 `open_preview` 里 toast 并提示改用「下载到本地」），其余类型（含 `.psd` / `.wps` / 无扩展名等未知类型）保留「打开」—— 白名单需覆盖 Office 全家桶 + WPS 专有格式，易漏。原 TODO 里「未知类型也取消」的表述按此口径修正。
  - **大文件确认**：非媒体预览需先整份下载，`size > 64 MiB` 且未命中预览缓存时先弹「预览大文件」（显示名称与大小），确认后才发起；媒体走 mpv 流播无整份下载成本，不弹。
  - **假成功修复**：`helpers::open_path` / `open_dir` 换成后台探针 `open_async`（xdg-open 退出码非 0 时按 stderr 措辞区分「无关联程序」与其它失败，1s 超时视为已启动，避免阻塞式 handler 拖住提示），结果经 `PendingOpen` 通道回 UI；预览打开与传输页「打开文件 / 打开目录」三处统一走探针，打开目录成功保持安静。
  - **进度与取消**：非媒体预览下载接入 `download_to` 的 cancel / on_progress 管线（150ms 节流），右下角常驻状态条显示文件名 + 进度条（总大小未知时走不确定动画）+ ✕ 取消；取消后 worker 丢弃未完成的 `.part` 并静默退出（新增 `Cmd::CancelPreview` / `Msg::PreviewProgress`）。
  - 新增单测 10 项：filetypes 分类 9（媒体扩展名 / 强 mime 覆盖 `.ts` / 通用 mime 回退 / 非媒体分类 / 只下载类 / 保留打开类 / 图标与判定一致 / 字幕识别 / 文件夹）+ 探针措辞匹配 1（`filetypes.rs` / `app/helpers.rs` / `app/mod.rs` / `app/files_page.rs` / `app/transfers_page.rs` / `app/dialogs.rs` / `app/types.rs` / `worker.rs` / `msg.rs` / `main.rs`）

- [x] P2-4 补测试（唯一保留项）：`de_number` / `de_string` 的 serde 兼容性用例 5 项 —— `File.size` 与 `Quota` 覆盖数字 / 数字字符串 / 浮点截断 / `null` / 缺字段；`Share` 计数类字段覆盖数字 / 布尔 / `null` 统一转字符串；并把「无法解析的字符串静默归零」固化为已知取舍（`types.rs`，`7097890`）。其余子项按原评估继续搁置（`visible_rows` 需先抽自由函数，`app/*` 页面不追覆盖率）

- [x] P2-8 缩略图纹理降采样 + 上限淘汰（`65fd939`，方向取「并用」）：`Cmd::LoadThumbnail` 带上 `max_edge`（UI 按最大卡片 160 × 0.85 × `pixels_per_point` 算出，钳 128–512），worker 解码后经 `fit_within_max_edge` 缩到最长边以内再上传 GPU —— 服务端实测 720×405（RGBA ≈ 1.17 MB/张）降到 136×76（≈ 41 KB）～272×153（≈ 166 KB）；`thumbnail_textures` 换成新 `app/thumbs.rs` 的 `ThumbTextures`——64 MiB / 512 张**双上限**（同 `cache.rs` 语义）+ 2s 宽限期，网格每帧 `mark_used` 可见 ± 一屏、绘制后 `evict`，视野内不会被淘汰后立刻重解码，淘汰项滚回时命中磁盘缓存重解码、不走网络。卡片尺寸常量（`GRID_CARD_MIN/MAX`、`THUMB_MAX_CARD_RATIO`）单点化消除数字漂移；新增单测 12 项（纹理 LRU 8 + `thumb_max_edge` 1 + `fit_within_max_edge` 3）

- [x] P2-5 对话框起始目录记忆（`5281820`）：新增持久化字段 `Settings.last_dir`，上传文件 / 上传文件夹两个原生选择框改从「上次用过的目录」起（`helpers::picker_start_dir`：记忆目录仍存在 → 其本身，否则 `home_dir` → 进程 CWD），选择确认后回写（`helpers::picked_dir`：目录选择记其自身、文件多选记首个文件的父目录）；下载目录框沿用原有 `download_dir` 记忆，两者互不干扰。**原描述按实际口径修正**：新建 / 重命名是应用内纯文本弹框、没有本地目录参数，下载目录框也从未从 CWD 起（一直用 `download_dir` 兜底），真正用了 `std::env::current_dir()` 的只有上传的两个弹框。新增单测 3 项（记忆目录命中 / 失效与空值回退 / 记自身与父目录的推导）

- [x] P3-1 发行包（AppImage / Flatpak）+ 发布 CI —— **rpm 不做**：
  
  - **AppImage**：`packaging/build-appimage.sh` 构建 release → 组装 AppDir（二进制 / desktop / hicolor 图标 / 根目录同名 PNG 与 `.DirIcon`）→ 固定版 appimagetool（1.9.1，SHA256 写死、缺失时自动下载到 `~/.cache/kichi-packaging`）出包，入口为 `packaging/appimage/AppRun`。踩坑：appimagetool 1.9 只在 AppDir **根目录**找 `.desktop`、且必须是普通文件；光栅化器改为逐个试并校验产物非空（`ksvgtopng` 缺输出目录 / `magick` 参数顺序不对会「退出码 0 但没产物」）
  - **Flatpak**：`packaging/flatpak/io.github.lyndon0na.Kichi.yml`（freedesktop 25.08 Platform/Sdk + `rust-stable` 扩展，沙箱内源码构建；构建期网络靠 `build-options.build-args: [--share=network]` 放行）与 `packaging/build-flatpak.sh`（可选 `--install` 装入用户级 flatpak）
  - **沙箱适配**：播放经 `flatpak-spawn --host mpv` 借宿主 mpv（`helpers::host_command`，沙箱外形态不变）；中文字体候选改为「挂载根 × 相对路径」两维展开，加扫 flatpak 挂进来的 `/run/host/fonts`；`app_id` 取 `FLATPAK_ID` 以关联 `<应用 ID>.desktop`；`finish-args` 给 home 访问、`kdeglobals` 只读、Secret Service 与 Flatpak 桥接权限
  - **CI**：`.github/workflows/release.yml` —— AppImage 在 `ubuntu-22.04` 构建（glibc 2.35 门槛）、Flatpak 在 `ubuntu-24.04`；推 `v*` tag 自动发 Release，`workflow_dispatch` 只上传 artifacts
  - **CI 首跑踩坑（图标导出校验）**：flatpak 导出时用宿主 gdk-pixbuf 校验图标，Ubuntu 24.04（gdk-pixbuf 2.42）需从 `librsvg2-common` 加载 SVG —— 首次发 `v1.0.0` 时编译安装都成功，导出 `io.github.lyndon0na.Kichi.svg` 报 `is not a valid icon: Format not recognized`；工作流 apt 列表补装该包（Fedora / gdk-pixbuf ≥ 2.44 已内置 SVG 加载器，本地不复现）
  - **CI 首跑踩坑（发布任务）**：`release` job 无 checkout，`gh` 定位不到仓库（`fatal: not a git repository …`），给该步骤补 `GH_REPO: ${{ github.repository }}`
  - 新增单测 2 项；本地实测 AppImage 解包启动正常、Flatpak 安装后沙箱内无缺失库、宿主字体 / `kdeglobals` 可见、mpv 桥接可用、GUI 起窗正常（`helpers.rs` / `main.rs` / `packaging/` / `.github/workflows/release.yml`）

- [x] P3-3 补回仓库链接：`Cargo.toml` 的 `[workspace.package]` 补 `repository = "https://github.com/lyndon0na/kichi"`，两个 crate 用 `repository.workspace = true` 继承（`cargo metadata` 可直接读到，只写 workspace 层不会传导到包）；README 顶部徽章全部改为可点链接并新增动态徽章 —— 版本徽章换成 shields 的 GitHub release 徽章（`sort=semver`，首个 tag 发布前显示 "no releases"）、新增 Release 工作流状态徽章；免责声明末尾补 Issues 与 LICENSE 链接（`Cargo.toml` / `crates/*/Cargo.toml` / `README.md`）

- [x] P3-2 常规 CI（`3b0ea78`）：新增 `.github/workflows/ci.yml`（push `master` / PR 触发，同分支新推送取消旧跑）—— `fmt` 与 `clippy + test` 两个 job；clippy 走严格模式（`-- -D warnings`）、cargo 命令带 `--locked`（`Cargo.lock` 与 `Cargo.toml` 不一致直接失败）；缓存 key 用 `Linux-ci-cargo-*` 与 `release.yml` 区分（同 key 会互相覆盖，且 release 的 AppImage 在 `ubuntu-22.04` 构建、target 不通用）；README 顶部补 CI 状态徽章、目录树与「构建与运行」补 CI 说明（`.github/workflows/ci.yml` / `README.md`）

> [!TIP]
> P0–P2 与 P3-1 / P3-2 / P3-3 已完成；
