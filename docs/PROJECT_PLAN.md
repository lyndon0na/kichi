# Kichi 开发计划与进展

> [!NOTE]
> 本文档记录 Kichi 的技术选型、里程碑、功能现状与关键实现要点，并与代码保持同步。

## 目录

- [一、目标](#一目标)
- [二、技术选型](#二技术选型)
- [三、里程碑](#三里程碑)
- [四、功能现状](#四功能现状)
- [五、关键实现笔记](#五关键实现笔记)
- [六、质量](#六质量)
- [七、路线图](#七路线图)
- [八、环境](#八环境)
- [九、附录：实现方案](#九附录实现方案)

## 一、目标

做一个可在 Linux 桌面上使用的 PikPak 云盘客户端，覆盖常用能力。

- **形态**：桌面 GUI（egui / eframe）
- **语言**：Rust（edition 2021）——性能好、单二进制、便于长期维护
- **首发范围**：登录 / 文件浏览管理 / 磁力离线下载

## 二、技术选型

| 项 | 选择 | 理由 |
| :-- | :-- | :-- |
| 语言 | Rust（edition 2021） | 内存安全、体积小、生态成熟 |
| GUI | egui + eframe 0.31 | 纯 Rust、迭代快、依赖少 |
| HTTP | reqwest 0.13（native-tls） | 异步、成熟 |
| 异步 | tokio | GUI 后台线程跑网络、channel 通信 |
| JSON | serde / serde_json | 配合防御式解析 |

> [!TIP]
> **关于 eframe 版本**：曾评估 0.35，其 `App` trait 与面板系统大改（`update` → `ui`、移除 `TopBottomPanel`）。为避免在全新 API 上踩坑，固定使用 API 稳定的 **0.31**。

## 三、里程碑

> [!NOTE]
> M0–M24 已全部完成；细节分别见[第四节](#四功能现状)与[第五节](#五关键实现笔记)。

- [x] **M0 · 调研 API**
  - 梳理端点、加签算法、登录 / 文件 / 离线任务的数据结构
  - 参考社区逆向 `Quan666/PikPakAPI`(py)、`Muione/PikpakAPI`(ts) 交叉验证
  - 用固定输入 + python 生成 golden 向量，保证 `captcha_sign` / `device_sign` 正确
- [x] **M1 · `kichi-core`**
  - 常量 / 签名、错误模型、HTTP 层、模型、业务方法、会话持久化、单测
- [x] **M2 · `kichi-gui`**
  - 后台 worker + 登录页 + 文件页 + 离线任务页，真机登录联调
- [x] **M3 · 联调修复**
  - `error_code=16` → 自动 refresh 重试
  - `error_code=9` → 按操作 action 重新 `captcha_init` 后重试（文件列表恢复）
  - `blocking_lock` 在 tokio 线程内 panic → 改 async 获取锁
- [x] **M4 · UI 打磨**
  - 多选 / 全选、排序、过滤、右键菜单、类型标签、快捷键、任务角标
- [x] **M5 · 体验**
  - 深浅主题、记住账号、离线保存位置选择、任务组清空
- [x] **M6 · 本地下载**
  - `.part` + Range 断点续传、原子改名、瞬时错误退避重试、进度 / 速率 / 取消、同名去重、下载目录记忆
- [x] **M7 · UI 重构**
  - 左侧边栏导航 + 底部存储配额卡；文件列表类型图标与可点击排序表头；登录 / 任务 / 设置卡片化；统一明暗主题配色参数（`theme.rs` / `icons.rs`）
- [x] **M8 · 体验继续**
  - 目录缓存（SWR，导航命中零请求）+ 文件列表 / 图标双视图 + 列宽拖拽
  - 本地下载并入「传输任务」页（上传 / 下载分栏、状态筛选、批量操作、虚拟滚动卡片）
  - 下载历史持久化（`downloads.json`，最多 200 条，启动时恢复任务列表）
  - 主题跟随系统 KDE 配色（`kde.rs`，定时轮询），Breeze 风格适配；非 KDE 回退内置浅色
  - 应用图标 / 桌面项（`app_id` 关联）、预览按需解析清晰度并缓存、外挂字幕自动挂载
  - 会话写入原子化 + 续期单飞；「记住密码」存系统密钥环（Secret Service），失效自动重登
- [x] **M9 · 本地上传**
  - gcid 秒传哈希 + 阿里云 OSS 分片（HMAC-SHA1，无 AWS SDK）
  - 文件页 / 传输任务页「上传」入口，进度 / 速率 / 取消 / 重试，秒传命中直接完成
  - 分片并发（默认 4，可在设置页调整）与断点续传（重试间保留 upload_id / 已传分片）、目录递归上传
  - 跨重启续传（`8637660` + `dadd907`，2026-10-03）：落盘 STS 凭证与已传分片断点，12 小时凭证窗口内重启后免取票自动续传，过期 / 本地文件变化安全降级全量重传 —— 探针实机复核推翻了早期「不可行」结论（跨进程 ×4 / 跨重启 ×1 / 回下载 gcid 一致），报告 `docs/UPLOAD_RESUME_PROBE_REPORT.md`
- [x] **M10 · 分享**
  - 文件页选中项创建分享链接（有效期 / 提取码可选）
  - 「我的分享」页列出、复制链接 / 提取码、取消分享、分页
  - 接口 `share`（创建）/ `share/list` / `share:batchDelete`
- [x] **M11 · 回收站**
  - `parent_id=*` + `trashed.eq=true` 列出已删除项（分页，全局跨目录）
  - 右上「清空回收站」调用 `files/trash:empty`（服务端一次清空）
  - 勾选批量 / 单行「还原」（`files:batchUntrash`）与「彻底删除」（`files:batchDelete`，二次确认）
  - 列表 SWR 缓存，还原 / 删除后即时移除并刷新配额与目录缓存
- [x] **M12 · 分享转存**
  - 解析他人分享链接（`share` / `share/detail`）→ 多选文件转存到我的网盘（`share/restore`）
  - 「我的分享」页「转存分享」入口，支持分页加载、文件名过滤、目标网盘目录选择
  - 转存后按快照自动移动新增文件（`batchMove`，失败可重试）
- [x] **M13 · 离线任务分页与轮询优化**
  - 按 `phase` 分页「加载更多」（`page_token`）；手动刷新与自动轮询均保持已加载页数
  - 自动轮询节拍随任务活动自适应：有等待 / 下载中任务时加快，否则显著降频；配额轮询降频，并由登录 / 删除 / 上传完成等关键操作显式刷新
  - 列表重设计：状态页签（带计数）+ 定高卡片 + 虚拟滚动 + 多选批量，与「传输任务」页统一（长名截断、图标操作）
- [x] **M14 · 整目录递归下载**
  - `kichi-core::walk_folder` 递归遍历目录树（`file_list` 分页 + 已访问集合防环 + 文件数上限）
  - 本地按云端层级建同名目录（含空目录）；传输任务页将目录聚合为单张卡片（显示「文件 已完成/全部」与合计大小 / 速率），展开后按层级呈现目录树：子目录行可单独折叠并显示子树文件进度，文件行缩进显示各自进度
  - 取消 / 移除作用于整个目录；失败重试仅重下失败的子文件；目录记录内联目录树快照，避免单次下载挤占历史条数上限
- [x] **M15 · 真实缩略图与网格缩放**
  - 网格视图加载 `File::thumbnail_link` 预签名直链：worker 下载 → 磁盘缓存（`~/.cache/kichi/thumbnails/`）→ `image` crate 解码为 RGBA → `spawn_blocking` 回传 UI → `ctx.load_texture` 上传 GPU 纹理
  - 缩略图保持宽高比显示（不拉伸），无缩略图时回退到文件类型矢量图标
  - Ctrl + 滚轮缩放网格大小（80–160px），图标尺寸 / 文字大小 / 缩略图随卡片等比缩放
- [x] **M16 · 并发 / 重试参数可配置（P2-2）**
  - 设置页新增「传输」卡片：下载并发（1–8，默认 3）、上传并发（1–4，默认 2）、上传分片并发（1–10，默认 4）、单任务重试次数（1–10，默认 5），改动即持久化（`settings.json`）并推送 worker
  - 生效语义：并发即时生效（扩容立刻放行排队任务、缩容不打断在传任务）；重试与分片并发对新启动的任务生效。原编译期常量（`DL/UL_CONCURRENCY`、`DL/UL_MAX_ATTEMPTS`、`OSS_UPLOAD_CONCURRENCY`）移除
  - tokio `Semaphore::set_capacity` 未稳定且 `forget_permits` 缩容会被释放的许可回填，改为自研动态并发闸 `Gate`（AtomicUsize + `Notify::enable` 先注册后判定，避免丢唤醒），补单测验证扩容 / 缩容语义
- [x] **M17 · 日志轮转与磁盘缓存淘汰（P2-3）**
  - `logging.rs` 按大小轮转：1 MiB × 3 备份（`KICHI_LOG_MAX_MB` / `KICHI_LOG_FILES` 可覆盖），仅在**行尾**轮转以免把一条 event 劈到两个文件；启动时兜底轮转上次遗留的超限文件；轮转或写失败降级为只写 stderr
  - 新增 `cache.rs`：预览与缩略图共用一套按 mtime 的 LRU，**双上限**同时生效（预览 512 MiB / 200 条，缩略图 128 MiB / 1000 条）；命中缓存 `touch` 刷新 mtime；**软上限**语义（豁免窗口内 / 正在使用 / 自身超限的条目不删，淘汰扫一轮即止）
  - worker 在启动时后台扫一次，写入后按节流（≥60s 或新增 ≥8 MiB）触发；预览 / 字幕 / 缩略图下载期间登记 `in_use`（含 `.part`），避免删掉正在用的缓存
  - 设置页新增「缓存」卡片：展示占用并可清空（`Cmd::MaintainCache` / `Msg::CacheUsage`）
  - 新增 11 项单测（日志轮转 3 + 缓存淘汰 8）

- [x] **M18 · 缩略图调度重构（P2-7）**
  - `Cmd::LoadThumbnail` 不再于命令主循环内 `await`（原先会占住循环，期间的目录加载 / 预览 / 删除 / 配额刷新全部排队），改为 `tokio::spawn` + 独立并发闸（4）—— 与用户可配的下载并发解耦，缩略图既不排在大文件下载后面，也不占用用户为下载预留的槽位；获取槽位后重查磁盘缓存，避免排队期间的重复下载
  - 请求点由整目录收窄为「可见行 ± 一屏」（`thumb_row_range`，4 项边界单测覆盖未滚动 / 滚动中 / 越界 / 短列表）
  - 新增目录代数 `thumb_gen`：`ListFiles` / `SearchFiles` 的 `append=false`（切换目录 / 刷新 / 新搜索）时自增；在跑任务在下载前 / 解码前 / 回包前三个检查点自行放弃，离开视野的图不再继续下载
  - 失败不再静默：下载只对**瞬时**错误（超时 / 断连 / 5xx 等，复用 `Error::is_transient`）按 600ms 递增退避重试至多 3 次，404 / 403 之类永久失败直接收工；终态回 `Msg::ThumbnailFailed`，UI 结束在途登记并记入 `thumbnail_failed`（本次会话不再重复请求，刷新目录 / 重新搜索时清空以重试）
  - 链接可用性前置判定：服务端对**没有缩略图的文件**（多为非图片 / 视频）会下发空串或相对路径之类的值，reqwest 连请求都构造不出来（`builder error`，实际零网络开销）。这类文件不下载、不重试，直接回退类型图标 —— 属正常状态，默认不写日志（`KICHI_LOG=kichi_gui=trace` 可查），下载失败则每个文件只在终态记一行（含错误来源链）
  - `download_thumbnail` 改为 `.part` + rename 原子落盘并加 8 MiB 响应上限：原先直接写目标路径，中途失败留下的截断文件会被 `exists()` 当成缓存命中，解码必然失败且永不重下

- [x] **M19 · 文件类型分类与预览入口分层（P2-6）**
  - 类型分类收敛到新 `filetypes.rs` 单点：强 mime 优先 → 通用 mime（`octet-stream` 等）视为无信息 → 唯一扩展名表；图标 / 右键菜单 / 工具栏 / 双击路由共用同一份表，消除此前两处表不一致（`m2ts` / `mpg` / `mpeg` 显视频图标却走整份下载）
  - 入口分层取**排除名单**口径：只有压缩包 / 镜像 / 可执行 / 种子不给「打开」（双击这类文件时提示改用「下载到本地」），其余（含未知类型）保留 —— 文档白名单需覆盖 Office 全家桶 + WPS 专有格式，易漏
  - 非媒体预览超过 64 MiB 且未命中缓存时先弹「预览大文件」确认；媒体走 mpv 流播不产生整份下载成本，不弹
  - `open_path` / `open_dir` 换成后台探针 `open_async`：等 `xdg-open` 退出码（1s 超时视为已启动，防阻塞式 handler 拖住提示），非 0 时按 stderr 措辞区分「无关联程序」与其它失败，修掉「spawn 成功即已打开」的假成功
  - 非媒体预览接入 `download_to` 的 cancel / on_progress 管线（150ms 节流）：右下角常驻进度条 + ✕ 取消（`Cmd::CancelPreview` / `Msg::PreviewProgress`），取消后丢弃未完成的 `.part`
  - 新增单测 10 项（filetypes 分类 9 + 打开探针措辞 1）

- [x] **M20 · 缩略图纹理降采样与上限淘汰（P2-8）**
  - **降采样**：`Cmd::LoadThumbnail` 带上 `max_edge`（UI 按「卡片最大尺寸 160 × 0.85 × `pixels_per_point`」算出，钳在 128–512 物理像素），worker 解码后经 `fit_within_max_edge` 用 `image` 的 `thumbnail`（整数箱式抽样）缩到最长边以内再上传 GPU —— 服务端实测下发 720×405（RGBA ≈ 1.17 MB/张），按卡片所需可降到 136×76（≈ 41 KB）～272×153（≈ 166 KB）
  - **上限淘汰**：`thumbnail_textures` 由裸 `HashMap` 换成 `app/thumbs.rs` 的 `ThumbTextures`——按解码后字节数（64 MiB）与张数（512）**双上限**的 LRU，**软上限**语义与 `cache.rs` 一致：只淘汰「距上次使用已过宽限期（2s）」的最久未用项，全部在宽限期内则允许暂时超出
  - **可见集保护**：网格每帧把「可见行 ± 一屏」（与请求区间同一份）的纹理 `mark_used` 刷入宽限期，绘制结束后 `evict` —— 当前视野里的图不会被淘汰后立刻重解码；被淘汰的图滚回视野时重新请求，命中磁盘缓存、不走网络
  - 卡片显示尺寸与判定口径常量化（`GRID_CARD_MIN` / `GRID_CARD_MAX` / `THUMB_MAX_CARD_RATIO`），消除缩放钳制与 `thumb_max_edge` 各自写死数字的漂移
  - 新增单测 12 项（纹理 LRU 8 + `thumb_max_edge` 1 + `fit_within_max_edge` 3）；`KICHI_LOG=kichi_gui=trace` 可看每张纹理的尺寸与缓存累计，debug 可看超限淘汰释放量

- [x] **M21 · 本地选择框起始目录记忆（P2-5）**
  - 新增 `Settings.last_dir`（`settings.json` 持久化）：上传文件 / 上传文件夹两个原生选择框不再从进程 CWD 起（桌面入口 `.desktop` 无 `Path=`，CWD 不可控），改走 `helpers::picker_start_dir` —— 记忆目录仍存在就用它，否则 `home_dir`，再退回进程 CWD
  - 确认选择后经 `helpers::picked_dir` 回写：目录选择记其自身（下次一键重选 / 直接进子目录），文件多选记首个所选文件的父目录；目录已删除时回退自动生效，无需校验界面
  - 下载目录框不受影响：它本就用 `download_dir` 记忆，与上传的记忆互不干扰；原任务描述里的「新建 / 重命名对话框」是应用内纯文本弹框、没有本地目录参数，已按实际口径修正
  - 新增单测 3 项（记忆目录命中（含末尾分隔符）/ 失效与空值回退 / 记自身与父目录的推导）

- [x] **M22 · 发行包与发布 CI（P3-1）**
  - **AppImage**（`packaging/appimage/build-appimage.sh` + `packaging/appimage/AppRun`）：cargo 构建 release → 组装 AppDir（`usr/bin` + desktop + hicolor 图标 + 根目录同名 PNG 与 `.DirIcon`）→ appimagetool 出包；工具版本与 SHA256 写死在脚本里，缺失时自动下载到 `~/.cache/kichi-packaging`
  - **appimagetool 1.9 的坑**：它只在 **AppDir 根目录**找 `*.desktop`，且 `G_FILE_TEST_IS_REGULAR` 要求是普通文件 —— `usr/share/applications/` 下的那份不算数，必须额外在根部放一份；光栅化器改为逐个试（rsvg-convert → ksvgtopng → magick → inkscape）并校验产物非空，因为 `ksvgtopng` 缺输出目录、`magick` 参数顺序不对时会「退出码 0 但没产物」
  - **Flatpak**（`packaging/flatpak/io.github.lyndon0na.Kichi.yml` + `packaging/flatpak/build-flatpak.sh`）：freedesktop 25.08 Platform / Sdk + `rust-stable` 扩展，沙箱内 `cargo build --release --locked` 后装进 `/app`（`skip` 掉 `target` / `dist` / `.git` / `.github`）
  - **沙箱适配**（三处，UI 无差别）：沙箱里没有 mpv（freedesktop 运行时不带），播放改经 `flatpak-spawn --host mpv` 借宿主程序（`helpers::host_command` 统一两种形态，参数完全一致）；运行时字体不含中文，`install_fonts` 的候选表改为「挂载根 × 相对路径」两维展开，同时扫 `/usr/share/fonts` 与 flatpak 挂进来的 `/run/host/fonts`；`app_id` 取 `FLATPAK_ID`，让窗口能关联 `<应用 ID>.desktop`
  - **构建期网络**：flatpak-builder 默认掐断构建沙箱网络（实测沙箱内 DNS 直接失败），清单里用 `build-options.build-args: [--share=network]` 放行（`--share-net` 是 bwrap 语法、flatpak 不认）；若日后要上 Flathub，需改为离线构建并提交 cargo 源清单
  - **CI**（`.github/workflows/release.yml`）：AppImage 在 `ubuntu-22.04` 构建（glibc 门槛等于构建机 → 官方包可跑 Ubuntu 22.04+ / Debian 12+；本地在 Fedora 44 构建只能跑 Fedora 43+/滚动发行版），Flatpak 在 `ubuntu-24.04`（沙箱内自带运行时，与构建机发行版无关）；推 `v*` tag 构建完自动发 Release（已存在则 `--clobber` 覆盖上传），`workflow_dispatch` 只上传 artifacts
  - **CI 首跑踩坑（图标导出校验）**：flatpak 导出阶段用**宿主** gdk-pixbuf 校验图标（`flatpak-validate-icon` 在 ro-bind 宿主 `/usr` 的沙箱里跑），Ubuntu 24.04 的 gdk-pixbuf 2.42 要从 `librsvg2-common` 加载 SVG —— 首次发 `v1.0.0` 时编译安装都成功，导出 `io.github.lyndon0na.Kichi.svg` 才报 `is not a valid icon: Format not recognized`；工作流 apt 列表补装该包解决（Fedora 的 gdk-pixbuf ≥ 2.44 已内置 SVG 加载器，本地构建不复现）
  - **CI 首跑踩坑（发布任务）**：`release` job 只下载 artifacts、不做 checkout，`gh` 没有 `.git` 可推断目标仓库、在第一条 `gh release view` 就失败（`fatal: not a git repository (or any of the parent directories): .git`）—— 给该步骤补 `GH_REPO: ${{ github.repository }}`（此时 AppImage / Flatpak 两个 job 均已成功，只差最后一步）
  - 本地实测：AppImage 解包后二进制正常启动；Flatpak 装进用户级安装后沙箱内无缺失库、宿主字体与 `kdeglobals` 可见、`flatpak-spawn --host mpv` 可用、GUI 正常起窗（不暴露 X11，日志里会有一条 arboard 的 X11 剪贴板告警 —— 剪贴板实际走 Wayland 通道）
  - **桌面集成口径**：桌面文件与图标由发行包提供 —— AppImage 把 `kichi.desktop`（AppDir 根 + `usr/share/applications`）与图标打进包内，Flatpak 由清单导出 `<应用 ID>.desktop` 与同名图标，安装后菜单 / 任务栏图标即正常；源码直跑不在支持范围（Wayland 下任务栏无图标，需要时手工放置 `packaging/kichi.desktop` + `assets/kichi.svg`）。曾用于补这条路的 `packaging/install-icon.sh` 于 2026-09-24 移除
  - 新增单测 2 项（宿主命令前缀 / 中文字体候选查找）

- [x] **M23 · 常规 CI（P3-2）**
  - 新增 `.github/workflows/ci.yml`：`push` 到 `master` 与所有 PR 触发；`concurrency` 用 `ci-${{ github.ref }}` + `cancel-in-progress`（同分支新推送取消上一轮，省 runner 排队）
  - 两个 job：`fmt`（只装 `rustfmt` 组件，跑 `cargo fmt --all --check` —— 无系统依赖、秒级反馈，失败不会被后面的 clippy / test 掩盖）与 `check`（`clippy` + `test`，apt 装 eframe 构建依赖，与 `release.yml` 的列表同源，缓存 `~/.cargo/registry` / `~/.cargo/git` / `target`）
  - clippy 直接上严格模式（`cargo clippy --workspace --all-targets --locked -- -D warnings`）：本地实测早已 0 告警，不需要「先宽松后收紧」的过渡期；cargo 命令统一带 `--locked`，把依赖漂移挡在主干之外
  - 缓存 key 用 `Linux-ci-cargo-*`，与 `release.yml` **刻意区分**：同 key 时两个工作流互相覆盖缓存，且 AppImage 任务在 `ubuntu-22.04` 构建、target 不通用
  - 上 CI 前本地复跑三闸门确认基线：`cargo fmt --all --check` 通过、`cargo clippy --workspace --all-targets --locked -- -D warnings` 0 告警、`cargo test --workspace --locked` 58 项全过（GUI 侧；核心库 33 项合计 91）；README 顶部补 CI 状态徽章、目录树与「构建与运行」补 CI 说明
  - **工具链固定（2026-09-27）**：原先用浮动 `dtolnay/rust-toolchain@stable`，CI 静默升到 rustc 1.98.1，其新增 `float_literal_f32_fallback`（future-incompat，默认告警）在 `-D warnings` 下命中 55 处 `Stroke::new` 字面量而编译失败，本地 1.94 不复现。新增根 `rust-toolchain.toml` 固定 `1.98.1`（含 `clippy` / `rustfmt` 组件），`ci.yml` / `release.yml` 的 Action `@版本` 同步固定；代码侧给 55 处宽度字面量补 `_f32`（纯类型标注，行为不变）。升级工具链时改 `rust-toolchain.toml` 与两处 Action 版本（该 Action 不读该文件）

- [x] **M24 · 打包目录归位（P3-4）**
  - `build-appimage.sh` 收进 `packaging/appimage/`、`build-flatpak.sh` 收进 `packaging/flatpak/`（`kichi.desktop` 留根，AppImage / Flatpak 共用）；脚本内只改 `ROOT` 计算（多退一级到仓库根），其余 `$ROOT/...` 引用零改动
  - `release.yml` 两处调用路径、README 目录树与命令、Flatpak 清单头注释同步；本地出包命令变为 `./packaging/appimage/build-appimage.sh` 与 `./packaging/flatpak/build-flatpak.sh`
  - 为什么等出包时做：脚本平时 CI 不跑（`ci.yml` 只有 fmt / clippy / test），路径敲错只有出包才暴露；本次随 v1.0.1 出包一并验证，错误当场可见
  - 本地验收（2026-09-29）：AppImage 完整出包 + 解包 `AppRun` 启动正常（GUI 恢复登录态、运行至 timeout）；Flatpak 完整出包成功（flatpak-builder 沙箱内 release 构建 45.91s）

## 四、功能现状

### 已实现

<details>
<summary><b>账号与会话 · 文件管理</b></summary>

**账号与会话**
- 邮箱 / 手机号密码登录（含 captcha 流程）、退出登录
- 会话持久化 / 恢复 / 自动续期（access token 过期自动 refresh）
- 会话文件原子写入 + 续期单飞；「记住密码」存系统密钥环，会话失效自动重登

**文件管理**
- 目录浏览（面包屑）、分页加载；目录缓存（SWR，导航命中零请求）
- 排序（表头点击切换升 / 降序）、当前目录名称过滤、列表 / 图标（网格）双视图、列宽拖拽
- 新建文件夹 / 重命名 / 移入回收站（多选、右键、`Delete` 键）；回收后本地即时隐藏，服务端同步后自动解除
- 复制 / 剪切 / 粘贴（多选、右键、工具栏、`Ctrl+C/X/V`）；目录空白处右键可粘贴 / 新建 / 刷新，文件夹右键可「粘贴到此处」；移动后本地即时隐藏并按最终一致性延迟重列，避免残留
- 文件类型矢量图标（文件夹 / 视频 / 图片 / 音频 / 文档 / 压缩）；网格视图加载真实缩略图（`thumbnail_link` 预签名直链下载，磁盘缓存 `~/.cache/kichi/thumbnails/`，`image` crate 解码为 RGBA 后上传 GPU 纹理；只请求可见区 ± 一屏，切目录即取消在跑请求）；Ctrl + 滚轮缩放网格大小（80–160px），图标 / 文字 / 缩略图随卡片等比缩放

</details>

<details>
<summary><b>上传 · 下载 · 传输任务</b></summary>

**本地上传**
- gcid 秒传哈希 + 阿里云 OSS 分片（HMAC-SHA1，**并发 + 断点续传**；**跨重启续传**：12 小时凭证窗口内重启后免取票自动续传，过期 / 文件变化自动全量重传），进度 / 速率 / 取消 / 退避重试；秒传命中直接完成
- 入口：文件页工具栏 / 右键、传输任务页（Wayland 下不支持拖拽，winit 未实现 Wayland 拖放）
- 目录递归上传：云端建目录 + 逐个文件
- 上传列表：目标路径、速率 / ETA、完成时间、状态筛选、全选批量取消 / 重试 / 移除、清除已完成、在网盘中打开
- 上传历史持久化（`uploads.json`，最多 200 条，启动时恢复列表）
- 本地选择框记忆上次用过的目录（`Settings.last_dir`）：文件多选取首个文件的父目录、文件夹选择取所选目录本身，失效 / 首次时回退到 home，再退回进程 CWD

**本地下载**
- 文件页选中 / 右键、离线「已完成」任务一键下载
- 整目录递归下载：选中文件夹（工具栏多选或右键）后遍历云端目录树，本地按云端层级建同名目录（含空目录）
- `.part` + Range 断点续传、原子改名、瞬时错误退避重试、并发上限可配置（默认 3）、进度 / 速率 / 取消、同名自动 ` (n)` 去重、下载目录记忆
- 下载历史持久化（`downloads.json`，最多 200 条，启动时恢复列表）；目录任务保存为一条记录并内联子文件快照

**推送到 aria2（外部下载器）**
- 文件 / 目录右键或选中后「发送到 aria2」：解析限时直链后经 JSON-RPC 推给 aria2 / Motrix 自行下载；整目录先递归扫描，云端层级映射到逐任务的 `dir`、`out` 用文件名
- 手动推送 MVP：不做「默认下载方式」切换、不在传输任务页跟踪进度（下载进度归 aria2 自己的界面）
- 设置页新增「aria2」卡片：启用开关 / RPC 地址（默认 `http://127.0.0.1:6800/jsonrpc`）/ 密钥 / 下载目录（留空 = 读 aria2 的 `getGlobalOption.dir`）/「测试连接」（`aria2.getVersion`）
- 推送期间右下角常驻提示实时更新进度（150ms 节流），结束替换为结果（成功 / 部分失败 / 空目录 / 整批失败）；请求头（UA / `X-Device-Id` / 必要时 Bearer）按 aria2 的 `header` 数组下发，密钥只存 `settings.json` 并随命令下发，不进日志

**传输任务页**
- 上传 / 下载分栏；下载卡片带进度 / 速率 / 状态筛选 / 批量操作；上传卡片带进度 / 取消 / 重试
- 目录下载聚合为单张卡片（显示「文件 已完成/全部」）；展开箭头按层级显示子目录与文件，子目录行可单独折叠、显示其子树文件进度，文件行缩进显示各自进度；取消 / 重试 / 移除作用于整个目录（重试仅重下失败子文件）
- 状态筛选（全部 / 进行中 / 已完成 / 失败）、批量重试 / 移除、虚拟滚动卡片

</details>

<details>
<summary><b>离线下载 · 分享 · 回收站 · 预览 · 界面</b></summary>

**离线下载**
- 磁力 / 直链转存，等待 / 下载 / 完成 / 失败状态页签（带计数）与自适应自动轮询
- 定高卡片列表 + 虚拟滚动；长文件名截断 / hover 显示全名；图标操作与多选批量（重试 / 删除）；顶部常驻提交表单
- 每个状态超过 100 条时「加载更多」分页（`page_token`）；手动刷新与自动轮询均保持已加载页数
- 失败重试、删除任务、保存目录可选

**分享**
- 文件页工具栏 / 右键对选中项（含文件夹）创建分享，可选有效期（1 / 7 / 30 天或永久）与提取码
- 「我的分享」页列出已创建分享（公开 / 私密 / 失效、文件数、有效期、浏览 / 转存数、创建时间），分页加载、列表 SWR 缓存
- 复制链接 / 链接 + 提取码、单条或勾选批量取消、在浏览器打开
- 创建成功后弹框展示链接与提取码，一键复制 / 在浏览器打开

**分享转存**
- 「我的分享」页「转存分享」入口：输入他人分享链接（或 ID）与提取码，解析后多选文件转存到自己网盘
- 支持分页加载、文件名过滤、目标网盘目录选择
- 转存后按快照对比自动移动新增文件，移动失败可重试

**回收站**
- 「我的文件」删除的项在此列出（左图标 + 名称 + 大小 / 删除时间），分页加载、SWR 缓存
- 单行或勾选批量「还原」（回原目录）与「彻底删除」（二次确认，不可恢复）
- 右上「清空回收站」（服务端一次清空）；操作成功即时移除并刷新配额与目录缓存

**文件预览**
- 类型决定「给不给入口」（`filetypes.rs` 单点分类：强 mime 优先 → 通用 mime 视为无信息 → 回退唯一扩展名表）：音视频 / 图片 / PDF / Office / 文本 / 字幕等保留「打开」，压缩包 / 镜像 / 可执行 / 种子只提供「下载到本地」（双击这类文件时提示改用下载）
- 视频子菜单列出云端清晰度（原画 / 1080P / 720P…，无转码流时仅原画，打开子菜单时按需解析并缓存），选择后由 `mpv` 流式播放（带签名直链请求头），自动挂载同集外挂字幕；音频只有原文件，直接播放；缺少 `mpv` 时仅提示安装
- 其他文件下载到 `~/.cache/kichi/preview` 后交系统查看器，命中缓存不重复下载；超过 64 MiB 且未命中缓存时先弹「预览大文件」确认（需整份下载），下载期间右下角常驻进度条并可取消
- 「打开」走后台探针（`helpers::open_async`）：等 `xdg-open` 退出码，无关联程序时提示「系统未关联打开「x」的程序」而非假成功；打开目录成功保持安静，失败才提示
- 预览等待反馈常驻：解析播放地址 / 准备缓存 / 唤起查看器期间右下角提示持续显示（非媒体与进度条并存），直到软件真正唤起才被结果提示替换（`app/global.rs` 的 `Toast::sticky` 常驻形态）；媒体预览的就绪判据是 mpv IPC 观察 `vo-configured`（窗口 / VO 配置完成，`app/helpers.rs::watch_mpv`），解析完成后提示先转「正在唤起 mpv…」，mpv 在窗口出现前退出则如实告警

**界面与主题**
- 左侧边栏导航（我的文件 / 我的分享 / 回收站 / 离线下载 / 传输任务 / 设置）+ 账户 / 存储配额卡
- 读取系统 KDE 配色（`kde.rs`）自动跟随明暗与强调色并适配 Breeze 风格，非 KDE 回退内置浅色
- 任务 / 下载角标、记住账号、中文界面与 CJK 字体自动加载
- 网格视图真实缩略图（磁盘缓存 + GPU 纹理，仅请求可见区 ± 一屏；解码前按卡片所需尺寸降采样，纹理按字节上限 LRU 淘汰），Ctrl + 滚轮缩放网格大小（80–160px），图标 / 文字 / 缩略图随卡片等比缩放

</details>

### 未实现 / 已知边界

| 项 | 说明 |
| :-- | :-- |
| 搜索 | PikPak 无服务端全局搜索 API，采用客户端递归遍历所有目录并按文件名模糊匹配（大小写不敏感） |
| 离线任务展示 | 任务字段防御式解析，仅展示名称 / 大小等有限信息（无进度百分比等） |
| 人机验证 | 部分高危操作被要求网页人机验证时只能失败提示 |
| 上传跨重启续传 | **已实现（12 小时凭证窗口）**：落盘 OSS 临时凭证 + `upload_id` + 已传分片断点（`upload_resume.json`，0600），窗口内重启后**免取票**自动续传；过期 / 本地文件指纹变化时安全降级为全量重传。⚠️ **不要回到「重启后重取票续旧 `upload_id`」的旧路线**（STS 凭证按对象 key 授权，旧 `upload_id` 仍被 `403 AccessDenied` 挡死）。边界：窗口从取票时刻起算，长传中途过期即降级；跨重启实机样本 1 次、>100 分片大文件未验。详见 `UPLOAD_RESUME_PROBE_REPORT.md`（结论）与 `UPLOAD_RESUME_NOTES.md`（历史复盘） |
| 非媒体预览成本 | 预览前需整份下载到本地缓存（超过 64 MiB 且未命中缓存时先弹确认）—— 压缩包 / 镜像这类「看了也没用」的类型已收掉入口，但图片 / 文档仍可能是个大文件 |
| aria2 直链限时 | 推送成功即入队，之后由 aria2 用同一条限时直链下载 / 重试：排队过久或下载太慢会在 aria2 侧过期失败，需重新推送（不做 Kichi 代理中转）；同名冲突同样交给 aria2 自身策略（`auto-file-renaming` / `allow-overwrite`），不代做 ` (n)` 去重 |
| 文件类型兜底 | 服务端 `mime_type` 缺失或为通用类型（`octet-stream` 等）时按扩展名判定，扩展名也未知则**保留「打开」**（排除名单口径）：宁可多给入口，也不因白名单漏项而让正常文件失去打开方式 |
| 打开失败的判定 | 依赖 `xdg-open` 退出码 + stderr 措辞（KDE 分支实际调用 `kde-open`，文案无法从源码确认）：命中「no method available / no application / no handler」等措辞时提示「系统未关联打开…」，其余非零退出统一回退为「打开失败: <stderr 首行>」；阻塞型 handler 超过 1s 未退出即视为已启动，不再等待 |
| 发行包 | 只提供 AppImage 与 Flatpak（rpm 不做）。AppImage 的 glibc 门槛等于构建机：官方产物在 `ubuntu-22.04` 构建（glibc 2.35），本地在 Fedora 44 构建只能跑 Fedora 43+ / 滚动发行版 |
| Flatpak 沙箱 | 配置 / 缓存落在 `~/.var/app/io.github.lyndon0na.Kichi/`（首次需重新登录并重选下载目录）；音视频播放依赖**宿主**已安装的 `mpv`（经 `flatpak-spawn --host`）；不暴露 X11（剪贴板走 Wayland 通道，启动日志可能有一条 arboard 告警） |

## 五、关键实现笔记

### 1) 认证与错误恢复

> [!IMPORTANT]
> 登录前必须走 captcha；`request_inner` 内置两类错误的自动恢复，均**只重试一次**。

**captcha token 签发**
- `POST user.mypikpak.com/v1/shield/captcha/init` 先拿 `captcha_token`
- **登录用**：meta 带邮箱 / 手机号标识
- **一般操作用**：meta 需带 `captcha_sign`，由 `1.` + 对 `client_id+client_version+package+device_id+timestamp` 做 15 轮盐值 MD5 得到

**请求恢复流程**

```text
                       业务请求 request_inner
                              │
                   ① 附当前 captcha_token
                              ▼
                       ┌─────────────┐
                       │  发送请求    │
                       └──────┬──────┘
                              ▼
                    ┌───────────────────┐
                    │  响应 error_code?  │
                    └──┬─────────────┬──┘
              error_code=16        error_code=9
              （token 过期）      （captcha 作用域不匹配）
                    │                │
                    ▼                ▼
        ┌───────────────────┐  ┌──────────────────────────┐
        │ grant_type=        │  │ 按失败请求推导 action      │
        │   refresh_token    │  │  = METHOD:/drive/...      │
        │ 换新 access token  │  │ captcha_init(action) 重签 │
        └─────────┬─────────┘  └────────────┬─────────────┘
                  │ 重试一次                 │ 重试一次
                  └────────────┬────────────┘
                               ▼
                        返回响应 / 结果
```

> [!NOTE]
> `captcha_init` 必须走**裸请求**，避免在恢复流程中递归触发同类处理；用于解析直链的 captcha token 用后即焚，避免影响后续普通请求。

### 2) 设备身份

`X-Device-Id`、`User-Agent`（ANDROID 风格，含 device_sign）需与 captcha / 登录保持一致；device id 每次登录生成并持久化。

### 3) 离线任务状态无字段保证

离线任务 JSON 未公开、字段不稳定，因此按 `phase`（PENDING / RUNNING / COMPLETE / ERROR）分四次查询、按状态分桶展示（GUI 侧为对应状态页签），状态以请求结果为准。

### 4) GUI 与网络隔离

网络在独立 tokio worker 线程中运行；UI 与 worker 通过 `mpsc` 交换 `Cmd` / `Msg`。UI 每帧 `try_recv` 收敛消息，操作即时性由「点击 → 发送 → 回包」驱动。

### 5) 本地下载实现要点（参考社区 pikpakcli / pikpak-downloader）

- **解析直链**：对该文件 id 做 `captcha_init(action=GET:/drive/v1/files/{id})`，再 `GET /drive/v1/files/{id}`，响应里的 `web_content_link` 即带签名的限时直链；详情 kind 含 folder 时明确报错。
- **断点续传**：下载写 `<文件>.part`，完成后 rename 收尾（原子）；`.part` 存在时用 `Range bytes={len}-` 续传，服务端回 200（忽略 Range）或 416（part 失效）时从头 / 截断重来。
- **鉴权回退**：直链对 CDN 无需鉴权头；若返回 401 / 403 再补 Bearer 重试一次。
- **完整性校验**：完成后按详情声明大小校验，收到字节不足则判为截断、保留 part 触发续传，绝不落残缺文件。
- **退避重试**：解析直链与传输的瞬时错误（断连 / 超时 / 5xx / 429 / 传输不完整）统一按退避重试，每次重试前重新解析直链（限时）。

### 6) 会话持久化与记住密码

- `session.json` 保存 access / refresh token 与 device id，续期会重写它。写入采用「临时文件 + rename」原子替换并加进程锁串行化，避免下载 / 预览与定时刷新并发续期把文件写坏（写坏会导致下次启动无法恢复、被迫重新登录）。
- `refresh_token` 刷新加单飞锁，防止并发 401 触发重复刷新；refresh 被服务端拒绝时归类为 `AuthExpired`，UI 明确回到登录页而非停留在「假登录」。
- 「记住密码」用 `keyring`（Secret Service：KDE Wallet / GNOME Keyring）保存账号密码，仅在手动登录成功且勾选时写入；会话失效或首启无会话时用它自动重登。密钥环不可用或无条目时静默回落到登录表单。密码不写入任何配置文件。

### 7) 整目录递归下载

```text
选中文件夹 ──▶ Cmd::StartDownloadFolder
                    │
        worker: kichi-core::walk_folder
          BFS 每层 file_list 翻页; 已访问 id 防环; 文件数上限兜底
                    │
          本地按云端层级建目录(逐级净化, 含空目录)
                    │
   Msg::FolderScanned { items:[FolderItem{is_dir,depth,name,file_id,dir}], total_bytes }
        # worker 合并目录/文件条目并按路径先序排序(同前缀目录在前)
                    │
   UI: 目录条目 → DlNode(is_dir); 文件条目 → DlJob::child(parent=目录) + DlNode(rid)
                    │
   子文件走既有单文件管线; 每次回包 recompute_folder() 汇总聚合进度 + 各子目录计数
                    │
        目录终态 → 写一条记录(内联目录树快照)
```

- **目录树**：`DlJob.nodes` 按先序存放 `DlNode`（目录或文件，带 `depth`）。目录卡片展开后按层级渲染：子目录行可单独折叠，文件行按 `depth * 18px` 缩进；`dl_rows` 跳过已折叠子目录的子孙。虚拟滚动仍要求逐行等高（`DL_CARD_H`），子行只读不参与多选。
- **聚合与计数**：`aggregate_children` 汇总 `total/done/speed` 与完成文件数；`compute_dir_counts` 用栈做一次先序遍历，自底向上累加每个子目录节点的子树文件数，供「k/N 个文件」展示。卡片状态行附加「文件 已完成/全部」。
- **取消即移除**：取消目录会向所有运行中子任务发 `CancelDownload` 并连同子行移除，不写历史（与单文件取消一致）。
- **重试只补失败项**：目录重试仅重下失败的子文件（按节点下标原位替换 `rid`；扫描阶段失败才重新扫描）。不整棵重扫，避免已存在文件被 `unique_name` 的 `exists()` 判定成 ` (n)` 重复下载。
- **历史不膨胀**：目录只写一条 `DownloadRecord`（`is_folder=true` + 目录树快照，含 `is_dir` / `depth`），200 条上限不受子文件数量影响；重启后据此重建目录卡片与目录树。

### 8) 日志轮转与本地缓存淘汰（M17）

```text
日志  写满 1 MiB ──▶ 关句柄 ──▶ .(n-1)→.n 位移 ──▶ kichi.log→.1 ──▶ 重新 append
      (仅在行尾触发; 备份保留 3 个; 轮转失败降级为只写 stderr)

缓存  预览/缩略图写入 ──▶ 节流判定(纯内存) ──▶ spawn_blocking 扫描
                            │                      ├─ 按 mtime 升序: 最久未用在前
                            │                      ├─ 跳过 in_use / 5 分钟豁免窗口
                            └─ ≥60s 或新增 ≥8 MiB   └─ 删到「字节 + 条目」双上限内(一轮即止)
```

- **口径统一**：与内存里的目录缓存（`DIR_CACHE_CAP = 64` + `evict_dir_cache`）一致 —— 有上限、超限按「最久未用」淘汰。磁盘上的差别是「最近使用」只能靠 mtime 表达（Linux 默认 `relatime`，atime 不可信），所以命中缓存要显式 `touch`。
- **目录条目按子树计龄**：预览缓存是「每个文件一个目录」，而目录自身的 mtime 只在增删文件时变；命中缓存时 touch 的是目录里的文件，因此目录条目的「最近使用」取子树里最新的 mtime，否则「刚看过」的预览会被误判成最旧的。
- **软上限**：处于豁免窗口内（5 分钟）、正在使用（`in_use`，含下载中的 `.part`）、或自身就超过上限的条目一律不删，淘汰扫一轮即止 —— 从根上避免「预览完立刻自删、下次又重下」的死循环，超限是允许保留的状态。
- **触发时机**：启动时后台扫一次（不挡首屏）；运行期只在缓存写入后做纯内存节流判定，真扫描全部丢进 `spawn_blocking`。
- **手动清理**：设置页「缓存」卡片展示占用并可清空；清空同样跳过正在使用的条目。

### 9) 缩略图调度（M15 初版 → M18 重构 → M20 显存收口）

```text
UI(图标视图)  可见行 ± 一屏 ──▶ Cmd::LoadThumbnail(file_id, url, max_edge)
              每帧 mark_used 可见集, 绘制后 evict 超限纹理      │
                                │  (命令循环只取样代数并 spawn, 不 await)
                                ▼
worker          thumb_gen 代数比对 ──▶ thumb_sem(4) 取槽 ──▶ 磁盘缓存命中?
                                │                              ├─ 是: touch 后直接解码
                                │                              └─ 否: 下载(退避重试 3 次)
                                ▼                                       │
                      过期则各检查点放弃                        .part → rename 原子落盘
                                │                                       │
                                └── spawn_blocking: 解码 → 按 max_edge 降采样 RGBA ──┘
                                                │
                            ThumbnailReady / ThumbnailFailed ──▶ UI(按字节记账进纹理 LRU)
```

- **为什么 spawn**：命令循环是单点的，缩略图下载在循环里 `await` 会让目录加载 / 预览 / 删除 / 配额刷新全部排队（`Cmd::MaintainCache` 早已 spawn，属口径不一致）。
- **为什么单独一个并发闸**：缩略图下载不经过用户可配的下载并发闸 —— 它既不该排在 10 GB 下载后面，也不该占用用户为下载预留的槽位；但必须限并发，否则大目录会同时开出大量连接。
- **代数取消**：目录切换 / 刷新 / 新搜索时自增 `thumb_gen`，任务在下载前 / 解码前 / 回包前比对，与「命令循环串行取样」配合保证顺序确定；UI 侧同一时刻清空 `inflight` / `failed`，否则被取消的请求会永远占着在途登记（等价于换一种形式的永久空洞）。
- **失败可见**：失败会回包而不是静默 `return`，否则请求方的在途登记悬空、该文件本次会话再也不会被请求（网格永久留白）。终态失败记入 `failed`，刷新目录 / 重新搜索时清空以再试一次。
- **原子落盘**：缩略图与预览一样写 `.part` 再 rename，避免中断产生的截断文件被 `exists()` 当成缓存命中（解码必然失败且永不重下）。
- **按显示尺寸降采样（M20）**：服务端缩略图是给大屏看的（实测 720×405，RGBA ≈ 1.17 MB/张），而卡片最多显示 160 × 0.85 = 136 逻辑像素。`max_edge` 由 UI 按「最大卡片尺寸 × `pixels_per_point`」算出（钳在 128–512），worker 解码后用 `image` 的 `thumbnail`（整数箱式抽样）缩到最长边以内再上传 —— 单张纹理降到 41 KB ~ 166 KB，卡片放到最大、屏幕是 HiDPI 也不糊。
- **纹理按上限淘汰（M20）**：`thumbnail_textures` 是 `ThumbTextures`（64 MiB / 512 张双上限，宽限期 2s）。网格每帧把请求区间（可见 ± 一屏）里的纹理 `mark_used`，绘制结束后 `evict`；超限时只淘汰超过宽限期的 LRU 项，全在宽限期内就允许暂时超出 —— 视野里的图不会被淘汰后立刻重解码，被淘汰的图滚回来时重新请求、命中磁盘缓存不走网络。

### 10) 文件类型分类与预览路由（P2-6）

```text
classify(name, mime)              预览入口                       预览执行
  强 mime(video/ audio/ image/ ─┐
  pdf / office / zip 系 / exe…) │
        └─ 命中 → 直接定类型    │
  通用 mime(octet-stream 等) ───┤
        └─ 视为无信息 → 扩展名表│
  扩展名也未收录 ───────────────┘
        └─ Other(保留「打开」)  │
                                ▼
              preview_kind: Play ──▶ mpv 流播(不下载整份)
                            Open ──▶ >64 MiB 且未命中缓存? 先确认 ──▶ 下载到缓存再 xdg-open
                            DownloadOnly ──▶ 不给入口, 双击时提示改用「下载到本地」
```

- **单点维护**：图标、右键菜单、工具栏与双击路由全部读同一份 `filetypes.rs`；此前 `helpers.rs` 与 `files_page.rs` 各有一份扩展名表且不一致（`m2ts` / `mpg` / `mpeg` 显视频图标却走整份下载），收敛后这类漂移不可能再发生。
- **为什么强 / 通用 mime 分级**：`.ts` 既是 MPEG-TS 视频又是 TypeScript，只看扩展名会误送 mpv；服务端给的 `video/mp2t` 或 `text/plain` 能直接定性。而 `application/octet-stream` 是「服务端也不知道」，若当成结论会退化成「什么都不可预览」，故视为无信息、回退扩展名表。
- **排除名单口径**：只有确定「看了也没用」的四类（压缩包 / 镜像 / 可执行 / 种子）收掉入口；文档白名单要覆盖 Office 全家桶 + WPS 专有格式（`.wps` / `.et` / `.dps`）极易漏项，漏一项就是正常文件失去打开方式。
- **确认阈值只与成本有关**：媒体走 mpv 流播不产生整份下载，多大都不问；非媒体必须下完才能交系统查看器，故 `> 64 MiB` 且未命中缓存时确认一次（命中缓存说明下过，不再打扰）。
- **打开不再假成功**：`xdg-open` 在无关联程序时也会「正常启动又失败」，只 `spawn` 看不出结果；改为后台线程等退出码（最多 1s，超时视为已启动，防阻塞式 handler 拖住提示），按 stderr 措辞区分「无关联程序」与其它失败，结果经通道回 UI。打开目录这类高频噪声只有失败才提示。
- **预览进度与取消复用下载管线**：非媒体预览的缓存下载直接调 `download_to` 的 `cancel` / `on_progress`，不在预览里另造一套；取消同样走 `AtomicBool` 标志位，取消后丢弃未完成的 `.part`，UI 自己清状态（要求取消回包等于让「取消」多一种失败态）。

### 11) 结构优化：巨型文件与 god object（P2-9 起，已完成）

动机：AI 一次改动越容易「碰到隔壁逻辑」，评审 diff 的成本也越高；单次改动的爆炸半径要可见。目标是**按域建子目录**，不是「把文件切小」。

判据（怎么切才算对）：① 职责能一句话命名、文件能一口气读完；② **目录 = 域的边界，文件 = 域内的角色**（文件名取里面的核心类型名）；③ **一起变的东西放一起** —— 同一处业务改动若总要同时改 3 个文件，说明切错了；④ 停止信号：出现 `utils.rs` / `common.rs`、叫不出文件名、一个类型的 `impl` 散落多文件（手写 trait impl 单独成文件属正常例外）、为「每文件 < 500 行」这类教条继续切。

参照（本机 `~/.cargo` 源码 `wc -l` 实测）：egui 114 文件 / 中位 226 行，但 `src/context.rs` **4420 行**、`style.rs` 3163；eframe 28 文件 / 最大 1666；image 71 文件 / 最大 2476（`codecs/` 一个目录 39 文件，每种格式一个）；rustls 107 文件 / 最大 `msgs/handshake.rs` **3266 行**（协议消息表，形态类似 `worker::handle`）。结论：**大文件本身不是问题，一个文件里混着多个概念才是**；god object（`App` 的 100+ 方法与字段）光拆文件治不了。

- **现状（2026-09-24）**：`worker/` 已按域拆完（P2-13，`9da37dd` 起 19 个提交）——`worker.rs` 2460 行拆成 `worker/` 下 12 个文件、入口 `worker/mod.rs` 降至 367 行，`handle` 的 43 个 `Cmd` 分支全部收敛为一行转调；每步 `fmt` 零差异 / `clippy` 0 告警 / 单测全绿，纯搬移零行为变更，实机冒烟（登录 / 上传 / 下载 / 预览 / 转存）已确认。`app/` 侧第 1 步（P2-9）随后落地：`app/mod.rs` 的 4 个无 UI 依赖纯函数（`dl_record_status` / `aggregate_children` / `compute_dir_counts` / `sample_speed`）连同 5 项相关单测外移到新模块（现为 `app/transfers/model.rs`）并另补 2 项（共 7 项，可直接单测），`App::new` 的启动逻辑抽成 `restore_req_id` / `start_session`（`e85211b` / `caa89db` / `c9b57a3`），`app/mod.rs` 3264 → 3085 行；同样纯搬移零行为变更（函数体 / 文档注释逐字比对、抽取语句 `app.` → `self.` 逐行一致）
- **P2-12 切法（2026-09-24 拍板）**：`transfers_page.rs` → `app/transfers/`（`upload_tab` 约 910 行为重灾区），路线取 **A**（拆域时顺手 struct 化，同 P2-11）；`transfers_model.rs` 一并挪进 `app/transfers/model.rs` 成域内纯逻辑文件；切法与提交序列见下条，队列与逐项验收见 `TODO.md`
- **P2-10 切法（2026-09-24 细化）**：域 = 结构体 + 方法 + `drain` 一行转调（沿用 P2-13 的形态）；新增极小的共用句柄 `app/global.rs::Global`（`tx` / `toast` / KDE 配色与主题轮询），域方法签名 `fn on_x(&mut self, g: &mut Global, …)` —— App 侧 `self.trash.on_x(&mut self.global, …)` 是不相交字段借用，不引入 `Rc` / `RefCell`；`App::send` / `theme` / `toast_*` 保留转发壳，页面调用点零改动；跨域副作用（目录缓存作废、配额刷新、页面路由）留在 `drain` 臂里做 2–4 行编排。对原 TODO 条目的两处修正：① `app/browse/`（目录缓存 SWR / 导航栈 / 文件列表）并入 P2-11 的 `FilesPage`（本就是文件页状态，先搬会再收编一次）；② tasks 域（离线任务 + 「保存到」目录选择器）纳入 P2-10（原条目漏列）。`app/system.rs` / `app/dnd.rs` 取消（避免夹缝模块）：`poll_file_picker` / `upload_pick` / `last_dir` 归上传域（P2-12），`poll_pending_open` 归预览域，`persist_settings` / `chrono_now` / `choose_download_dir` 跟全局设置走。序列：D0 `app/global.rs` → D1 回收站 → D2 我的分享 → D3 转存分享 → D4 预览 / 清晰度 / 打开探针 → D5 搜索 → D6 缩略图在途 / 失败登记 → D7 离线任务；P2-14（`App` 字段按页面分组）由 P2-10 / P2-11 / P2-12 全覆盖，不再单独做
- **进度（P2-10）**：**已完成**（D0 `935d3dd` · D1 `83f5ddd` · D2+D3 `72b670a` · D4 `7a6493b` · D5 `9e4b919` · D6 `d0be9fc` · D7 `73edc3e`）。七个域结构体落地：`Global`（命令通道 / 提示条 / 系统配色）、`TrashPage`、`SharesPage`（我的分享 + 转存分享）、`PreviewPage`、`SearchPage`、`ThumbsPage`（纹理 LRU + 在途 / 失败登记）、`TasksPage`（离线任务 + 「保存到」选择器），各持自身状态与 `on_x` 方法；`drain` 的 60 个 `Msg` 臂全部一行转调（跨域副作用留 2–4 行编排），`app/mod.rs` 3085 → 2127 行（`drain` 约 840 → 548 行），`App::new` 的 310 行字面量随各域 `Default` 自然消掉。每步纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / 93 项单测全绿（各步不减），并逐个用归一化脚本比对旧新函数体（逐段 `ALL_MATCH`）；实机冒烟已确认（维护者，2026-09-24）
- **进度（P2-11）**：**已完成**（F1 `37c70ab` · F2 `93a769c` · F3 `60c00cc` · F4a `d86d268` · F4b `1ee33b1`）。`FilesPage` 落在 `app/files/mod.rs`，并**吸收 P2-10 原列的 `browse/`**（目录缓存 SWR / 导航栈 / 文件列表本就是文件页状态），不再单独建 `app/browse/`；渲染按角色分家：`list.rs`（列宽布局 / 表头 / 行集合）、`grid.rs`（网格卡片 + 缩略图预取 + 原有 `thumb_row_range` / `thumb_max_edge` / 卡片尺寸常量）、`row.rs`（单行与右键菜单项）、`toolbar.rs`（顶部栏 + 面包屑）。渲染与动作分离是本轮的额外收获：`FilesPage::show(...) -> Vec<FilesAction>` 只读写自身状态，15 个跨域动作（弹窗 / 预览 / 播放 / 下载 / 分享 / 上传 / 搜索）由 `App::apply_files_action` 在本帧渲染后按序执行 —— 页面不再需要 `&mut App`，也就不会被 `App` 的字段牵着长；顶部栏与两个视图各收一个上下文结构（`TopBar` / `ListCtx` / `GridCtx`）压住参数列表。`app/files/mod.rs` 1891 → 1098 行（`show` 约 440 行 = 状态机 + 入口）。每步纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / 93 项单测全绿，并逐个用逐词归一化脚本比对旧新函数体（入口 38 处差异全为 `req.` 前缀与动作外提，两个视图分支仅有 `cx.` 前缀、`&mut` 重借与格式化换行）；唯一有意的微差异是旧实现在「用户取消选下载目录」时会提前 `return` 漏画一帧列表
- **进度（P2-12）**：**已完成**（T0 `6550268` · T1 `ccf255c` · T2 `54f0636` · T3 `be44c61` · T4 `a81afa9` · T5 `cc20d19`）。`transfers_page.rs`（1951 行）与 `transfers_model.rs` 归并为 `app/transfers/`：`TransfersPage` 持传输域全部状态（两条任务表 / 选中集 / 筛选 / 展开 / 上传选择框）与生命周期方法（入队 / 重试 / 移除 / 历史记录 / 目录扫描收敛 / 进度与速率），`drain` 的 11 条传输消息臂改为一行转调（仅 `UlFinished` 留 4 行跨域编排）；`TransfersPage::show(...) -> Vec<TransfersAction>` 承接原 `App::transfers_page`，4 类跨域动作（打开本地路径 / 打开下载目录 / 文件选择框 / 跳转「我的文件」）由 `App::apply_transfers_action` 在本帧渲染后执行，页面从此不持 `&mut App`（沿用 P2-11 的渲染与动作分离）；分栏分家：`upload.rs` 收 `upload_tab` / `upload_action_bar` / `ul_card` / `upload_status_line`，`download.rs` 收 `download_tab` / `download_action_bar` / `dl_card` / `dl_dir_node` / `dl_file_node` / `status_line` / `paint_rails` / `paint_disclosure` / `node_h` / `tree_block_height`；`last_dir` / `download_dir` 等持久化设置与下载目录选择仍留 `App`。**前置 T0** 把 `req_id` 分配器从 `FilesPage` 搬到 `Global`：worker 的 `cancel_task` 只认一张按 req_id 索引的取消登记表，下载 / 上传 / 预览必须共用同一命名空间（原先下载走 `App`、上传走 `FilesPage`，双分配器迟早撞号）。`app/mod.rs` 1679 → 968 行（再降 711 行，累计 3264 → 968）；`transfers_page.rs` 1951 行变为 `app/transfers/mod.rs` 1216 行（状态 + 生命周期 + 页壳）+ `download.rs` 1020 + `upload.rs` 591 + `model.rs` 253。每步纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / 93 项单测全绿（各步不减），并逐个用逐词归一化脚本比对旧新代码（`download.rs` 的旧代码整体按序包含于新文件，差异只有 `pub(super)` 可见性、动作外提与 rustfmt 换行）；实机冒烟已确认（维护者，2026-09-24）
- **进度（P2-15）**：**已完成**（T1 `88651aa` · T2 `e0ac2c0` · T3 `90f7089`）。`app/tasks.rs`（317 行）与 `app/tasks_page.rs`（643 行，唯一还挂 `impl App` 的带状态页面）收进 `app/tasks/`：`mod.rs::TasksPage` 持状态 / 生命周期与 `TasksAction`，`list.rs` 收页壳（标题 / 新建表单）/ 阶段页签 / 批量操作条 / 任务列表 + 「加载更多」，`card.rs` 收 `task_card`，`picker.rs` 收「保存到」目录选择器（状态操作 + 弹窗渲染）。渲染与动作分离：`TasksPage::show(...) -> Vec<TasksAction>` 只读写自身状态并返回动作，唯一跨域调用「下载到本地」改为 `TasksAction::Download`，由 `App::apply_tasks_action` 在本帧渲染后执行，页面不再持 `&mut App`（其余 `Cmd`：提交 / 刷新 / 重试 / 删除 / 加载更多仍在域内经 `Global` 发出）；自动轮询本就由 worker 按 `tasks_active` 自适应推进，UI 侧无落点需搬。每步纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / 93 项单测全绿（各步不减）；T2 逐词归一化比对旧新 `show` 主体，差异全为签名、`self.tasks.` / `&mut self.global` 前缀去除与动作外提；T3 的 17 个函数体逐一比对 `ALL_MATCH`（仅可见性关键字差异）
- **进度（P2-16）**：**已完成**（C1 `dfe5c1c` · C2 `0b50da6` · C3 `5015bfb` · C4 `a7586e1` · C5 `d6e583e` · C6 `6f2a40f`）。`app/types.rs`（506 行）清空删除，类型随域归位：跨域共享的 `Page` / `CacheUsage` 进 `app/global.rs`；文件域 `SortBy` / `Crumb` / `DirEntry` / `RowAction` / `ViewMode` / `ClipKind` / `Clipboard` / `ColDrag` 进 `app/files/mod.rs`；预览域 `QualityReady` / `QualityMenuState` / `PendingOpen` / `PreviewConfirm` / `PreviewProgress` 进 `app/preview.rs`；`ShareResult` 进 `app/shares.rs`；任务域 `OfflineTab` / `TaskOp` / `TaskSel` 进 `app/tasks/mod.rs`；传输域 `TransferTab` / `DlSel` 留 `app/transfers/mod.rs`，下载类型 `Dl*` 进 `download.rs`、上传类型 `Ul*` / `UploadPick` 进 `upload.rs`（`mod.rs` 以 `pub(crate) use` 重导出 `DlStatus` 给 sidebar 角标）。**按代码实况修正计划**：`TransferTab` 实测只被 `app/transfers/mod.rs` 使用（非跨域），未留 `global.rs` 而归入传输域。纯机械搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / 93 项单测全绿（各步不减）；脚本取出改动前 `app/types.rs` 的 457 行非导入内容逐行 `grep -F`，在当前 `app/` 树全部命中（`missing=0`）验证无丢行；可见性保持原 `pub(crate)` 未顺手收紧，留待后续按引用面单独处理
- **进度（P2-17）**：**已完成**（`66614b8`）。`app/shares.rs`（1608 行，GUI 最大单文件）拆为 `app/shares/`：`mod.rs`（409 行）持 `SharesPage` 状态 / 生命周期 / `Msg` 处理与 `ShareResult`；`mine.rs`（634 行）收「我的分享」列表与创建分享 / 分享结果 / 取消分享确认弹窗（`draw` / `draw_dialogs`）；`restore.rs`（597 行）收转存分享（`parse_share_input` 解析 / 文件浏览 / 「保存到」目标目录选择器 / 保存后自动移动与重试）。**原计划「`mod.rs` 含页壳」按实况修正**：该域没有独立页壳（`draw` 即我的分享页本身），故 `draw` / `draw_dialogs` 一并归入 `mine.rs`，`mod.rs` 只留状态与生命周期。不动结构与 `App` 字段；三个渲染方法可见性由 `pub(super)` 放宽为 `pub(crate)`（调用点 `app::sidebar` / `app::dialogs`）。纯搬移零行为变更：`fmt` 零差异 / clippy 0 告警 / 93 项单测全绿（各步不减）；归一化脚本比对旧新全部 1348 行非导入内容，唯一差异是 `// ---------- 渲染 ----------` 分隔注释（分文件后不再适用）
- **决策时的三选项（备查）**：
  - **A（已选）** 拆域时顺手 struct 化（`app/files/` 里直接放 `FilesPage` + `show()`，`App` 持 `files: FilesPage`）：一次到位，但触及所有 `self.xxx` 引用，逐页 UI 冒烟
  - **B** 先只按职责切文件、不动结构：diff 小、好回滚，但要再搬一轮
  - **C** 先做 `worker/`（管线边界清晰、无 UI 状态纠缠），页面结构化留最后：业务风险最低，`app/mod.rs` 期间仍在长
- **目标形态**：`app/` 下页面 / 工具单文件 + 域目录（`files/` / `transfers/`）+ `worker/` 域文件，总量与 eframe 同量级；`app/mod.rs` 只留 `App` 字段 + `new` + `drain` 分发 + `eframe::App` + 跨域胶水。**实况修正（2026-09-24）**：`app/mod.rs` 收敛到 968 行（`drain` 307 行为消息扇出表，形态同 rustls `msgs/handshake.rs`；其余约 660 行为跨域胶水：preview 路由 / 下载与上传入口 / 选择框与拖拽 / 搜索与分享触发 / 持久化 / `apply_*_action`），未达原始的「300–500 行」；复核后决定**不再继续拆 `App`**（继续下沉要动 preview / files / transfers 三家交接面，边际收益低），目标按实况修正为「`drain` 扇出表 + 跨域胶水」，约 900–1000 行
- **第二批收尾（2026-09-24 立项）**：主线队列清空后的复核结论 —— 按判据 ② / ③ 还剩三处同类尾巴：tasks 域收口（P2-15，**已完成** `88651aa` / `e0ac2c0` / `90f7089`）、`app/types.rs`（506 行）类型未随域归位（P2-16，**随后已完成**）、`app/shares.rs`（1599 行）状态与渲染同文件（P2-17，**已完成** `66614b8`）；三条互不依赖、可各自回滚，清单与验收见 `TODO.md` 的「第二批」小节。**外部结构建议（桶目录）复核不采纳**：有建议把 `worker/` 的 Cmd 处理收进 `handlers/`（`files` / `transfer` / `shares`）、把 `app/` 的跨域逻辑收进 `state/`（`file_ops` / `preview` / `persistence`）—— 其「巨型单文件拆成目录」的目标与本轮一致，且「低风险」一条已达成（`worker` / `app` 均为私有模块，`mod worker;` 对 `worker.rs` 与 `worker/mod.rs` 同样解析，拆分对 `main.rs` 与外部 `use` 零侵入），但其目录形态是**桶目录**（夹缝层）：`handlers/transfer.rs` 会把下载 / 上传两条独立管线捆回一个文件，`state/` 会把文件页与预览的状态从各自域里再切出去，违背判据 ②「目录 = 域的边界」与 ③「一起变的东西放一起」，故实际采用「按域平铺 + 域结构体」；其引用的 `app/mod.rs` 3200+ 行 / `worker.rs` 2400+ 行为 P2-9 之前的快照，现为 968 / 367 行
- **队列与逐项验收**：见 `TODO.md` 的「P2 · 结构优化」第一批（P2-9 ~ P2-14）与第二批（P2-15 ~ P2-17）；每步都要 `cargo fmt` + `clippy -- -D warnings` + `cargo test` 全绿、`#[test]` 数不减，`worker/` 与上传 / 下载管线额外实机验证，路径变化同步 README 目录树与 `AGENTS.md` 代码地图

### 12) aria2 外部下载器推送（P0-7）

需求：把云端文件 / 目录交给 aria2（或 Motrix）下载 —— 手动推送 MVP，不做「默认下载方式」切换、不在传输任务页跟踪进度。

数据流：`文件页（右键 / 选中工具栏）→ FilesAction → App::push_aria2 → Cmd::PushToAria2 → worker/aria2.rs`：

```text
整目录: walk_folder ─┐
单文件: ─────────────┴─▶ 逐文件: file_download_link → stream_headers → aria2.addUri(url, {out, dir, header})
                              │(独立小闸 4, 与下载 / 缩略图解耦)
                              └─ 进度 150ms 节流 → 常驻提示 → 结果(成功 / 部分失败 / 空目录 / 整批失败)
```

实现要点与实测结论（aria2 1.37 独立 daemon，2026-09-29）：

- **请求头统一走 `header` 数组**：实测逐任务 `user-agent` 选项与 `header` 数组都能覆盖全局 UA；取后者是因为它同时承载 `X-Device-Id`（必要时还有 Bearer）—— 一次下发全部服务端要求的请求头。
- **`dir` 缺失时 aria2 会自行创建**（含多级，实测 `dir+out` 落盘正确），推送前不建本地目录；下载目录取设置页配置，留空则读一次 `aria2.getGlobalOption.dir` 再拼云端层级。
- **错误先按 JSON-RPC 解析**：aria2 对 RPC 错误（如密钥不对）也返回 HTTP 400 + 标准错误体，若先判状态码会把 `Unauthorized` 包成 `HTTP 400: {json}`；现顺序为「JSON 可解析 → 取 result / error，否则回退状态码错误」。
- **密钥只走内存**：`Aria2Client` 每次推送由命令携带的配置构造（worker 不读设置文件）；日志只记文件名与错误原因，不含直链 / 密钥。
- **两个已知代价**（已同步 README 已知限制）：直链限时 —— 排队过久 / 下载太慢会在 aria2 侧过期失败，需重新推送，不做 Kichi 代理中转；同名冲突交给 aria2 自身的 `auto-file-renaming` / `allow-overwrite` 策略，不代做 ` (n)` 去重。
- **验证**：单测（请求体构造 / options 编码 / 响应解析 / 服务不可达）对拍本机 `aria2c --enable-rpc --rpc-secret=test` 实推（`dir+out+header` 落盘、错误消息均实测确认）；推给真实 PikPak 直链与 Motrix 的端到端由维护者实机确认。

## 六、质量

| 检查 | 结果 |
| :-- | :-- |
| `cargo test --workspace` | **106 项**（核心库 41 + GUI 65；本轮新增 aria2 JSON-RPC 请求体 / 响应解析 / 服务不可达 8 项，推送目标目录拼接与净化 2 项） |
| `cargo check --workspace` | 零 warning |
| `cargo clippy --workspace --all-targets` | 零 warning |
| `cargo fmt --all -- --check` | 零差异（配置见 `rustfmt.toml`） |
| CI（`.github/workflows/ci.yml`，P3-2 / M23） | 上表前三项由 push `master` / PR 自动执行（clippy 带 `-- -D warnings`、cargo 命令带 `--locked`） |
| 真机验证 | 登录、自动续期、目录加载、离线任务查询（用户账户实测）；Flatpak 装进用户级安装后沙箱内无缺失库、宿主字体 / `kdeglobals` 可见、`flatpak-spawn --host mpv` 可用、GUI 起窗正常 |

<details>
<summary>测试覆盖范围</summary>

加签 golden 向量、JSON 解析、token 边界、直链解析回退、清晰度解析、文件名净化、batchMove / batchCopy 请求体、媒体类型识别、分享列表 / 创建响应解析、回收站 trashed 过滤条件；目录下载的本地路径拼接 / 净化、目录子文件聚合进度与状态推导、目录树子目录计数上卷、下载记录状态映射（终态直传 / 非终态兜底记为未完成）与速率取样的短间隔节流（后四项纯逻辑迁至 `app/transfers/model.rs`，可直接单测）；日志的备份位移与最旧丢弃、只在行尾轮转、启动时兜底轮转超限文件；磁盘缓存的字节 / 条目上限淘汰顺序、目录条目按子树最新 mtime 计龄、豁免窗口与「正在使用」路径豁免、清空缓存、缺失根目录视为空；缩略图请求行区间的「可见区 ± 一屏」、滚动跟随、越界收敛与短列表截断；文件类型分类的强 mime 覆盖扩展名（`.ts` 三态）/ 通用 mime 回退扩展名 / 只下载类与保留打开类 / 图标与预览判定一致 / 字幕识别 / 文件夹判定；`xdg-open` 无关联程序措辞匹配（且不误判「文件不存在」）；`de_number` / `de_string` 的数字 / 字符串 / 浮点 / 布尔 / `null` / 缺字段兼容形式；宿主程序调用前缀（Flatpak 内 `flatpak-spawn --host` / 沙箱外直连）与中文字体候选的「挂载根 × 相对路径」查找顺序；aria2 JSON-RPC 的 token 前置与参数顺序、`addUri` options 的空字段省略与 `header` 编码、错误 / result 解析与响应片段截断、服务不可达报错，以及推送目标目录的云端层级拼接与逐级净化。

</details>

> [!NOTE]
> 本地下载链路依据社区逆向实现，直链字段以 `web_content_link` 优先，采用防御式解析降低变更风险。

## 七、路线图

- [x] **上传** —— 见 M9 与[第九节](#九附录实现方案)。已实现目录递归、分片并发、运行期内断点续传（同一张票内保留 upload_id / 已传分片，退避重试后接着传）；**跨重启续传已落地**（`8637660` + `dadd907`：落盘凭证 12 小时窗口内免取票续传、过期自动全量降级 —— 探针复核推翻了早期「不可行」结论，见 `docs/UPLOAD_RESUME_PROBE_REPORT.md`）；后续可加：多选批量队列优化
- [x] **分享 / 回收站** —— 端点见[第九节 6)](#6-分享--回收站端点m10--m12)
  - 生成自己的分享链接 + 「我的分享」管理（创建 / 列出 / 复制 / 取消）
  - 分享转存：解析 mypikpak 分享链接并保存到我的网盘（`share` / `share/detail` / `share/restore`），含分页 / 过滤 / 目标目录 / 移动重试；转存暂存目录（「转存自分享」）按持久化 ID 定位，ID 失效时回退名称匹配并刷新缓存
  - 回收站浏览 / 还原 / 彻底删除（含清空）
- [x] **全局搜索** —— 客户端递归遍历所有目录 + 文件名模糊匹配（PikPak 无服务端全局搜索 API；盘大时偏慢为已知取舍）
- [x] **推送到 aria2** —— 手动推送 MVP：文件 / 目录经 JSON-RPC 交给 aria2 / Motrix 下载（`kichi-core/src/aria2.rs` + `worker/aria2.rs`，设置页配置与「测试连接」，整目录保留云端层级）；不做默认下载方式切换与传输页进度跟踪；直链限时与同名冲突策略为已知代价（P0-7，`7d835da` + `72b9fa5`）
- [ ] **离线任务进度详情** —— 离线任务字段无官方契约，现仅展示名称 / 大小等有限信息，暂无进度百分比
- [x] **可维护性** —— 结构优化：巨型文件与 god object，P2-9 ~ P2-14 全部落地。`worker/` 拆分（P2-13）已完成（`worker.rs` 2460 行 → 入口 `worker/mod.rs` 367 行 + 11 个域文件）；`app/` 侧第 1 步（P2-9）已完成（纯逻辑外移到纯逻辑模块 + `App::new` 启动逻辑外抽），第 2 步（P2-10，按域 struct 化）已完成（七个域结构体 + `drain` 60 臂一行转调，`app/mod.rs` 3085 → 2127 行），第 3 步（P2-11，文件页）已完成（`FilesPage` + `app/files/` 五个角色文件，渲染与动作分离），第 4 步（P2-12，传输页）也已完成（`TransfersPage` + `app/transfers/` 四文件，`transfers_page.rs` 1951 行收进 `app/transfers/mod.rs` 1216 行 + 上传 / 下载分栏与纯逻辑分文件，渲染与动作分离；`app/mod.rs` 最终 2127 → 968 行）；P2-14（`App` 字段按页面分组）由 P2-10 / P2-11 / P2-12 覆盖。第二批 P2-15（tasks 域收口）与 P2-16（`app/types.rs` 类型随域归位，该文件已删除）随后完成，P2-17（分享页按角色分文件，`66614b8`）也已落地 —— 两批结构优化全部收尾。判据、参照数据、目标形态与已定的 A 路线见第五节 11) 与 `TODO.md` 的「P2 · 结构优化」一节
- [x] **分发** —— AppImage / Flatpak 打包脚本 + GitHub Actions 发布工作流（M22，推 `v*` tag 自动发 Release）；rpm 不做；仓库元数据与链接随后补齐（P3-3：`Cargo.toml` 的 `repository` 字段 + README 动态 release / 打包状态徽章 + Issues / LICENSE 链接）；日常闸门随后补齐（P3-2 / M23：`.github/workflows/ci.yml`，push `master` / PR 跑 fmt + clippy + test，`release.yml` 仍只管打包发版）；打包脚本随 M24（P3-4，`8bbb355`）归位 `packaging/appimage/` 与 `packaging/flatpak/`，本地出包命令随之更新

## 八、环境

| 项 | 值 |
| :-- | :-- |
| 开发机 | Fedora 44，KDE Plasma，Wayland |
| 工具链 | cargo / rustc 1.98.1（由根 `rust-toolchain.toml` 固定，CI / 发布同步） |

## 九、附录：实现方案

### 参考实现

| 项目 | 语言 | 用途 |
| :-- | :-- | :-- |
| `Bengerthelorf/pikpaktui` | Rust | 上传 / 下载 / 分享（**首选**） |
| `52funny/pikpakcli` | Go | 上传 + 分享 |
| `52funny/pikpakhash` | Go | gcid 算法 |
| `rclone/rclone backend/pikpak` | Go | gcid 交叉验证 |

### 1) gcid（秒传哈希）

分块 SHA1，再对「各块 sha1 原始摘要的拼接」取 SHA1（三处实现 + rclone 一致）。

```text
                    文件字节流
                        │
        ┌───────────────┴───────────────┐
        │  按块大小切块（256KB 起）      │   ≤ 128MB  → 256KB
        │  size/psize > 512 且          │   ≤ 256MB  → 512KB
        │  psize < 2MB 时块大小翻倍      │   ≤ 512MB  → 1MB
        └───────────────┬───────────────┘   > 512MB  → 2MB
                        ▼
          [ blk0 ][ blk1 ][ blk2 ] ⋯
                        │  每块 SHA1
                        ▼
              concat( raw_sha1(block) )
                        │  SHA1
                        ▼
             hex( · ) = GCID   ── 创建票据时用大写
```

- 最终 `hex(sha1(concat(raw_sha1(block))))`；空文件为零块 → `sha1("")`
- 创建票据时 `hash` 用大写（rclone 行为；pikpakcli 传小写也工作）

### 2) 创建上传票据

`POST /drive/v1/files`：

```json
{ "kind": "drive#file", "name": "...", "size": "<十进制字符串>",
  "hash": "<GCID 大写>", "upload_type": "UPLOAD_TYPE_RESUMABLE",
  "objProvider": { "provider": "UPLOAD_TYPE_UNKNOWN" },
  "parent_id": "<可选>" }
```

- 需 captcha；`error_code=9` 时按 action `POST:/drive/v1/files` 重新 captcha 后重试一次（本项目 `request_inner` 已内置该恢复）
- `file.phase == "PHASE_TYPE_COMPLETE"` → 秒传命中，直接结束
- `file.phase == "PHASE_TYPE_PENDING"` → 取 `resumable.params`（`access_key_id` / `access_key_secret` / `bucket` / `endpoint` / `key` / `security_token`）

### 3) OSS 分片上传（Aliyun OSS HMAC-SHA1，无需 AWS 签名 / SDK）

**请求流程**

```text
 ① 发起   POST  https://{endpoint}/{key}?uploads
                └─▶ 解析 XML <UploadId>

 ② 分片   PUT   https://{endpoint}/{key}?partNumber=N&uploadId=ID
                └─▶ 读响应头 ETag（去引号）          ┐ 可并发（沿用
             （块大小 max(ceil(size/10000), 5MB)）    ┘ 下载并发思路）

 ③ 完成   POST  https://{endpoint}/{key}?uploadId=ID
                体 <CompleteMultipartUpload>
                     <Part><PartNumber>N</PartNumber><ETag>E</ETag></Part> ⋯
                   </CompleteMultipartUpload>
```

**签名（string_to_sign 逐行拼接）**

```text
 string_to_sign =
   METHOD                         + "\n" +
   <Content-MD5，留空>            + "\n" +
   Content-Type                   + "\n" +
   <RFC1123 GMT Date>             + "\n" +
   x-oss-security-token:<token>   + "\n" +
   /{bucket}/{key}?{raw_query}         ← query 顺序须与请求一致

 sig = base64( HMAC-SHA1(secret, string_to_sign) )
 Authorization: OSS {access_key_id}:{sig}
```

- 请求头另需 `Date` / `Content-Type: application/octet-stream` / `x-oss-security-token`
- 分片成功后**无需等 task**（pikpaktui / pikpakcli 均不上报 task），清理目录缓存即可

### 4) 依赖

`sha1 = "0.10"`、`hmac = "0.12"`、`base64 = "0.22"`（`Date` 头可手写或加 `httpdate`）。

### 5) 落地步骤

- **`kichi-core`**：新增 `upload.rs`（gcid / OSS 签名 / 分片）+ `client.rs` 的 `upload_create` / `oss_initiate` / `oss_upload_part` / `oss_complete`；模型防御式解析。
- **`kichi-gui`**：`Cmd::StartUpload`、`Msg::Ul*`、`worker::spawn_upload`（并发信号量 / 取消 / 退避重试 / 进度）、`helpers::pick_files`、文件页「上传到此处」、传输任务页上传分栏接真实列表。
- **测试**：gcid 黄金向量（与 pikpakhash / pikpaktui 对拍）、签名串黄金向量、创建响应解析、0 字节 / 秒传 / 分片完成分支。

### 6) 分享 / 回收站端点（M10 / M12）

| 操作 | 端点 |
| :-- | :-- |
| 分享信息 | `GET /drive/v1/share?share_id=&pass_code=` → `pass_code_token` / `title` / `share_status` |
| 分享目录 | `GET /drive/v1/share/detail?share_id=&parent_id=&pass_code_token=&limit=100[&page_token=]` |
| 转存 | `POST /drive/v1/share/restore` `{kind:"drive#file", share_id, pass_code_token, file_ids}` → `restore_status` / `restore_task_id` |
| 创建分享 | `POST /drive/v1/share` `{file_ids, share_to, expiration_days, pass_code_option}` |
| 我的分享 | `GET /drive/v1/share/list` |
| 删除分享 | `POST /drive/v1/share:batchDelete` |
| 回收站列表 | `GET /drive/v1/files?parent_id=*&filters={"trashed":{"eq":true}}`（全局跨目录，必须带 `parent_id=*`） |
| 还原 | `files:batchUntrash` |
| 彻底删除 | `files:batchDelete` |
| 清空回收站 | `PATCH /drive/v1/files/trash:empty` |

> [!NOTE]
> **转存不带 `to`**：`share/restore` 请求体不含目标目录，转存先落到分享自带目录；目标位置由客户端在转存成功后 `batchMove` 到用户指定目录（按快照对比新增项）。
