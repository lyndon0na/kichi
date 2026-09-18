# Kichi 开发任务清单

> [!NOTE]
> 本清单独立于 [`PROJECT_PLAN.md`](./PROJECT_PLAN.md)：`PROJECT_PLAN` 记录**已完成的里程碑与实现笔记**，这里只收敛**待办与改进项**，按优先级排序。
> 文件定位以「文件 + 关键符号」标注（不写行号，避免格式化后漂移）。每项完成后请同步回 `PROJECT_PLAN.md` 的「未实现 / 已知边界」与「路线图」。

## 进度总览

| 优先级 | 主题 | 项数 |
| :-- | :-- | :-- |
| **P0** | 用户可见的功能缺口 | 0 |
| **P1** | 正确性与健壮性 | 4 |
| **P2** | 可维护性与工程 | 5 |
| **P3** | 分发与发布 | 4 |

---

## P0 · 用户可见的功能缺口

> ✅ 全部完成

## P1 · 正确性与健壮性

| 编号 | 任务 | 主要文件 | 说明 / 验收 |
| :-- | :-- | :-- | :-- |
| P1-1 | 分享转存目标目录改为持久化 ID | `crates/kichi-gui/src/worker.rs`（`snapshot_pack_folder` / `move_new_files` / `Cmd::RetryMoveShare`）<br>`crates/kichi-gui/src/msg.rs`<br>`crates/kichi-gui/src/settings.rs` | 目前靠文件夹名 `contains("Pack From Shared") \|\| contains("转存自分享")` 定位服务端生成的暂存目录，服务端改文案或换语言即失效。应改为记录目录 ID |
| P1-2 | 分享链接解析增强 | `crates/kichi-gui/src/app/mod.rs`（`extract_share_id`）<br>`crates/kichi-gui/src/app/dialogs.rs` | 仅识别 URL 中的 `/s/` 子串，其它形态（短链 / 带参数）会被当成裸 ID；补充解析与错误提示 |
| P1-3 | 「打开下载目录」空路径修复 | `crates/kichi-gui/src/app/transfers_page.rs`（`dirs::download_dir().unwrap_or_default()`）<br>参考 `crates/kichi-gui/src/app/mod.rs`（`choose_download_dir`） | 取不到系统下载目录时回退为空路径，按钮变成无操作。应回退到已记住的下载目录或给出提示 |
| P1-4 | 人机验证流程 | `crates/kichi-core/src/client.rs`<br>`crates/kichi-gui/src/app/login.rs` | 被要求网页端人机验证时当前只报错；至少给出明确引导（打开验证页 / 换网络重试） |

## P2 · 可维护性与工程

| 编号 | 任务 | 主要文件 | 说明 / 验收 |
| :-- | :-- | :-- | :-- |
| P2-1 | 上传跨重启续传 | `crates/kichi-core/src/upload.rs`（`OssUploadState`）<br>`crates/kichi-gui/src/worker.rs`<br>`crates/kichi-gui/src/settings.rs`（持久化 `upload_id` / 凭据 / 已传分片） | 现在只在运行期内续传，重启即丢 `upload_id` 与 ETags |
| P2-2 | 并发 / 重试参数可配置 | `crates/kichi-gui/src/worker.rs`（`DL_CONCURRENCY` / `DL_MAX_ATTEMPTS` / `UL_CONCURRENCY` / `UL_MAX_ATTEMPTS`）<br>`crates/kichi-core/src/client.rs`（`OSS_UPLOAD_CONCURRENCY`）<br>`crates/kichi-gui/src/app/settings_page.rs`<br>`crates/kichi-gui/src/settings.rs` | 目前全是编译期常量，设置页只暴露下载目录 |
| P2-3 | 日志轮转 + 预览缓存淘汰 | `crates/kichi-gui/src/logging.rs`<br>`crates/kichi-gui/src/worker.rs`（`preview_root`） | `kichi.log` 无轮转、预览缓存不清理；而目录缓存已有 LRU，口径不一致 |
| P2-4 | 补测试 | `crates/kichi-gui/src/worker.rs`、`msg.rs`、`settings.rs`、`app/*`（各页面）<br>`crates/kichi-core/src/types.rs`、`session.rs`、`error.rs` | 现有 39 项测试只覆盖签名 / 解析 / 格式化等叶子模块，业务逻辑与持久化无测试。优先补纯逻辑：`visible_rows` 排序、`unique_name` 去重、`extract_share_id`、`de_number` / `de_string` 防御式解析 |
| P2-5 | 对话框起始目录记忆 | `crates/kichi-gui/src/app/mod.rs`（`std::env::current_dir()`） | 新建 / 重命名 / 选择目录对话框都从进程 CWD 起，应改用上次使用过的目录 |

## P3 · 分发与发布

| 编号 | 任务 | 主要文件 | 说明 / 验收 |
| :-- | :-- | :-- | :-- |
| P3-1 | 打包（rpm / AppImage / flatpak） | `packaging/`（新增 spec / AppImage 配置）<br>`assets/` | 目前只有 `.desktop` + `install-icon.sh`，无发行包 |
| P3-2 | 发布构建 CI | 新增 `.github/workflows/` | 无任何 CI；建议加 `fmt --check` / `clippy` / `test` + tag 触发打包 |
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

> [!TIP]
> P0 任务已全部完成。建议下一轮从 **P1（正确性与健壮性）** 入手。
