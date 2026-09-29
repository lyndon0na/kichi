# Changelog

本项目所有值得注意的变更都记录在此文件。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

## [1.0.1] - 2026-09-29

### 新增

- **文件页面包屑「…」可点击跳转**：目录层级过深、面包屑折叠出「…」时，点击「…」会弹出被省略的上级目录列表，可直接跳转到任意一层（此前「…」只是省略提示、不可交互）；列表沿用 KDE 风格扁平菜单项，超长目录名截断
- **日常 CI**：新增 `.github/workflows/ci.yml`，推送 `master` 与所有 PR 自动跑 `cargo fmt --all --check` 与 `cargo clippy --workspace --all-targets --locked -- -D warnings` + `cargo test --workspace --locked`（`fmt` 拆成独立 job：无系统依赖、秒级反馈；`--locked` 让依赖漂移不进主干；同分支新推送取消上一轮未跑完的检查）；缓存 key 用 `Linux-ci-cargo-*` 与 `release.yml` 区分，避免两个工作流互相覆盖缓存。README 顶部补 CI 状态徽章，本地开发命令不变
- **工程：新增界面风格规范 `UI_STYLE.md`**：把 KDE / Breeze（Dolphin）观感的落地要求收敛成单一规范 —— 总原则、颜色 / 圆角 / 间距 / 字号台账、图标与布局骨架、各类控件（按钮 / 菜单 / 列表 / 网格卡片 / 面包屑 / 弹窗 / Toast）与三态反馈、新建 / 改动界面的检查清单；与代码实况冲突时以代码为准并顺手修正文件。纯文档新增，无行为变化

### 变更

