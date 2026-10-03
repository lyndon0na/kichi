# Kichi 开发任务清单

> [!NOTE]
> 本清单独立于 [`PROJECT_PLAN.md`](./PROJECT_PLAN.md)：`PROJECT_PLAN` 记录**已完成的里程碑与实现笔记**，这里只收敛**待办与改进项**，按优先级排序。
> 文件定位以「文件 + 关键符号」标注（不写行号，避免格式化后漂移）。每项完成后请同步回 `PROJECT_PLAN.md` 的「未实现 / 已知边界」与「路线图」。

## 进度总览

| 优先级    | 主题        | 项数  |
|:------ |:--------- |:--- |
| **P0** | 用户可见的功能缺口 | 0   |
| **P1** | 正确性与健壮性   | 0   |
| **P2** | 可维护性与工程   | 0   |
| **P3** | 分发与发布     | 0   |

> 上表只统计主线队列（P0–P3）；另有一节独立的「P2 · 结构优化」队列（第一批 P2-9 ~ P2-14 **全部完成** —— P2-9 / P2-10 / P2-11 / P2-12 / P2-13 已落地，P2-14 已由 P2-10 / P2-11 / P2-12 覆盖；第二批 P2-15 / P2-16 / P2-17 **已全部完成**），见下方专节。

---

## P0 · 用户可见的功能缺口

> ✅ 全部完成

| 编号   | 任务                       | 主要路径                                                                                                                        | 说明 / 验收                                                                                                                                                                                                                                                                                                                                 |
|:---- |:------------------------ |:--------------------------------------------------------------------------------------------------------------------------- |:-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P0-7 | 下载推送到 aria2（JSON-RPC 外部下载器） | 新增 `kichi-core/src/aria2.rs`、`worker/aria2.rs`；改 `msg.rs`、`settings.rs`、`worker/{mod,download}.rs`、`app/files/{mod,list,grid,row,toolbar}.rs`、`app/mod.rs`、`app/sidebar.rs`、`app/settings_page.rs` | **✅ 已完成**（`7d835da` core + `72b9fa5` GUI，2026-09-29）。范围：**手动推送 MVP + 含整目录**；不做「默认下载方式」切换、不在 Transfers 页跟踪进度。  

- **入口**：文件 / 目录右键菜单「发送到 aria2」（与「下载到本地…」并列）+ 选中工具栏次级按钮；支持单文件 / 多选 / 整个目录（`walk_folder` 后逐文件推，`dir` 承载云端目录层级、`out` 用文件名）；仅设置页启用后出现。
- **设置页**「aria2」卡片：启用开关、RPC 地址（默认 `http://127.0.0.1:6800/jsonrpc`）、密钥、下载目录（留空 = 读 aria2 的 `getGlobalOption.dir`）、「测试连接」（`aria2.getVersion`）；改动即持久化。
- **推送管线**（`worker/aria2.rs`，`tokio::spawn`，**不占**下载 Gate）：`file_download_link` → `stream_headers` → `aria2.addUri(url, {out, dir, header})`；点击即常驻提示条、结束替换为结果（照 P0-5 的 `toast_sticky`），进度消息 150ms 节流（照 `preview_download`），推送并发走独立小闸（固定 4）。`kichi-core/src/aria2.rs` 只做 JSON-RPC（`version` / `add_uri` / `get_global_option`），请求体构造与响应解析为纯函数。密钥只随命令下发、只存 `settings.json`，不进日志。
- **实验结论**（实现期已各做一次最小实验，2026-09-29）：① per-download `user-agent` 选项与 `header` 数组都能覆盖 aria2 全局 UA —— 取 `header` 数组，一次承载 UA / `X-Device-Id` / 必要时 Bearer；② `dir` 指向不存在的多级目录时 aria2 会自行创建，推送前无需本地 `create_dir_all`。另实测发现 aria2 对 JSON-RPC 错误也返回 HTTP 400 + 标准错误体，客户端改为「先按 JSON-RPC 解析、再回退状态码错误」，使「aria2 错误(1): Unauthorized」直达用户。
- **已知代价**（已同步 README / PROJECT_PLAN 的已知边界）：① 直链限时 —— 推送后由 aria2 自行重试同一链接，排队久 / 下载慢会过期失败、需重新推送，不做 Kichi 代理中转；② 同名冲突交给 aria2 自身策略（随其 `auto-file-renaming` / `allow-overwrite` 配置，可能自动改名或直接报错），如实回传错误，不复制内置「(n)」占位逻辑。
- **实测留痕**（2026-09-29）：① 独立靶子 `aria2c --no-conf --enable-rpc --rpc-listen-port=6800 --rpc-secret=test`：用真实 `Aria2Client` 实推 —— `getVersion` 返 1.37.0、`getGlobalOption.dir` 读取正确、`addUri` 带 `dir`（多级不存在的目录 + 非 ASCII 文件名）+ `out` + `header` 落盘正确，靶子收到的 User-Agent 为自定义值（证明覆盖全局 UA）、`X-Device-Id` 透传；错误密钥 → 「aria2 错误(1): Unauthorized」，端口不可达 → 「无法连接 aria2: …」。② 闸门：`cargo fmt --all -- --check` 零差异、`cargo clippy --workspace --all-targets -- -D warnings` 0 告警、`cargo test --workspace` **106 项全绿**（core 41 + GUI 65，较此前基线 96 增 10）。③ 推真实 PikPak 直链 + Motrix（16800）的端到端待维护者实机确认。

