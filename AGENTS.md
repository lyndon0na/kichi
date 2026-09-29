# AGENTS.md — Kichi 开发须知

> 写给在本仓库工作的 AI 助手（Qoder / Claude Code / Cursor / Windsurf 等）与协作者。
> 动手前先读本文件；若发现内容与代码实况不符，**以代码为准，并顺手修正本文件**。
> 本文件只讲「怎么在这里干活」；功能与用法看 `README.md`，历史与实现笔记看 `docs/PROJECT_PLAN.md`。

Kichi 是 [PikPak](https://mypikpak.com) 网盘的非官方 Linux 桌面客户端（Rust + egui/eframe，单一二进制，另出 AppImage / Flatpak 发行包）。接口与加签算法来自社区逆向成果，仅供学习与个人使用。

| 想知道什么                            | 看哪里                           |
|:-------------------------------- |:------------------------------ |
| 功能说明、构建步骤、已知限制（用户视角）             | `README.md`                   |
| 里程碑（M0–M24）、实现笔记、已实现清单           | `docs/PROJECT_PLAN.md`        |
| **当前待办**（唯一队列，P0–P3）             | `docs/TODO.md`                |
| 版本变更记录（Keep a Changelog + 语义化版本） | `CHANGELOG.md`                |
| UI 风格规范（KDE / Breeze：颜色 / 间距 / 控件 / 清单） | `docs/UI_STYLE.md`            |
| 一次失败实验的完整复盘（上传跨重启续传）             | `docs/UPLOAD_RESUME_NOTES.md` |
| 工程结构评审与改进方向（拆分、CI、协作习惯）          | `docs/VIBE_CODING_NOTES.md`   |

## 常用命令

```bash
cargo run -p kichi-gui                 # 开发运行（需要图形环境）
cargo build --release -p kichi-gui     # 发布构建 → target/release/kichi-gui
cargo test --workspace                 # 全部单测（kichi-core 纯逻辑 + GUI 纯函数）
cargo fmt --all                        # 格式化（rustfmt.toml 全默认 + edition 2021）
cargo clippy --workspace --all-targets -- -D warnings   # 必须 0 告警
./packaging/appimage/build-appimage.sh          # → dist/Kichi-<版本>-x86_64.AppImage
./packaging/flatpak/build-flatpak.sh [--install]   # → dist/Kichi-<版本>-x86_64.flatpak
```

- 系统依赖（Fedora）：`sudo dnf install gcc pkgconf openssl-devel libxkbcommon-devel wayland-devel mesa-libGL mesa-libEGL fontconfig`；运行还需 `mpv`（音视频预览）与一款中文 TTF 字体。
- 本机环境：Fedora 44 / KDE Plasma / Wayland，rustc 1.98.1（由根 `rust-toolchain.toml` 固定，CI 同步）；无 Docker / Go（用不到），`sudo` 需密码 —— 装系统包前先征得维护者同意。
- 调日志：`tail -f ~/.cache/kichi/kichi.log`；级别用 `KICHI_LOG`（如 `KICHI_LOG=kichi_gui=trace`），配置 / 缓存在 `~/.config/kichi/` 与 `~/.cache/kichi/`。

## 硬约定

1. **注释、文档、提交信息一律中文**。提交用约定式前缀 `feat / fix / refactor / perf / docs / test / chore`（可带 scope：`gui` / `core` / `ci` / `packaging`），说清「做了什么、为什么」，例如 `fix(gui): 打开文件不再假成功, 无关联程序时给出明确提示`。
2. **文档四处同步**：行为 / 功能 / 工程变更落地必须同步 `docs/TODO.md`（状态）、`docs/PROJECT_PLAN.md`（里程碑 / 已实现 / 已知边界）、`README.md`（用户可见行为）、`CHANGELOG.md`（`[Unreleased]` 段）。漏同步视为未完成；TODO 条目完成后要补**实现提交的短哈希**（docs 提交不算）。
3. **文件定位写「文件 + 符号」不写行号**（如 `worker/mod.rs::handle`、`msg.rs::Cmd`）—— 格式化后行号会漂移。
4. **版本号单点**：只改 `Cargo.toml` 的 `[workspace.package]`，两个 crate 继承。发版 = 更新 CHANGELOG + 版本号 → 推 `v*` tag → `release.yml` 自动出包发 Release。
5. **小步提交**：一次一个变更、独立可回滚；不夹带无关改动、不改写已推送历史（不 force push）。
6. **提交前过闸门**：`cargo fmt --all` + `cargo clippy --workspace --all-targets -- -D warnings`（当前 0 告警，保持）+ `cargo test --workspace` 全绿。不要提交「测试待补」的半成品。
7. **测试跟着纯逻辑走**：新逻辑尽量抽成纯函数并补 `#[test]`；`app/*` 页面渲染不追覆盖率。网络 / D-Bus / 密钥环 / OSS 这类单测覆盖不到的路径，必须提示维护者实机验证。
8. **UI 风格统一 KDE / Breeze（Dolphin）**：新界面与交互默认照 KDE 观感做 —— 扁平、克制、信息密度高：控件与菜单项**常态无边框**（不要「框里套框」），仅悬停 / 选中时整行填充高亮；列表平铺不套多余卡片；间距紧凑。**完整规范见 `docs/UI_STYLE.md`**（颜色 / 圆角 / 间距 / 图标 / 各控件与检查清单），落地参考 `theme.rs` 的 Breeze 参数、`app/files/toolbar.rs::popup_menu`。与既有风格冲突时以 KDE 观感为准，并在实机确认后再定稿。

## 架构与线程模型

```text
UI 主线程 (egui) ←── mpsc：Cmd / Msg ──→ worker 线程 (tokio) ──→ kichi-core (HTTPS) ──→ PikPak API / Aliyun OSS
```

- UI 主线程只做渲染与状态收敛（每帧 `try_recv`）；**禁止**在 UI 线程做网络、阻塞 IO 或整文件操作。
- 新后端能力加在 `kichi-core`（纯 API、无 UI 依赖），经 `msg.rs` 的 `Cmd` / `Msg` 暴露给 UI；协议改动两侧一起改。
- 长任务（下载 / 上传 / 预览 / 缩略图）在 worker 里 `tokio::spawn` + 并发闸，不允许占住命令循环（`worker/mod.rs::handle`）。
- 并发 / 重试参数是**用户可配**的（`settings.rs` + `Cmd::SetTransferLimits`），不要新增编译期写死的传输常量。

## 代码地图

**kichi-core**（`crates/kichi-core/src/`，无 UI 依赖，可复用）：

| 文件            | 职责                                                                                                                                                        |
|:------------- |:--------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `client.rs`   | `KichiClient`：全部 API 与鉴权。自动 refresh（error_code 16）/ 自动 captcha 重试（error_code 9），各只重试一次；文件列表、离线任务、分享、回收站、直链、`walk_folder`（整目录下载用）、`search_files`（客户端递归，见下） |
| `captcha.rs`  | `captcha_sign` / `device_sign` 加签算法                                                                                                                       |
| `consts.rs`   | client_id / host / 盐值表 / 分页常量                                                                                                                             |
| `session.rs`  | 会话（token / device id）持久化                                                                                                                                  |
| `types.rs`    | `File` / `Quota` / `Task` / `Share` 等模型，防御式 serde（`de_number` / `de_string`）                                                                              |
| `download.rs` | 直链解析、`.part` + Range 断点续传、完整性校验                                                                                                                           |
| `aria2.rs`    | aria2 JSON-RPC 客户端（推送到外部下载器）：`getVersion` / `addUri` / `getGlobalOption`；请求体构造与响应解析为纯函数                                                              |
| `upload.rs`   | gcid 秒传哈希、阿里云 OSS HMAC-SHA1 分片签名与上传                                                                                                                       |
| `error.rs`    | 统一错误类型（`CaptchaReview` 携带人机验证页链接）                                                                                                                         |

**kichi-gui**（`crates/kichi-gui/src/`）：

| 文件                                                         | 职责                                                                                             |
|:---------------------------------------------------------- |:---------------------------------------------------------------------------------------------- |
| `main.rs`                                                  | 入口；窗口 `app_id`（Flatpak 内取 `FLATPAK_ID`）与视图尺寸                                                   |
| `worker/mod.rs`                                            | 后台 tokio 线程：`handle` 命令循环（45 个 `Cmd` 分支各一行转调）+ 跨域辅助（`cancel_task` / `set_transfer_limits` / `refresh_quota` / `download_backoff` / `discard_part`）；⚠️ 勿再堆新逻辑 |
| `worker/{download,upload,preview,thumbs}.rs`               | 四条长任务管线：并发闸 + 退避重试 + 取消登记（**敏感区**：下载 / 上传续传核心路径）                                                                 |
| `worker/aria2.rs`                                          | aria2 推送管线（P0-7）：整目录 `walk_folder` → 逐文件解析直链 → `aria2.addUri`；独立小闸 4、进度 150ms 节流；不占下载 Gate、不传字节 |
| `worker/{files,tasks,shares,trash,auth}.rs`                | 各域请求处理：文件列表 / 搜索 / 新建重命名移动复制 · 离线任务 · 分享转存 · 回收站 · 登录会话 —— `Cmd` 分支逐条转到这些函数                                        |
| `worker/{gate,cache}.rs`                                   | 动态并发闸 `Gate` / 磁盘缓存淘汰 `CacheCtl`（预览与缩略图共用）                                                                     |
| `msg.rs`                                                   | `Cmd` / `Msg` 协议单点                                                                             |
| `app/mod.rs`                                               | `App` 状态、消息收敛与跨域编排、目录缓存（SWR）、传输调度与持久化（⚠️ 勿再堆新逻辑，各域状态已下沉到下方域文件；P2-9 ~ P2-17 结构优化已收尾）                                  |
| `app/global.rs`                                            | 各域共用的全局句柄 `Global`：命令通道 / 提示条 / KDE 配色与主题轮询 / `req_id` 分配器（下载 / 上传 / 预览共用同一命名空间，域方法收 `&mut Global`，不引入 `Rc`/`RefCell`）；另收跨域共享类型 `Page` / `CacheUsage`（P2-16）             |
| `app/{trash,shares,preview,search,thumbs}.rs`              | 五个域结构体（P2-10）：`TrashPage` / `SharesPage`（我的分享 + 转存分享）/ `PreviewPage`（预览进度 + 清晰度 + 外部打开探针）/ `SearchPage` / `ThumbsPage`（纹理 LRU + 在途 / 失败登记）；`drain` 臂一行转调，域内弹窗在各自文件 |
| `app/files/`                                               | 文件浏览域（P2-11）：`mod.rs::FilesPage` 持导航栈 / 目录缓存（SWR）/ 选中集 / 排序过滤 + 渲染入口 `show`；渲染按角色分 `list.rs`（列表）/ `grid.rs`（网格 + 缩略图预取）/ `row.rs`（行与右键菜单）/ `toolbar.rs`（顶部栏 + 面包屑）。文件域类型（`Crumb` / `RowAction` / `SortBy` / `ViewMode` / `DirEntry` / `Clipboard` / `ColDrag`）随域定义在 `mod.rs`。渲染与动作分离：`show` 返回 `Vec<FilesAction>`，由 `App::apply_files_action` 在本帧渲染后执行（页面不持 `&mut App`） |
| `app/transfers/`                                           | 传输任务域（P2-12）：`mod.rs::TransfersPage` 持任务表 / 选中集 / 筛选 / 展开 + 生命周期（入队 / 重试 / 移除 / 历史记录 / 目录扫描收敛）+ 渲染入口 `show`；渲染按分栏分 `download.rs`（任务卡片 / 目录树 / 底部批量操作条）/ `upload.rs`（任务卡片 / 底部批量操作条）；纯逻辑在 `model.rs`；下载 / 上传域类型分别在 `download.rs`（`Dl*`）/ `upload.rs`（`Ul*` / `UploadPick`），共享的 `TransferTab` / `DlSel` 在 `mod.rs`。渲染与动作分离：`show` 返回 `Vec<TransfersAction>`，由 `App::apply_transfers_action` 在本帧渲染后执行（页面不持 `&mut App`）；`last_dir` / `download_dir` 仍在 `App` |
| `app/tasks/`                                               | 离线任务域（P2-15）：`mod.rs::TasksPage` 持分桶 / 分页 / 选中 / 新建表单 + 生命周期，`show` 返回 `Vec<TasksAction>`（唯一跨域动作「下载到本地」由 `App::apply_tasks_action` 执行，页面不持 `&mut App`）；渲染按角色分 `list.rs`（页壳 / 阶段页签 / 批量条 / 列表 + 加载更多）/ `card.rs`（单卡）/ `picker.rs`（「保存到」目录选择器）；任务域类型（`OfflineTab` / `TaskOp` / `TaskSel`）定义在 `mod.rs` |
| `app/shares/` / `app/trash.rs`                           | 「我的分享 / 转存分享」页与回收站页（P2-17）：`SharesPage` 状态 / 生命周期 / 消息处理在 `shares/mod.rs`，渲染与弹窗按角色分家 —— `mine.rs`（我的分享列表 + 创建分享 / 分享结果 / 取消确认）、`restore.rs`（转存链接解析 / 文件浏览 / 目标目录选择器 / 保存与移动重试）；`TrashPage` 内部以 `TrashAction` 做渲染与动作分离；`ShareResult` 定义在 `shares/mod.rs` |
| `app/login.rs` / `app/sidebar.rs` / `app/settings_page.rs` | 登录页 / 侧边栏与配额卡 / 设置页                                                                            |
| `app/dialogs.rs`                                           | 通用弹窗：新建 / 重命名 / 移入回收站 / 退出登录 + 提示条（域内弹窗在各域文件）                                                 |
| `app/helpers.rs`                                           | 中文字体加载、原生选择框、`open_async`（xdg-open 探针）、mpv 播放、Flatpak 宿主命令（`host_command`）                     |
| `app/thumbs.rs`                                            | 缩略图域：GPU 纹理 LRU（64 MiB / 512 张软上限）+ 在途 / 失败登记（P2-10 D6）                                     |
| `app/transfers/model.rs`                                   | 传输纯逻辑：状态映射 / 进度聚合 / 目录树计数 / 速率取样 / 历史记录 id（无 UI 依赖，直接单测）                                             |
| `filetypes.rs`                                             | 文件类型分类**单点**：图标 / mpv / 系统打开 / 只下载共用一份表（强 mime 优先，扩展名兜底）                                       |
| `cache.rs`                                                 | 磁盘缓存淘汰：预览 / 缩略图共用的 mtime-LRU（软上限）                                                              |
| `settings.rs`                                              | 设置与上传 / 下载历史持久化（`~/.config/kichi/`）                                                            |
| `credentials.rs`                                           | 系统密钥环读写密码（Secret Service，阻塞式调用）                                                                |
| `logging.rs`                                               | 日志初始化与按大小轮转（`~/.cache/kichi/kichi.log`）                                                        |
| `format.rs` / `theme.rs` / `icons.rs` / `kde.rs`           | 文案格式化 / 主题参数 / 矢量图标 / KDE 配色读取                                                                 |

**工程与发布**：

| 路径                              | 职责                                                                                                       |
|:------------------------------- |:-------------------------------------------------------------------------------------------------------- |
| `packaging/`                    | 发行包：`kichi.desktop`（AppImage / Flatpak 共用）+ `appimage/`（`build-appimage.sh` + `AppRun`）+ `flatpak/`（`build-flatpak.sh` + 清单 `.yml`）        |
| `.github/workflows/release.yml` | 推 `v*` tag → 构建 AppImage（ubuntu-22.04）与 Flatpak（ubuntu-24.04）并发 Release；`workflow_dispatch` 只留 artifacts |

## 已知边界与已证伪方向（不要再踩）

- **上传跨重启续传：协议限制，不可行**。PikPak 的 OSS STS 凭证按对象 key 授权，而 key 每次 `upload_create` 都变（`upload_tmp/<GCID>_<时间戳>`），重启后重取票访问旧 `upload_id` 会被 `403 AccessDenied` 拒绝；曾实现的版本已整体回退 —— **不要重新实现**，除非有全新方案。会话内（同票同凭证）的退避重试是保留的。详见 `docs/UPLOAD_RESUME_NOTES.md`。
- **服务端无全局搜索 API**：`client.rs::search_files` 是客户端递归遍历全盘 + 文件名匹配，慢是已知问题（优化方向待定）。另有**未经复核**的线索称 `/drive/v1/files` 支持 `name` 查询参数做全局搜索（filters 里的 `name.contains` / `name.like` 会被服务端拒绝）—— 做搜索优化前先花几分钟实测这条线索，不要直接信。
- **`thumbnail_link` 有值 ≠ 可用**：服务端会对无缩略图的文件下发空串 / 相对路径，必须先判可用性（`worker/thumbs.rs::is_usable_thumb_url`）再请求，否则会刷失败日志。
- **分享转存落点固定**：`share/restore` 只能落进「转存自分享」暂存目录（无 `parent_id` 支持），需转存后定位暂存目录再移动（`worker/shares.rs::find_pack_folder` / `move_new_files`）；不要试图直接把文件转存到目标目录。
- **回收站列表必须 `parent_id=*`**（`client.rs::trash_list`），否则服务端按当前目录过滤，返回空列表。
- **Wayland 不支持拖拽上传**（winit 限制），不要尝试「修复」；入口只有「上传文件 / 上传文件夹」。
- **离线任务字段无契约**：按 `phase` 分桶（PENDING / RUNNING / COMPLETE / ERROR）请求，展示层防御式解析。
- **不要过度防御、不要过度抽象**：不为不可能发生的情况加兜底，不为一个用例造框架；声称「已修复」前必须有实测依据。

## 敏感区（改动需人工复核 + 实机验证）

| 面                                                 | 原因                                             |
|:------------------------------------------------- |:---------------------------------------------- |
| `client.rs` 认证 / 加签 / 重试恢复                        | 单测覆盖不到真实服务端；错了直接登录失败或请求全挂                      |
| `credentials.rs` / `session.rs` / `consts.rs` 盐值表 | D-Bus 密钥环与登录态，必须实机验证                           |
| 上传 / 下载续传核心路径（`worker/download.rs` / `worker/upload.rs` 管线）                   | 涉及数据完整性，改错会造成残缺文件或丢数据                          |
| 日志与错误信息                                           | 不得打印 token / 密码 / 完整签名 URL（用 `url_snippet` 截断） |

另外：**不要把真实账号密码、cookie、token 写进代码、日志、提交或与 AI 的对话**；调试用小号或脱敏样例。

## 与维护者协作

- 这是维护者的**首个 vibe coding 项目**，但其工程习惯成熟（计划 / 验收 / 实测留痕）—— 不做入门科普，按专业协作者对待。
- **先方案后动手**：非平凡改动（新功能、重构、协议相关）先给出方案、范围与验收标准，等确认再实现；沿用 `docs/TODO.md` 条目的「说明 / 验收」格式。
- **维护者会逐条读 diff**：改完先让他过目；大改动（如拆文件）拆成多个可独立回滚的提交，逐个验证。
- **服务端行为不要断言**：涉及 API 字段 / 错误码 / 限速的结论，先给假设 + 一条可执行的验证方式（命令或看哪行日志），由维护者实机确认后再写进文档。
- 完成报告给出**实际跑过的命令与输出**（`cargo test` / `cargo clippy` / 实机冒烟结果），不要只写「已完成」。
- **上下文防护**：“单个 Task 会话如果超过 15 分钟或进行了多次大改动，在完成当前 commit 后，**主动提醒维护者另开新会话（New Session）**。”
  - *原因：防止 AI 会话历史过长后注意力分散，把前面的好约束给“忘记”了。*