- **文件页顶部栏上传入口改为图标按钮**：「我的文件」右上角原先的文字「上传」下拉菜单与 `+`（新建文件夹）合并为一个上传图标按钮，点击弹出「上传文件 / 上传文件夹」（菜单项为 KDE 风格扁平行：常态无边框、仅悬停整行高亮，宽度按文字自适应，不会缩成一条也不至于过宽）；移除顶部栏的新建文件夹按钮（列表空白处右键菜单仍可新建）。同时删除不再使用的 `Glyph::Plus` 图标
- **工程：分享域渲染按角色分文件**：`app/shares.rs`（1608 行，GUI 最大单文件）拆成 `app/shares/` 下三个文件 —— `mod.rs`（`SharesPage`：状态 / 生命周期 / 消息处理 + `ShareResult`）、`mine.rs`（我的分享列表 + 创建分享 / 分享结果 / 取消分享确认弹窗）、`restore.rs`（转存分享：链接解析 / 文件浏览 / 「保存到」目标目录选择器 / 保存后自动移动与重试）；不动结构与 `App` 字段，三个渲染方法可见性由 `pub(super)` 放宽为 `pub(crate)`。纯结构调整：用户可见行为零变化
- **工程：界面层内部类型随域归位，删除 `app/types.rs`**：原 506 行的 `app/types.rs` 把跨域与各域类型混放一处，改一处传输任务类型要同时开 `app/transfers/*` 与该文件；现按域拆开 —— 跨域共享的 `Page` / `CacheUsage` 进 `app/global.rs`；文件域类型（`SortBy` / `Crumb` / `DirEntry` / `RowAction` / `ViewMode` / `ClipKind` / `Clipboard` / `ColDrag`）进 `app/files/mod.rs`；预览域类型进 `app/preview.rs`；`ShareResult` 进 `app/shares.rs`；任务域类型（`OfflineTab` / `TaskOp` / `TaskSel`）进 `app/tasks/mod.rs`；传输域类型按下载 / 上传分别进 `app/transfers/download.rs` / `upload.rs`（共享的 `TransferTab` / `DlSel` 留 `app/transfers/mod.rs`）。纯结构调整：用户可见行为零变化
- **工程：离线任务页收进 `app/tasks/`**：`app/tasks.rs` + `app/tasks_page.rs`（后者是最后一个仍挂 `impl App` 的带状态页面）合并为 `app/tasks/` 下四个文件 —— `mod.rs`（`TasksPage`：任务分桶 / 分页 / 选中 / 新建表单 + 生命周期）、`list.rs`（页壳 / 阶段页签 / 批量操作条 / 列表 + 「加载更多」）、`card.rs`（单张任务卡片）、`picker.rs`（「保存到」目录选择器）；沿用「渲染与动作分离」：`TasksPage::show(...)` 只读写自身状态并返回 `Vec<TasksAction>`，唯一跨域动作「下载到本地」由 `App::apply_tasks_action` 在渲染后执行，页面不再需要 `&mut App`。纯结构调整：用户可见行为零变化
- **工程：传输任务页拆进 `app/transfers/`**：`transfers_page.rs`（1951 行）拆成 `app/transfers/` 下四个文件 —— `mod.rs`（`TransfersPage`：下载 / 上传任务表、选中集、筛选与展开 + 生命周期与渲染入口）、`download.rs`（下载分栏：任务卡片 / 目录树 / 底部批量操作条）、`upload.rs`（上传分栏：任务卡片 / 底部批量操作条）、`model.rs`（原 `app/transfers_model.rs` 的纯逻辑：状态映射 / 进度聚合 / 目录树计数 / 速率取样）；沿用「渲染与动作分离」：`TransfersPage::show(...)` 只读写自身状态并返回 `Vec<TransfersAction>`，打开本地路径 / 打开下载目录 / 文件选择框 / 跳转「我的文件」四类跨域动作由 `App::apply_transfers_action` 在渲染后统一执行，页面不再需要 `&mut App`；`drain` 的 11 条传输消息臂收敛为一行转调（仅上传完成留 4 行跨域编排）；顺带把 `req_id` 分配器从 `FilesPage` 搬到 `Global` —— 下载 / 上传 / 预览共用 worker 里同一张按 req_id 索引的取消登记表，必须共用同一命名空间。`app/mod.rs` 1679 → 968 行。纯结构调整：用户可见行为零变化
- **工程：文件浏览页拆进 `app/files/`**：`files_page.rs`（1891 行）拆成 `app/files/` 下五个文件 —— `mod.rs`（`FilesPage`：导航栈 / 目录缓存 SWR / 选中集 / 排序过滤 + 渲染入口）、`list.rs`（列表视图：列宽布局 / 表头 / 行集合）、`grid.rs`（网格视图：卡片绘制 + 缩略图预取行区间）、`row.rs`（列表行与右键菜单项）、`toolbar.rs`（顶部栏：面包屑 / 搜索框 / 视图切换 / 操作区）；顺带**渲染与动作分离**：`FilesPage::show(...)` 只读写自身状态并返回 `Vec<FilesAction>`，15 个跨域动作（弹窗 / 预览 / 播放 / 下载 / 分享 / 上传 / 搜索）由 `App::apply_files_action` 在渲染后统一执行，页面不再需要 `&mut App`。纯结构调整：用户可见行为零变化（唯一差异是取消选下载目录时不再少画一帧列表）
- **工程：界面层按域 struct 化**：`app/` 的七个域收进各自模块 —— `app/global.rs::Global`（命令通道 / 提示条 / KDE 配色与主题轮询）、`app/trash.rs::TrashPage`、`app/shares.rs::SharesPage`（我的分享 + 转存分享）、`app/preview.rs::PreviewPage`（下载进度 / 清晰度 / 外部打开探针）、`app/search.rs::SearchPage`、`app/thumbs.rs::ThumbsPage`（纹理 LRU + 在途 / 失败登记）、`app/tasks/mod.rs::TasksPage`（离线任务 + 「保存到」目录选择器，后续 P2-15 拆为 `app/tasks/` 目录）；`drain` 的 60 个 `Msg` 臂全部收敛为一行转调，`app/mod.rs` 3085 → 2127 行。纯结构调整，用户可见行为零变化
- **工程：后台线程按域拆分**：`worker.rs`（2460 行）拆成 `worker/` 下 12 个文件（`mod` / `gate` / `cache` / `download` / `upload` / `preview` / `thumbs` / `files` / `tasks` / `shares` / `trash` / `auth`），命令循环 `handle` 的 43 个 `Cmd` 分支收敛为一行转调，入口 `worker/mod.rs` 降至 367 行。纯结构调整：用户可见行为、协议与并发语义零变化
- **工程：界面层纯逻辑下沉**：`app/mod.rs` 的四个无界面依赖函数（下载记录状态映射 / 目录卡片进度聚合 / 目录树文件计数上卷 / 速率取样）连同单测迁到新模块（现为 `app/transfers/model.rs`，另补两项覆盖：非终态兜底、短间隔不取样），`App::new` 的启动逻辑抽成 `restore_req_id` / `start_session`。纯结构调整，用户可见行为零变化
- **工程：打包脚本归位 `appimage/` 与 `flatpak/`**：`build-appimage.sh` 收进 `packaging/appimage/`、`build-flatpak.sh` 收进 `packaging/flatpak/`（`kichi.desktop` 留根，AppImage / Flatpak 共用），脚本内路径与发布工作流同步调整；本地出包命令变为 `./packaging/appimage/build-appimage.sh` 与 `./packaging/flatpak/build-flatpak.sh`。纯结构调整：出包行为与产物不变