## P1 · 正确性与健壮性

> ✅ 全部完成

| 编号   | 任务           | 主要路径                                   | 说明 / 验收                                                                                                                                                                                                                                                                                                                           |
|:---- |:------------ |:-------------------------------------- |:----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1-5 | HTTP 请求超时 | `kichi-core/src/client.rs`（`KichiClient`） | **✅ 已完成**（`0e68f83`，2026-10-03）：共享 client 加 `connect_timeout(10s)`（连接阶段含 TLS 握手，SYN 黑洞不再无限等待）；控制类小请求（`request_inner` / 续期 / captcha / 直链探测）加 30s **per-request** 总超时；**传输体（本地下载 / OSS 分片 / 缩略图）刻意不设总超时** —— client 级总超时是「到响应体读完」的整体时限，会把慢速大传输直接杀掉。新增 2 项本地服务器单测（静默服务器下控制请求限时 timeout / 慢速分片下载跨过控制超时仍完整成功，防「改成 client 级总超时」回归）+ 1 项 `#[ignore]` 黑洞冒烟（`cargo test -p kichi-core blackhole -- --ignored --nocapture`）。实测：本机 `10.255.255.1` 经透明代理（Clash TUN）被立刻接成静默连接（tarpit，非 SYN 黑洞），由请求总超时兜底 —— 注入短值（连接 2s / 请求 5s）复核 **5.0s 返回明确错误**「http 请求失败: …」；闸门 `fmt` 零差异 / clippy 0 告警 / **111 项单测全绿**（core 42→44）。维护者实机复核路径：直连 / 代理两种网络形态下各跑一次上述冒烟 |

> P1-1 ~ P1-5 均已完成；P1-5（HTTP 请求超时）为 2026-10-03 新立项（来源：上传跨重启续传探针的边界观察，实现续传时按「另立条目」处理，未顺手改客户端），同日完成。

## P2 · 可维护性与工程

> ✅ 全部完成

## P3 · 分发与发布

> ✅ 全部完成

| 编号   | 任务                                | 主要路径        | 说明 / 验收                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
|:---- |:--------------------------------- |:----------- |:------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| P3-4 | `packaging/` 目录归位 | `packaging/` | **✅ 已完成**（`8bbb355`，随 v1.0.1 出包）。`build-appimage.sh` 收进 `packaging/appimage/`、`build-flatpak.sh` 收进 `packaging/flatpak/`，`kichi.desktop` 留根（AppImage / Flatpak 共用）；脚本内只改 `ROOT` 计算（多退一级到仓库根），其余 `$ROOT/...` 引用零改动；`release.yml` 两处调用路径、README 目录树与命令、Flatpak 清单头注释同步。本地验收：AppImage 完整出包 + 解包 `AppRun` 启动正常（恢复登录态、运行至 timeout）；Flatpak 完整出包成功（沙箱内 release 构建 45.91s） |

