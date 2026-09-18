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
> M0–M15 已全部完成；细节分别见[第四节](#四功能现状)与[第五节](#五关键实现笔记)。

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
  - 分片并发（4）与断点续传（重试间保留 upload_id / 已传分片）、目录递归上传
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
- 文件类型矢量图标（文件夹 / 视频 / 图片 / 音频 / 文档 / 压缩）；网格视图加载真实缩略图（`thumbnail_link` 预签名直链下载，磁盘缓存 `~/.cache/kichi/thumbnails/`，`image` crate 解码为 RGBA 后上传 GPU 纹理）；Ctrl + 滚轮缩放网格大小（80–160px），图标 / 文字 / 缩略图随卡片等比缩放

</details>

<details>
<summary><b>上传 · 下载 · 传输任务</b></summary>

**本地上传**
- gcid 秒传哈希 + 阿里云 OSS 分片（HMAC-SHA1，**并发 + 断点续传**），进度 / 速率 / 取消 / 退避重试；秒传命中直接完成
- 入口：文件页工具栏 / 右键、传输任务页（Wayland 下不支持拖拽，winit 未实现 Wayland 拖放）
- 目录递归上传：云端建目录 + 逐个文件
- 上传列表：目标路径、速率 / ETA、完成时间、状态筛选、全选批量取消 / 重试 / 移除、清除已完成、在网盘中打开
- 上传历史持久化（`uploads.json`，最多 200 条，启动时恢复列表）

**本地下载**
- 文件页选中 / 右键、离线「已完成」任务一键下载
- 整目录递归下载：选中文件夹（工具栏多选或右键）后遍历云端目录树，本地按云端层级建同名目录（含空目录）
- `.part` + Range 断点续传、原子改名、瞬时错误退避重试、并发上限 3、进度 / 速率 / 取消、同名自动 ` (n)` 去重、下载目录记忆
- 下载历史持久化（`downloads.json`，最多 200 条，启动时恢复列表）；目录任务保存为一条记录并内联子文件快照

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
- 双击或右键「播放」；视频子菜单列出云端清晰度（原画 / 1080P / 720P…，无转码流时仅原画，打开子菜单时按需解析并缓存），选择后由 `mpv` 流式播放（带签名直链请求头），自动挂载同集外挂字幕
- 音频只有原文件，直接播放
- 其他文件下载到 `~/.cache/kichi/preview` 后交系统查看器（`xdg-open`），命中缓存不重复下载
- 缺少 `mpv` 时仅提示安装

**界面与主题**
- 左侧边栏导航（我的文件 / 我的分享 / 回收站 / 离线下载 / 传输任务 / 设置）+ 账户 / 存储配额卡
- 读取系统 KDE 配色（`kde.rs`）自动跟随明暗与强调色并适配 Breeze 风格，非 KDE 回退内置浅色
- 任务 / 下载角标、记住账号、中文界面与 CJK 字体自动加载
- 网格视图真实缩略图（磁盘缓存 + GPU 纹理），Ctrl + 滚轮缩放网格大小（80–160px），图标 / 文字 / 缩略图随卡片等比缩放

</details>

### 未实现 / 已知边界

| 项 | 说明 |
| :-- | :-- |
| 搜索 | PikPak 无服务端全局搜索 API，采用客户端递归遍历所有目录并按文件名模糊匹配（大小写不敏感） |
| 离线任务展示 | 任务字段防御式解析，仅展示名称 / 大小等有限信息（无进度百分比等） |
| 人机验证 | 部分高危操作被要求网页人机验证时只能失败提示 |
| 上传跨重启续传 | 仅覆盖单文件上传；目录递归上传本轮按整目录管理、暂不续传。续传复用持久化 gcid 且仅校验文件大小，不重算 gcid；重启后依赖重新申请 STS 凭证并对账（`oss_list_parts`），位置不符 / upload_id 失效 / 本地文件变动均安全退回全新上传 |

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

## 六、质量

| 检查 | 结果 |
| :-- | :-- |
| `cargo test --workspace` | **46 项通过**（核心库 27 + GUI 19） |
| `cargo check --workspace` | 零 warning |
| `cargo clippy --workspace --all-targets` | 零 warning |
| `cargo fmt --all -- --check` | 零差异（配置见 `rustfmt.toml`） |
| 真机验证 | 登录、自动续期、目录加载、离线任务查询（用户账户实测） |

<details>
<summary>测试覆盖范围</summary>

加签 golden 向量、JSON 解析、token 边界、直链解析回退、清晰度解析、文件名净化、batchMove / batchCopy 请求体、媒体类型识别、分享列表 / 创建响应解析、回收站 trashed 过滤条件；目录下载的本地路径拼接 / 净化、目录子文件聚合进度与状态推导、目录树子目录计数上卷。

</details>

> [!NOTE]
> 本地下载链路依据社区逆向实现，直链字段以 `web_content_link` 优先，采用防御式解析降低变更风险。

## 七、路线图

- [x] **上传** —— 见 M9 与[第九节](#九附录实现方案)。已实现目录递归、分片并发、运行期内断点续传，以及**跨重启续传**（`upload_resume.json` 持久化 upload_id / OSS 位置 / 已传分片 ETag，重启后刷新 STS 凭证并用 `oss_list_parts` 对账，位置不符或失效则安全退回全新上传；单文件已支持，目录递归暂缓）；后续可加：目录递归续传、多选批量队列优化
- [x] **分享 / 回收站** —— 端点见[第九节 6)](#6-分享--回收站端点m10--m12)
  - 生成自己的分享链接 + 「我的分享」管理（创建 / 列出 / 复制 / 取消）
  - 分享转存：解析 mypikpak 分享链接并保存到我的网盘（`share` / `share/detail` / `share/restore`），含分页 / 过滤 / 目标目录 / 移动重试；转存暂存目录（「转存自分享」）按持久化 ID 定位，ID 失效时回退名称匹配并刷新缓存
  - 回收站浏览 / 还原 / 彻底删除（含清空）
- [ ] **体验继续** —— 全局搜索、任务详情进度
- [ ] **分发** —— 完善 `desktop` 文件与图标、rpm / AppImage 打包、发布构建 CI

## 八、环境

| 项 | 值 |
| :-- | :-- |
| 开发机 | Fedora 44，KDE Plasma，Wayland |
| 工具链 | cargo / rustc 1.94 |

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
