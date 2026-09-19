# Kichi 开发任务清单

> [!NOTE]
> 本清单独立于 [`PROJECT_PLAN.md`](./PROJECT_PLAN.md)：`PROJECT_PLAN` 记录**已完成的里程碑与实现笔记**，这里只收敛**待办与改进项**，按优先级排序。
> 文件定位以「文件 + 关键符号」标注（不写行号，避免格式化后漂移）。每项完成后请同步回 `PROJECT_PLAN.md` 的「未实现 / 已知边界」与「路线图」。

## 进度总览

| 优先级 | 主题 | 项数 |
| :-- | :-- | :-- |
| **P0** | 用户可见的功能缺口 | 0 |
| **P1** | 正确性与健壮性 | 0 |
| **P2** | 可维护性与工程 | 2 |
| **P3** | 分发与发布 | 4 |

---

## P0 · 用户可见的功能缺口

> ✅ 全部完成

## P1 · 正确性与健壮性

> ✅ 全部完成

## P2 · 可维护性与工程

| 编号 | 任务 | 主要文件 | 说明 / 验收 |
| :-- | :-- | :-- | :-- |
| P2-5 | 对话框起始目录记忆 | `crates/kichi-gui/src/app/mod.rs`（`std::env::current_dir()`） | 新建 / 重命名 / 选择目录对话框都从进程 CWD 起，应改用上次使用过的目录 |
| P2-8 | 缩略图纹理显存无上限、无驱逐 | `crates/kichi-gui/src/app/mod.rs`（`thumbnail_textures`）<br>`crates/kichi-gui/src/worker.rs`（解码为 RGBA） | **P2-7 收尾时发现**：纹理按**解码后的原始尺寸**上传（`worker.rs` 里直接用 `rgba.dimensions()`，未按卡片实际显示尺寸降采样），且 `thumbnail_textures` 只增不删 —— 2000 张图 × 160px RGBA ≈ 200 MB 显存，切目录 / 刷新都不释放；P2-7 收窄了「下载多少」，没管「留住多少」。方向二选一或并用：① 解码前把超过卡片所需尺寸的图降采样（顺带省内存与上传带宽）；② 纹理按目录 / LRU 设上限淘汰。判据：反复进出几个大图目录后显存不持续增长，且卡片显示质量无可感下降 |

> [!NOTE]
> **P2 剩余两项无耦合**：`P2-5`（对话框起始目录记忆）体量小、改动集中在一处，适合先收；`P2-8`（缩略图纹理显存只增不减）是 `P2-7` 收尾时发现的新问题，需先定「降采样 / 上限淘汰」的方向再动手，收益是长时间浏览大图目录不再持续吃显存。

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
- [x] P2-6 预览入口范围分层 + 健壮性（4 提交：`4d85185` / `a118b52` / `f5cf109` / `5d25f44`）：
  - **类型分类单点**：新建 `filetypes.rs`，判定顺序为「强 mime 优先 → 通用 mime（`octet-stream` 等）视为无信息 → 唯一扩展名表」；图标与预览判定共用同一份表，消除 `m2ts` / `mpg` / `mpeg` 图标显示视频却走整份下载的不一致（`files_page.rs` / `trash_page.rs` / `helpers.rs` 的重复扩展名表删除）。
  - **入口分层（最终取「排除名单」口径）**：只有压缩包 / 镜像 / 可执行 / 种子不给「打开」（双击等兜底路径在 `open_preview` 里 toast 并提示改用「下载到本地」），其余类型（含 `.psd` / `.wps` / 无扩展名等未知类型）保留「打开」—— 白名单需覆盖 Office 全家桶 + WPS 专有格式，易漏。原 TODO 里「未知类型也取消」的表述按此口径修正。
  - **大文件确认**：非媒体预览需先整份下载，`size > 64 MiB` 且未命中预览缓存时先弹「预览大文件」（显示名称与大小），确认后才发起；媒体走 mpv 流播无整份下载成本，不弹。
  - **假成功修复**：`helpers::open_path` / `open_dir` 换成后台探针 `open_async`（xdg-open 退出码非 0 时按 stderr 措辞区分「无关联程序」与其它失败，1s 超时视为已启动，避免阻塞式 handler 拖住提示），结果经 `PendingOpen` 通道回 UI；预览打开与传输页「打开文件 / 打开目录」三处统一走探针，打开目录成功保持安静。
  - **进度与取消**：非媒体预览下载接入 `download_to` 的 cancel / on_progress 管线（150ms 节流），右下角常驻状态条显示文件名 + 进度条（总大小未知时走不确定动画）+ ✕ 取消；取消后 worker 丢弃未完成的 `.part` 并静默退出（新增 `Cmd::CancelPreview` / `Msg::PreviewProgress`）。
  - 新增单测 10 项：filetypes 分类 9（媒体扩展名 / 强 mime 覆盖 `.ts` / 通用 mime 回退 / 非媒体分类 / 只下载类 / 保留打开类 / 图标与判定一致 / 字幕识别 / 文件夹）+ 探针措辞匹配 1（`filetypes.rs` / `app/helpers.rs` / `app/mod.rs` / `app/files_page.rs` / `app/transfers_page.rs` / `app/dialogs.rs` / `app/types.rs` / `worker.rs` / `msg.rs` / `main.rs`）
- [x] P2-4 补测试（唯一保留项）：`de_number` / `de_string` 的 serde 兼容性用例 5 项 —— `File.size` 与 `Quota` 覆盖数字 / 数字字符串 / 浮点截断 / `null` / 缺字段；`Share` 计数类字段覆盖数字 / 布尔 / `null` 统一转字符串；并把「无法解析的字符串静默归零」固化为已知取舍（`types.rs`，`7097890`）。其余子项按原评估继续搁置（`visible_rows` 需先抽自由函数，`app/*` 页面不追覆盖率）

> [!TIP]
> P0、P1 已全部完成，P2 只剩 `P2-5`（对话框起始目录记忆，体量小）与 `P2-8`（缩略图纹理显存，需先定方向）。P3-2 / P3-3 待确定远端仓库后再做。