---

## P2 · 结构优化：巨型文件与 god object（第一批 P2-9 ~ P2-14 ✅ 已完成 · 第二批 P2-15 ~ P2-17 ✅ 已完成）

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

### 第二批（2026-09-24 立项；P2-15 / P2-16 / P2-17 已完成）

> 第一批（P2-9 ~ P2-14）收尾后的复核结论：主线队列已清空，但按判据 ② / ③ 还剩三处同类尾巴。三条互不依赖、可各自独立回滚，建议顺序 P2-15 → P2-16 → P2-17（P2-17 收益最低，可后置）。外部结构建议（`worker/handlers/` 与 `app/state/` 桶目录）经复核**不采纳**：其目标与本轮一致，但目录形态是夹缝层（会把下载 / 上传两条管线捆回一个文件、把文件页与预览的状态从各自域里再切出去），违背判据 ② / ③ —— 完整理由见 `PROJECT_PLAN.md` 第五节 11)。

| 编号    | 任务                                             | 主要路径                                            | 说明 / 验收                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
|:------ |:---------------------------------------------- |:----------------------------------------------- |:------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P2-15  | tasks 域收口（唯一还挂 `impl App` 的带状态页面）              | `app/tasks.rs` + `app/tasks_page.rs` → `app/tasks/` | **✅ 已完成**（T1 `88651aa` · T2 `e0ac2c0` · T3 `90f7089`）。`app/tasks.rs`（317 行）与 `app/tasks_page.rs`（643 行，唯一还挂 `impl App` 的带状态页面）收进 `app/tasks/`：`mod.rs::TasksPage` 持状态 / 生命周期 + `TasksAction`，`list.rs` 收页壳（标题 / 新建表单）/ 阶段页签 / 批量操作条 / 任务列表 + 「加载更多」，`card.rs` 收 `task_card`，`picker.rs` 收「保存到」目录选择器（状态操作 + 弹窗渲染）。渲染与动作分离：`TasksPage::show(...) -> Vec<TasksAction>` 只读写自身状态并返回动作，唯一跨域调用「下载到本地」改为 `TasksAction::Download`，由 `App::apply_tasks_action` 在本帧渲染后执行，页面不再持 `&mut App`；其余 `Cmd`（提交 / 刷新 / 重试 / 删除 / 加载更多）仍在域内经 `Global` 发出。自动轮询本就由 worker 按 `tasks_active` 自适应推进，UI 侧无落点需搬。每步纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / **93 项单测**全绿（core 33 + GUI 60，各步不减）；T2 逐词归一化比对旧新 `show` 主体，差异全为签名 / `self.tasks.` / `&mut self.global` 前缀去除与动作外提；T3 的 17 个函数体逐一比对 `ALL_MATCH`（仅可见性关键字差异）。实机冒烟待确认（维护者） |
| P2-16  | `app/types.rs` 类型随域归位                          | `app/types.rs` → 各域文件 + `app/global.rs`         | **✅ 已完成**（C1 `dfe5c1c` · C2 `0b50da6` · C3 `5015bfb` · C4 `a7586e1` · C5 `d6e583e` · C6 `6f2a40f`）。`app/types.rs`（506 行）清空删除，类型随域归位：`Page` / `CacheUsage` → `app/global.rs`（跨域共享）；`SortBy` / `Crumb` / `DirEntry` / `RowAction` / `ViewMode` / `ClipKind` / `Clipboard` / `ColDrag` → `app/files/mod.rs`；`QualityReady` / `QualityMenuState` / `PendingOpen` / `PreviewConfirm` / `PreviewProgress` → `app/preview.rs`；`ShareResult` → `app/shares.rs`；`OfflineTab` / `TaskOp` / `TaskSel` → `app/tasks/mod.rs`；`TransferTab` / `DlSel` → `app/transfers/mod.rs`，`DlStatus` / `DlNode` / `DlJob` / `DlOp` / `DlRow` / `DlFilter` → `app/transfers/download.rs`，`UlStatus` / `UlJob` / `UlOp` / `UlFilter` / `UploadPick` → `app/transfers/upload.rs`（`transfers/mod.rs` 以 `pub(crate) use` 重导出 `DlStatus` 供 sidebar 角标使用）。**按实况修正**：`TransferTab` 实测仅 `app/transfers/mod.rs` 引用（并非跨域），未按原计划留 `global.rs`，归入传输域。纯机械搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / **93 项单测**全绿（core 33 + GUI 60，各步不减）；用脚本取出改动前的 `app/types.rs` 全部 457 行非导入内容，逐行在当前 `app/` 树中 `grep -F` 命中（`missing=0`），确认无丢行 / 改行。可见性保持原 `pub(crate)` 未顺手收紧（纯搬移优先）。实机冒烟待确认（维护者）                  |
| P2-17  | `app/shares.rs` 渲染按角色分文件（可后置）                  | `app/shares.rs` → `app/shares/{mod,mine,restore}.rs` | **✅ 已完成**（`66614b8`）。`app/shares.rs`（1608 行，GUI 最大）拆为 `app/shares/`：`mod.rs`（409 行）持 `SharesPage` 状态 / 生命周期 / 消息处理 + `ShareResult`；`mine.rs`（634 行）收我的分享列表 + 创建分享 / 分享结果 / 取消分享确认弹窗；`restore.rs`（597 行）收转存分享（链接解析 / 文件浏览 / 「保存到」目标目录选择器 / 保存后自动移动与重试）。**原计划「`mod.rs` 含页壳」按实况修正**：该域没有独立页壳（`draw` 即我的分享页本身），故 `draw` / `draw_dialogs` 一并归入 `mine.rs`，`mod.rs` 只留状态与生命周期。不动结构与 `App` 字段；三个渲染方法可见性由 `pub(super)` 放宽为 `pub(crate)`（调用点 `app::sidebar` / `app::dialogs`）。纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / **93 项单测**全绿（core 33 + GUI 60，不减）；归一化脚本比对旧新全部 1348 行非导入内容，唯一差异是 `// ---------- 渲染 ----------` 分隔注释（分文件后不再适用）。实机冒烟待确认（维护者） |