### 移除

- **移除 `packaging/install-icon.sh`**：桌面文件与图标现由发行包自带（AppImage 打进 AppDir、Flatpak 由清单导出），该脚本只服务「源码构建后直接跑二进制」一条路径，予以删除；README 的「桌面图标」一节并入打包说明，需要时可按说明手工放置 `packaging/kichi.desktop` 与 `assets/kichi.svg`

### 修复

- **CI 因新版 Rust 的 future-incompat lint 在 clippy 步骤编译失败**：CI 的 `dtolnay/rust-toolchain@stable` 拉到 rustc 1.98.1，其新增 `float_literal_f32_fallback`（[rust#154024](https://github.com/rust-lang/rust/issues/154024)，默认告警）命中仓库里 55 处 `Stroke::new(<裸浮点>, …)` —— egui 0.31 的 `Stroke::new(width: impl Into<f32>, …)` 让字面量走“回退到 f32”这条将被移除的旧推断路径；叠加 clippy 的 `-D warnings` 后报 `could not compile kichi-gui … due to 55 previous errors`。本地 rustc 1.94 尚无该 lint，故“本机绿、CI 红”。修复：55 处宽度字面量补 `_f32` 后缀（纯类型标注，渲染行为不变），并新增根 `rust-toolchain.toml` 固定 `1.98.1`，`ci.yml` / `release.yml` 的 `dtolnay/rust-toolchain@版本` 一并固定，避免再次静默漂移
- **文件页顶部面包屑不再溢出到右侧操作区**：顶部栏改为先在整行内从右往左按实际宽度排布右侧控件，再把返回 / 面包屑 / 计数 / 剪贴板嵌在其后占用剩余宽度，去掉原先写死的 360px 右侧预留（右侧控件变宽时面包屑不再被反向覆盖）；并修正面包屑宽度估算的两处漏算 —— ① `allocate_exact_size` 会在每个矩形后追加一次 `item_spacing`（本主题 10px），② 末级目录名截断时未为「…」与分隔符预留宽度 —— 二者叠加会让长目录名把「· N 项」计数挤进右侧视图按钮；计数预留宽度也改为按实际文案测量（选中态「已选 x/y 项」更长也能覆盖）
- **Flatpak 首次 CI 出包失败（缺 SVG 加载器）**：flatpak 导出阶段用宿主 gdk-pixbuf 校验图标，Ubuntu 24.04 的 gdk-pixbuf 把 SVG 支持放在 `librsvg2-common` 里 —— 发布工作流的 apt 列表补装该包（首次发 `v1.0.0` 时 AppImage 成功、Flatpak 编译安装都通过，仅导出报 `Format not recognized`）；Fedora / gdk-pixbuf ≥ 2.44 已内置 SVG 加载器，本地构建不受影响
- **发布任务定位不到仓库**：`release` job 只取 artifacts、不 checkout，`gh` 无从推断目标仓库（报 `fatal: not a git repository (or any of the parent directories): .git`）—— 给该步骤补 `GH_REPO` 环境变量（同一次发版中 AppImage / Flatpak 构建与上传均已通过，仅此一步失败）

## [1.0.0] - 2026-09-19

### 新增

- **登录 / 账户**：邮箱 / 手机号密码登录（captcha 流程）、会话持久化与自动续期、「记住密码」写入系统密钥环
- **文件管理**：目录浏览与分页、排序、名称过滤、列表 / 网格双视图、新建 / 重命名 / 移动，复制 / 剪切 / 粘贴
- **本地上传**：gcid 秒传 + 阿里云 OSS 分片（并发、运行期内断点续传）、目录递归上传、上传历史持久化
- **本地下载**：`.part` + Range 断点续传、原子改名、完整性校验、退避重试、下载历史持久化；整目录递归下载（按云端层级建本地目录，传输任务页聚合为单张目录卡片，可展开为目录树：子目录可单独折叠、卡片显示「文件 已完成/全部」）
- **离线下载**：磁力 / 直链转存、状态页签与自适应轮询、「加载更多」分页、多选批量（重试 / 删除）
- **分享**：创建与管理自己的分享；解析他人分享链接并转存到自己网盘
- **回收站**：列出 / 还原 / 彻底删除 / 一键清空
- **文件预览**：`mpv` 流式播放（清晰度选择 + 外挂字幕）、其它文件交系统查看器；非媒体预览下载期间显示进度条并可随时取消
- **界面与体验**：侧边栏导航、KDE 主题跟随、中文字体与任务角标
- **设置 · 缓存**：展示预览 / 缩略图缓存占用，并支持一键清空
- **发行包与发布 CI**：新增 AppImage 与 Flatpak 打包 —— `packaging/build-appimage.sh`（组装 AppDir + 固定版本 appimagetool 出包）与 `packaging/build-flatpak.sh` + `packaging/flatpak/io.github.lyndon0na.Kichi.yml`（freedesktop 25.08 沙箱内源码构建，`--install` 可直接装入用户级 flatpak）；`.github/workflows/release.yml` 在推 `v*` tag 时自动构建两者并发 Release，手动触发只产出 workflow artifacts。Flatpak 内做了宿主桥接：音视频播放仍用**宿主**已装的 `mpv`（`flatpak-spawn --host`）、中文字体取自宿主（`/run/host/fonts`）、窗口 `app_id` 取 `FLATPAK_ID`（任务栏图标可正常关联）、界面配色照旧跟随 KDE `kdeglobals`

### 变更

- **本地选择框记住上次用过的目录**：上传文件 / 上传文件夹的系统选择框不再从进程工作目录起（桌面启动时不可控），而是从上次确认选择的位置继续（文件多选取所在目录、文件夹选择取所选目录本身）；记录随设置持久化，目录已被删除时回退到主目录
- **预览入口按类型分层**：压缩包 / 镜像 / 可执行 / 种子不再出现「打开」（只留「下载到本地」，双击时提示改用下载）；非媒体预览超过 64 MiB 且未命中缓存时先弹确认（需整份下载）；文件类型判定收敛为单点，`mime_type` 参与判定 —— `.ts` 之类二义扩展名不再被误送播放器，改名成 `.bin` / 无扩展名的视频也能走播放链路
- **离线任务轮询**：自动刷新节拍随任务活动自适应（存在等待 / 下载中任务时约 3s，否则约 60s）；配额轮询降频，并由登录 / 删除 / 上传完成等操作显式刷新
- **日志与磁盘缓存占用**：`kichi.log` 改为按大小轮转（单文件 1 MiB、保留 3 个备份，`KICHI_LOG_MAX_MB` / `KICHI_LOG_FILES` 可覆盖）；预览缓存与缩略图缓存按「最久未使用」自动淘汰（上限 512 MiB / 200 项、128 MiB / 1000 项），不再无限增长；正在预览或正在传输的条目不会被删
- **仓库链接**：`Cargo.toml` 补回 `repository` 字段（workspace 定义、两个 crate 继承，`cargo metadata` 可读到）；README 顶部徽章全部改为可点击的仓库链接，其中版本徽章与新增的打包状态徽章跟随 GitHub Release / Actions 自动更新（不再手写版本号），免责声明末尾补 Issues 与 LICENSE 链接

### 修复

- **打开文件不再假成功**：系统没有关联程序时 `xdg-open` 会正常启动却打开失败，以前界面仍提示「已打开」；现在等待退出码并提示「系统未关联打开「x」的程序」，其它失败回显 stderr 首行（打开目录成功仍保持安静）
- **缩略图加载不再拖慢后台**：缩略图下载不再占住后台命令循环（原先在大图目录下会连带卡住目录加载 / 预览 / 删除 / 配额刷新），改为独立任务并限并发；只请求可见区上下各一屏，滚出视野的画面不再继续下载，切换目录 / 刷新时在跑的请求即被取消
- **缩略图失败不再永久留空**：仅对瞬时错误（超时 / 断连 / 5xx）退避重试 3 次，404 / 403 之类直接放弃；失败会回执给界面，不再静默留一个空洞，也不再让该文件本次会话彻底失去重试机会（刷新目录或重新搜索可再试）
- **没有缩略图的文件不再刷失败日志**：服务端对这类文件（多为非图片 / 视频）会下发空串 / 相对路径之类的不可用链接，以前每个文件都要失败重试 3 次、写 3 行日志（实际零网络开销）；现在直接判定为「无缩略图」并回退类型图标，默认不写日志（需要排查时用 `KICHI_LOG=kichi_gui=trace`），真失败也只记一行并带错误来源链
- **缩略图缓存写入**：改为 `.part` + rename 原子落盘并限制响应体大小，避免中途失败留下的截断文件被当成有效缓存（解码必然失败且永不重下）
- **缩略图显存不再无限增长**：以前纹理按服务端下发的原始尺寸上传 GPU（实测 720×405，约 1.17 MB/张）且只增不删，长时间浏览大图目录显存持续上涨、切目录也不释放；现在解码前按卡片所需尺寸降采样（最长边 136–272 物理像素，单张降到 41 KB 起），纹理缓存再按 64 MiB / 512 张的软上限做「最久未用」淘汰 —— 视野里的图不会被淘汰，滚回来时命中磁盘缓存、无网络开销
