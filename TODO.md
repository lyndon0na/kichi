# Kichi 开发任务清单

> [!NOTE]
> 本清单独立于 [`PROJECT_PLAN.md`](./PROJECT_PLAN.md)：`PROJECT_PLAN` 记录**已完成的里程碑与实现笔记**，这里只收敛**待办与改进项**，按优先级排序。
> 文件定位以「文件 + 关键符号」标注（不写行号，避免格式化后漂移）。每项完成后请同步回 `PROJECT_PLAN.md` 的「未实现 / 已知边界」与「路线图」。

## 进度总览

| 优先级 | 主题 | 项数 |
| :-- | :-- | :-- |
| **P0** | 用户可见的功能缺口 | 0 |
| **P1** | 正确性与健壮性 | 0 |
| **P2** | 可维护性与工程 | 4 |
| **P3** | 分发与发布 | 4 |

---

## P0 · 用户可见的功能缺口

> ✅ 全部完成

## P1 · 正确性与健壮性

> ✅ 全部完成

## P2 · 可维护性与工程

| 编号 | 任务 | 主要文件 | 说明 / 验收 |
| :-- | :-- | :-- | :-- |
| P2-4 | 补测试（**已修剪，暂缓**） | `crates/kichi-core/src/types.rs` | **原清单挑的是「纯逻辑」，而非「错了不报错的逻辑」—— 经评估后大幅收敛**：① ~~`extract_share_id`~~ 已由 P1-2 覆盖（改名 `parse_share_input` + 补 `parse_share_id_from_url_forms`，见「已完成」），删除；② **唯一保留项**：`de_number` / `de_string` 的 serde 往返测试（`serde_json::from_str::<File>()` / 配额结构体，而非直接调私有函数）—— 这两个防御式解析器的失败是**静默的**：字符串解析失败 `unwrap_or(0)`、`Float as i64` 截断、null → `0` / 空串，服务端若换一种返回形式（如 `size` 给成 `"1.2GB"`），文件大小 / 配额会悄悄错掉且无异常无日志，与本项目已踩过的「对服务端行为的未验证假设」属同类风险（参见 `UPLOAD_RESUME_NOTES.md`）；③ `unique_name` 边界（`a.tar.gz` → `a.tar (1).gz`、`.gitignore` → 走无扩展名分支、`name.`）随 worker 改动顺手补，不单开提交；④ `visible_rows` 排序 / 过滤与**文件类型分类的测试折叠进 P2-6**（类型表本就要收敛成单点，先测旧的双份表属白写；`visible_rows` 挂在 `App` 上、而 `App::new` 吃 `&eframe::CreationContext` 并会 `worker::spawn()`，**无法直接测**，需先抽成自由函数再补，抽函数本身才是收益）；⑤ **明确放弃** `app/*` 各页面整体覆盖（需搭 egui harness，成本高、回归价值低，属为覆盖率数字写测试）。判据：只测「破坏时静默」的逻辑，不追覆盖率 |
| P2-5 | 对话框起始目录记忆 | `crates/kichi-gui/src/app/mod.rs`（`std::env::current_dir()`） | 新建 / 重命名 / 选择目录对话框都从进程 CWD 起，应改用上次使用过的目录 |
| P2-6 | 预览入口范围分层 + 健壮性 | `crates/kichi-gui/src/app/helpers.rs`（`open_path` / `is_media_file`）<br>`crates/kichi-gui/src/app/mod.rs`（`open_preview`）<br>`crates/kichi-gui/src/worker.rs`（`preview_download`）<br>`crates/kichi-gui/src/app/files_page.rs`（类型分类） | **范围已定**：音视频走 mpv 不动；**图片 / PDF / Office / 文本 / 字幕保留「打开」**；**压缩包 / 镜像 / 可执行 / 种子 / 未知类型取消「打开」，只留「下载到本地」**；**任何类型超过 64 MiB 先弹确认**。判据：类型决定「要不要给入口」（看一眼能否得到信息），大小决定「要不要先确认」（成本 ≈ 一次完整下载），两者不合并成「是不是文档」。用**排除名单**而非白名单（文档白名单需覆盖 Office 全家桶 + WPS 专有格式，易漏）。待修：① `open_path` 只 `spawn` 不看退出码，系统无关联程序时 `xdg-open` 其实失败了，界面仍提示「已打开」（假成功）；② 媒体判定只看扩展名，未使用 `File::mime_type` —— `.ts` 等既是视频扩展名又是纯文本，会被误送 mpv，而被改名成 `.bin` / 无扩展名的视频会掉到「整份下载」链路；③ 非媒体预览无进度、无取消；④ **扩展名表已重复且不一致**：`helpers::VIDEO_EXTS` 13 项（含 `m2ts` / `mpg` / `mpeg`）与 `files_page.rs` 里用于选图标的局部数组 10 项不同，导致 `m2ts` / `mpg` / `mpeg` 显示视频图标却被判定为非媒体、走整份下载。验收：压缩包类不再出现「打开」入口且提示可改用下载；无关联程序时提示「系统未关联打开「x」的程序」；`mime_type` 参与判定；类型分类收敛到 `helpers`（或新 `filetypes.rs`）单点维护，图标与预览判定共用同一份表 |
| P2-8 | 缩略图纹理显存无上限、无驱逐 | `crates/kichi-gui/src/app/mod.rs`（`thumbnail_textures`）<br>`crates/kichi-gui/src/worker.rs`（解码为 RGBA） | **P2-7 收尾时发现**：纹理按**解码后的原始尺寸**上传（`worker.rs` 里直接用 `rgba.dimensions()`，未按卡片实际显示尺寸降采样），且 `thumbnail_textures` 只增不删 —— 2000 张图 × 160px RGBA ≈ 200 MB 显存，切目录 / 刷新都不释放；P2-7 收窄了「下载多少」，没管「留住多少」。方向二选一或并用：① 解码前把超过卡片所需尺寸的图降采样（顺带省内存与上传带宽）；② 纹理按目录 / LRU 设上限淘汰。判据：反复进出几个大图目录后显存不持续增长，且卡片显示质量无可感下降 |