### 每步的固定验收

- `cargo fmt --all` → `cargo clippy --workspace --all-targets -- -D warnings`（0 告警）→ `cargo test --workspace` 全绿，`#[test]` 数不减（当前基线 **109 项**：core 42 + GUI 67）。
- 一次一个域、独立提交（`refactor(gui): …`），**纯搬移、零行为变更**；`worker/` 与上传 / 下载管线额外实机验证。
- 路径变了就必须同步 `README.md` 目录树与 `AGENTS.md` 代码地图，并在 `PROJECT_PLAN.md` 第五节 11) 追加进度。

---

## 已完成（本轮）

- [x] P1-5 HTTP 请求超时（`0e68f83`）：`KichiClient` 此前未配任何超时，连接挂起 / 黑洞地址下请求永久等待；现在连接阶段（含 TLS 握手）限时 10s、控制类小请求（API JSON / captcha / 续期 / 直链探测）30s 总超时，传输体（下载 / OSS 分片 / 缩略图）刻意不设总超时。新增 2 项本地服务器单测 + 1 项 `#[ignore]` 黑洞冒烟（`cargo test -p kichi-core blackhole -- --ignored --nocapture`）

- [x] 文档补漏：README「已知限制」补「上传跨重启续传不可行」条目（此前只写在 `AGENTS.md` / `PROJECT_PLAN.md` / `TODO.md`），功能表与上传详情的「断点续传」限定为「运行期内」（该两处表述已在 P2-1 落地后改回「跨重启续传」，见下方 P2-1）

- [x] 工作区清理与文档归位：删除 `.delta/` 旧快照残留与 `dist/` 的 0.1.0 旧产物（均为忽略文件），根 `.gitignore` 补 `.flatpak-builder/`；`PROJECT_PLAN` / `TODO` / `UI_STYLE` / `UPLOAD_RESUME_NOTES` 移入 `docs/`（本地 `VIBE_CODING_NOTES` 一并归位），`AGENTS.md` 纳入版本管理，README 目录树与各处引用同步

- [x] 文档失真修正：离线任务翻页说明、README 目录树补 `lib.rs` / `logging.rs`（`9c66bfa`）

- [x] 工程杂项：`LICENSE`、`CHANGELOG.md`、`rustfmt.toml`，移除占位 `repository`，版本号去硬编码，`.gitignore` 扩充，全仓库 `cargo fmt`（`d6044a3`）

- [x] P0-1 离线任务「加载更多」分页；顺带把离线任务 / 配额的自动轮询改为自适应节拍（`worker.rs` / `tasks_page.rs` / `app/mod.rs` / `msg.rs`）

- [x] P0-2 整目录递归下载：`kichi-core::walk_folder` 递归遍历 + 本地按云端层级建目录；传输任务页聚合为单张目录卡片（显示「文件 已完成/全部」），展开后按层级显示目录树、子目录可单独折叠；重试仅重下失败子文件（`client.rs` / `worker.rs` / `app/mod.rs` / `files_page.rs` / `transfers_page.rs` / `settings.rs` / `msg.rs`）

- [x] P0-3 真实缩略图：网格视图加载 `thumbnail_link` 预签名直链，worker 下载 + 磁盘缓存（`~/.cache/kichi/thumbnails/`）+ `image` crate 解码为 RGBA 后上传 GPU 纹理；保持宽高比显示；Ctrl + 滚轮缩放网格大小（80–160px），图标 / 文字 / 缩略图随卡片等比缩放（`client.rs` / `worker.rs` / `msg.rs` / `app/mod.rs` / `files_page.rs`）

- [x] P0-4 全局搜索：PikPak 无服务端搜索 API，采用客户端递归遍历所有目录并按文件名模糊匹配；搜索框按 Enter 触发搜索，支持分页加载更多；搜索模式下显示搜索结果指示器，导航/面包屑点击自动退出搜索模式（`client.rs` / `msg.rs` / `worker.rs` / `app/mod.rs` / `files_page.rs`）

- [x] P0-5 预览等待提示常驻：点击预览后的「正在解析播放地址…」/「正在准备预览文件…」不再 6 秒自动消失，改为常驻直到结果消息到达被成功 / 失败提示替换（媒体与非媒体两条路径，非媒体下载期间与进度条并存）；提示条新增 `Toast::sticky` 常驻形态（`Global::toast_sticky` / `clear_sticky_toast`，会话失效 / 登出时随预览状态一并收回），普通提示维持 6 秒超时；附 Toast 超时判定单测（`global.rs` / `dialogs.rs` / `preview.rs` / `mod.rs`，`4c5a450`）

- [x] P0-6 mpv 就绪检测：预览提示的切换判据由「spawn 成功」改为 mpv IPC 观察 `vo-configured`（spawn 时带 `--input-ipc-server` + 后台观察线程 `helpers.rs::watch_mpv`），窗口 / VO 配置完成才换成功提示；解析完成后提示先转「正在唤起 mpv…」并保持常驻；mpv 在窗口出现前退出（坏直链 / 断网）如实告警「mpv 已退出，未能播放「x」」，IPC 不可用（连接失败 / 老版本无该属性）按旧行为兜底。实测依据（mpv 0.41 / Wayland）：本地图片 0.31s 翻 true、坏链接全程 false、`--force-window=immediate` 反证 0.14s 即 true；新增单测 2 项（IPC 行解析 + Unix socket 扮演 mpv 走通三态），96 项全绿（`helpers.rs` / `preview.rs` / `mod.rs`，`abdfa2a`）

- [x] P1-1 分享转存目标目录改为持久化 ID：新增 `settings::load_pack_folder_id` / `save_pack_folder_id`（独立文件 `~/.config/kichi/pack_folder_id`，避免与 GUI 线程覆写 settings.json 竞争）；`worker.rs` 新增 `find_pack_folder` 统一按持久化 ID 定位「转存自分享」暂存目录，ID 失效时回退名称匹配并刷新缓存，`snapshot_pack_folder` / `move_new_files` / `Cmd::RetryMoveShare` 全部改用该入口（`worker.rs` / `settings.rs`）