> [!NOTE]
> **P2 存在建议顺序**：`P2-6` → `P2-5` → `P2-4`（`P2-7` 已完成）。下一项 `P2-6` 的验收里含用户可直接感知的缺陷（`xdg-open` 假成功、`.ts` 被误送 mpv、压缩包误给「打开」入口），紧迫性高出一档；`P2-4` 修剪后仅剩 `de_*` 一项（约二十分钟），但当前没有会碰到它的重构，故**暂缓**，也可在做 `P2-6` 时一并补掉。`P2-8` 是 `P2-7` 收尾时发现的新问题（纹理显存只增不减），与前两项无耦合，可独立推进。

## P3 · 分发与发布

| 编号 | 任务 | 主要文件 | 说明 / 验收 |
| :-- | :-- | :-- | :-- |
| P3-1 | 打包（rpm / AppImage / flatpak） | `packaging/`（新增 spec / AppImage 配置）<br>`assets/` | 目前只有 `.desktop` + `install-icon.sh`，无发行包 |
| P3-2 | 发布构建 CI（**暂缓**） | 新增 `.github/workflows/` | 无任何 CI。当前是本地单人开发，暂不需要流水线（`cargo test` 手动跑即可）；待确定远端仓库、开始出发行包时再加 `fmt --check` / `clippy` / `test` + tag 触发打包，与 P3-3 一并推进 |
| P3-3 | 补回 `repository` 字段 | `Cargo.toml` | 占位符已移除，确定远端仓库后填回，并同步 README 徽章 / 免责声明的链接 |
| P3-4 | desktop 文件完善 | `packaging/kichi.desktop`<br>`packaging/install-icon.sh` | 可按需补 `TryExec`、系统级安装路径等 |

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

> [!TIP]
> P0、P1 任务已全部完成。下一轮建议从 **P2-6（预览入口范围分层 + 健壮性）** 入手 —— 验收里含用户可直接感知的缺陷（`xdg-open` 假成功、`.ts` 被误送 mpv、压缩包误给「打开」入口）；`P2-4` 已修剪为「只补 `de_*` 静默解析」并暂缓，`P2-8`（缩略图纹理显存）可独立推进，`P3-2` / `P3-3` 待确定远端仓库后再做。