- [x] P1-2 分享链接解析增强：`extract_share_id` 改为 `parse_share_input`，支持带查询参数 / 片段 / 复制链接附带前后文字的形态，ID 截到首个非法字符为止，并顺带从 `password`/`pass_code` 回填提取码；无法识别（缺 `/s/` 的其它链接、非法字符、空）时返回 `None`，解析对话框给出错误提示而非当成裸 ID；补 `parse_share_id_from_url_forms` 单测（`mod.rs` / `dialogs.rs`）

- [x] P1-3「打开下载目录」空路径修复：系统下载目录取不到时不再回退成空路径导致按钮无反应，改为回退到已记住的下载目录，两者都无效时给出 toast 提示前往设置选择（`transfers_page.rs`）

- [x] P1-4 人机验证流程：`captcha_init` 取不到 `captcha_token` 时改为返回新错误变体 `Error::CaptchaReview`（携带从响应里递归提取的验证页链接 `data.url`/`*url`）；`Msg::LoginFailed` 增加 `verify_url` 字段并透传到 `App::auth_captcha_url`；登录页在需要验证时显示「打开验证页面」按钮（`helpers::open_url`）+ 完成验证后重试的引导，无链接时给出「稍后重试 / 换网络 / 用官方客户端验证」提示；补 `extract_verify_url` 单测（`error.rs` / `client.rs` / `msg.rs` / `worker.rs` / `app/mod.rs` / `login.rs`）

- [x] P2-1 上传跨重启续传（`8637660` core + `dadd907` GUI）—— **旧「不可行」结论已被探针实机复核推翻，并已实现**。不再走「重启后重取票续旧 upload_id」（该路线仍被 STS policy 挡死：凭证按对象 key 授权，403 AccessDenied），改为**落盘 STS 凭证本身**：在凭证有效期（实测 = 取票时刻 + 12 小时，样本 ×3 秒级吻合）内**免取票**，直接用旧 key / 旧 upload_id / 旧凭证续传；过期或本地文件指纹（size + mtime）不符时安全降级为全量重传。探针实测：跨进程 ×4 + 跨重启 ×1 全过、回下载 gcid 与本地逐字节一致；不可见的 `PENDING` 占位条目也可列出 / 删除（`phase.eq` 过滤 + `batch_trash` / `batch_delete`，`phase.in` 数组形式会被服务端拒）。实现：`settings.rs` 落盘 `upload_resume.json`（0600 原子写、worker 单写、不落日志）+ `worker/upload.rs` 续传管线（命中免 gcid / 免取票；每片 ETag 节流 1s 落盘；取消 / 移除时清记录并后台清理占位条目）+ 传输页把记录还原成排队卡片、登录后自动分发。新增单测 3 项（RFC3339 解析 1 + 续传记录判据 2）。结论与回归方法：`docs/UPLOAD_RESUME_PROBE_REPORT.md`（探针随仓库保留为手动回归工具）；历史复盘：`docs/UPLOAD_RESUME_NOTES.md`。

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

- [x] 移除 `packaging/install-icon.sh`：桌面文件与图标改由发行包自带（AppImage 打进 AppDir、Flatpak 由清单导出），脚本只服务「源码构建后直接跑二进制」一条路径；README「桌面图标」一节并入打包说明，需要时按说明手工放置 `packaging/kichi.desktop` 与 `assets/kichi.svg`

- [x] P3-4 打包目录归位（`8bbb355`，随 v1.0.1 出包）：`build-appimage.sh` 收进 `packaging/appimage/`、`build-flatpak.sh` 收进 `packaging/flatpak/`（`kichi.desktop` 留根），`release.yml` / README / 清单注释同步；本地完整跑通 AppImage 出包 + 解包启动、Flatpak 出包

> [!TIP]
> P0–P3 此前已全部完成；结构优化第二批的 P2-15（tasks 域收口）/ P2-16（类型随域归位）/ P2-17（分享域渲染分文件）已全部完成；P3-4（打包目录归位）已随 v1.0.1 出包完成；P0-7（下载推送到 aria2）已完成（`7d835da` + `72b9fa5`，端到端待维护者实机确认）；P2-1（上传跨重启续传）经探针复核后已实现（`8637660` + `dadd907`）；**P1-5（HTTP 请求超时）已完成**（`0e68f83`）—— 主线队列已清空。
