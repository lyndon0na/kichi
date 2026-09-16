# PROJECT_PLAN

PikPak Linux 客户端开发计划与进展记录。

## 一、目标

做一个可在 Linux 桌面上用的 PikPak 云盘客户端，覆盖常用能力，技术栈选用 Rust（性能好、单二进制、便于长期维护）。

- 形态：桌面 GUI（egui/eframe）
- 首发范围：登录 / 文件浏览管理 / 磁力离线下载

## 二、技术选型

| 项 | 选择 | 理由 |
| --- | --- | --- |
| 语言 | Rust (edition 2021) | 内存安全、体积小、生态成熟 |
| GUI | egui + eframe 0.31 | 纯 Rust、迭代快、依赖少 |
| HTTP | reqwest 0.13（native-tls） | 异步、成熟 |
| 异步 | tokio | GUI 后台线程跑网络，channel 通信 |
| JSON | serde / serde_json | 配合防御式解析 |

> 注：曾评估 eframe 0.35，其 `App` trait 与面板系统大改（`update` → `ui`、移除 `TopBottomPanel`），为避免在全新 API 上踩坑，固定使用 API 稳定的 0.31。

## 三、里程碑

- [x] M0 调研 API：端点、加签算法、登录/文件/离线任务的数据结构
  - 参考社区逆向：`Quan666/PikPakAPI`(py)、`Muione/PikpakAPI`(ts) 交叉验证
  - 用固定输入 + python 生成 golden 向量，保证 `captcha_sign`/`device_sign` 正确
- [x] M1 `pikpak-core`：常量/签名、错误模型、HTTP 层、模型、业务方法、会话持久化、单测
- [x] M2 `pikpak-gui`：后台 worker + 登录页 + 文件页 + 离线任务页，真机登录联调
- [x] M3 联调修复：
  - `error_code=16` → 自动 refresh 重试
  - `error_code=9` → 按操作 action 重新 captcha_init 后重试（文件列表恢复）
  - `blocking_lock` 在 tokio 线程内 panic → 改 async 获取锁
- [x] M4 UI 打磨：多选/全选、排序、过滤、右键菜单、类型标签、快捷键、任务角标
- [x] M5 体验：深浅主题、记住账号、离线保存位置选择、任务组清空
- [x] M6 本地下载：文件/离线任务直连本地；`.part` + Range 断点续传、原子改名、
      瞬时错误退避重试、进度/速率/取消、同名去重、下载目录记忆
- [x] M7 UI 重构：左侧边栏导航 + 底部存储配额卡；文件列表类型图标与可点击排序表头；
      登录 / 任务 / 设置卡片化；统一明暗主题配色参数（`theme.rs` / `icons.rs`）
- [x] M8 体验继续：
  - 目录缓存（SWR，导航命中零请求）+ 文件列表 / 图标双视图 + 列宽拖拽
  - 本地下载并入「传输任务」页（上传 / 下载分栏、状态筛选、批量操作、虚拟滚动卡片）
  - 下载历史持久化（`downloads.json`，最多 200 条，启动时恢复任务列表）
  - 主题跟随系统 KDE 配色（`kde.rs`，定时轮询），Breeze 风格适配；非 KDE 回退内置浅色
  - 应用图标 / 桌面项（`app_id` 关联）、预览按需解析清晰度并缓存、外挂字幕自动挂载
  - 会话写入原子化 + 续期单飞；「记住密码」存系统密钥环（Secret Service），失效自动重登
- [x] M9 本地上传：gcid 秒传哈希 + 阿里云 OSS 分片（HMAC-SHA1，无 AWS SDK）；
      文件页 / 传输任务页「上传」入口，进度 / 速率 / 取消 / 重试，秒传命中直接完成；
      分片并发（4）与断点续传（重试间保留 upload_id/已传分片）、目录递归上传

## 四、现状（功能清单）

### 已实现
- 账号密码登录（含 captcha）、会话持久化/恢复/自动续期、退出登录
- 目录浏览（面包屑）、排序（表头点击切换升/降序）、过滤、分页加载；
  目录缓存（SWR 命中零请求）+ 列表 / 图标（网格）双视图 + 列宽拖拽
- 新建文件夹 / 重命名 / 移入回收站（支持多选、右键、Delete 键）；回收后本地即时隐藏，
  服务端列表同步完成后自动解除
- 复制 / 剪切后再到目标目录粘贴（支持多选、右键、工具栏与 Ctrl+C/X/V 快捷键；
  目录空白处右键可粘贴 / 新建文件夹 / 刷新，文件夹右键可「粘贴到此处」，
  剪贴板为空时不显示粘贴项）；移动成功后本地即时隐藏原目录项，
  并按服务端列表最终一致性延迟重列以避免文件残留
- 界面：左侧边栏导航（我的文件 / 我的分享 / 回收站 / 离线下载 / 传输任务 / 设置）
  + 账户/存储配额卡；文件类型矢量图标（文件夹/视频/图片/音频/文档/压缩）
- 传输任务页：上传 / 下载分栏；下载卡片带进度/速率/状态筛选/批量操作；上传卡片带进度/取消/重试
- 本地上传：gcid 秒传 + 阿里云 OSS 分片(HMAC-SHA1，并发 + 断点续传)，进度/速率/取消/退避重试；秒传命中直接完成；
  入口：文件页工具栏/右键、传输任务页（Wayland 下不支持拖拽，winit 未实现 Wayland 拖放）；目录递归上传（云端建目录 + 逐个文件）；
  上传历史持久化（`uploads.json`，最多 200 条，启动时恢复列表）
- 离线下载：磁力/直链转存，等待/下载/完成/失败分桶展示与自动轮询
- 失败重试、删除任务、整组清空、保存目录可选
- 本地下载：文件页选中/右键、离线「已完成」任务一键下载；`.part`+Range 断点续传、
  原子改名、瞬时错误退避重试、并发上限 3、进度/速率/取消、同名自动 ` (n)` 去重、
  下载目录记忆、下载历史持久化（`downloads.json`，最多 200 条，启动时恢复）
- 文件预览：双击或右键「播放」；子菜单列出云端清晰度(原画/1080P/720P…，无转码流时仅原画，
  打开子菜单时按需解析)，选择后由 `mpv` 流式播放(带签名直链请求头)，自动挂载同集外挂字幕；
  其他文件下载到 `~/.cache/pikpak-linux/preview` 后交给系统查看器(`xdg-open`)，
  命中缓存不重复下载，缺少 `mpv` 时仅提示安装
- 主题：读取系统 KDE 配色（`kde.rs`）自动跟随明暗与强调色并适配 Breeze 风格，
  非 KDE 环境回退内置浅色；任务/下载角标、记住账号、中文界面与 CJK 字体自动加载
- 会话：原子写会话文件、续期单飞；「记住密码」经系统密钥环存储，会话失效自动重新登录

### 未实现 / 已知边界
- 分享转存（「我的分享」为占位）/ 搜索
- 回收站页面为占位：可移入回收站，但暂不支持在应用内浏览 / 还原
- 图标（网格）视图使用文件类型图标，暂不加载真实缩略图
- 本地整目录下载(会跳过文件夹)
- 离线任务长列表翻页；任务字段为防御式解析，展示信息有限
- 部分高危操作需网页人机验证时只能失败提示

## 五、关键实现笔记

### 1) 登录前必须走 captcha
`POST user.mypikpak.com/v1/shield/captcha/init` 先拿 `captcha_token`：
- 登录用：meta 带邮箱/手机号标识
- 一般操作用：meta 需带 `captcha_sign`，由
  `1.` + 对 `client_id+client_version+package+device_id+timestamp` 做 15 轮盐值 MD5 得到

### 2) token 失效判定
响应含 `error_code==16` → 用 `grant_type=refresh_token` 换新 token 后重试一次。

### 3) captcha token 作用域
`error_code==9 (Verification code is invalid)` 通常表示当前 captcha token 不是为该操作
（action = `METHOD:/drive/...`）签发的。解决方案：按失败请求推导 action → 重新
`captcha_init` → 原请求重试一次。注意 `captcha_init` 必须走裸请求，避免异步递归。

### 4) 离线任务状态无字段保证
离线任务 JSON 未公开、字段不稳定，因此按 `phase`（PENDING/RUNNING/COMPLETE/ERROR）
分四次查询分桶展示，状态以请求结果为准。

### 5) 设备身份
`X-Device-Id`、`User-Agent`(ANDROID 风格，含 device_sign) 需与 captcha/登录保持一致，
device id 每次登录生成并持久化。

### 6) GUI 与网络隔离
网络在独立 tokio worker 线程中运行；UI 与 worker 通过 `mpsc` 交换 `Cmd`/`Msg`。
UI 每次帧 `try_recv` 收敛消息，操作即时性由点击 → 发送 → 回包驱动。

### 7) 本地下载实现要点(参考社区 pikpakcli/pikpak-downloader)
- 解析直链: 对该文件 id 做 `captcha_init(action=GET:/drive/v1/files/{id})`, 再
  `GET /drive/v1/files/{id}`, 响应里的 `web_content_link` 即带签名的限时直链;
  captcha token 用后即焚, 避免影响后续普通请求; 详情 kind 含 folder 时明确报错。
- 下载写 `<文件>.part`, 完成后 rename 收尾(原子); `.part` 存在时用
  `Range bytes={len}-` 续传, 服务端回 200(忽略 Range) 或 416(part 失效)时从头/截断重来。
- 直链对 CDN 无需鉴权头; 若返回 401/403 再补 Bearer 重试一次。
- 完成后按详情声明大小校验, 收到字节不足则判为截断、保留 part 触发续传, 绝不落残缺文件。
- 解析直链与传输的瞬时错误(断连/超时/5xx/429/传输不完整)统一按退避重试,
  每次重试前重新解析直链(限时)。

### 8) 会话持久化与记住密码
- `session.json` 保存 access/refresh token 与 device id；续期会重写它。写入采用
  「临时文件 + rename」原子替换并加进程锁串行化，避免下载/预览与定时刷新并发续期
  把文件写坏（写坏会导致下次启动无法恢复、被迫重新登录）。
- `refresh_token` 刷新加单飞锁，防止并发 401 触发重复刷新；refresh 被服务端拒绝时
  归类为 `AuthExpired`，UI 明确回到登录页而非停留在「假登录」。
- 「记住密码」用 `keyring`（Secret Service：KDE Wallet / GNOME Keyring）保存账号密码，
  仅在手动登录成功且勾选时写入；会话失效或首启无会话时用它自动重登。密钥环不可用或
  无条目时静默回落到登录表单。密码不写入任何配置文件。

## 六、质量
- `cargo test --workspace`：核心库 16 项测试 + GUI 10 项通过（加签 golden 向量、JSON 解析、
  token 边界、直链解析回退、清晰度解析、文件名净化、batchMove/batchCopy 请求体、媒体类型识别）
- `cargo check --workspace` / `cargo clippy --workspace --all-targets`：零 warning
- 真机验证：登录、自动续期、目录加载、离线任务查询（用户账户实测）；本地下载链路
  依据社区逆向实现, 直链字段以 `web_content_link` 优先, 采用防御式解析降低变更风险

## 七、路线图（后续）

1. **上传（已完成，见 M9 / 第九节）**：目录递归、分片并发、运行期内断点续传均已实现；
   后续可加：跨重启续传（持久化 upload_id/凭据）、多选批量队列优化
2. **分享 / 回收站**：
   - mypikpak 分享链接解析与转存（`/drive/v1/share` 系列）；生成自己的分享链接
   - 回收站浏览 / 还原 / 彻底删除
   —— 端点见第九节 6)
3. **体验继续**：真实缩略图、本地整目录下载(递归)、任务详情进度、全局搜索
4. **分发**：完善 `desktop` 文件与图标、rpm/AppImage 打包、发布构建 CI

## 八、环境
- 开发机：Fedora 44，KDE Plasma，Wayland
- 工具链：cargo/rustc 1.94

## 九、上传实现方案（M9 细化）

参考实现：
- `Bengerthelorf/pikpaktui`（纯 Rust，上传/下载/分享，**首选**）
- `52funny/pikpakcli`（Go，上传 + 分享）
- `52funny/pikpakhash`（Go，gcid 算法）
- `rclone/rclone backend/pikpak`（Go，gcid 交叉验证）

### 1) gcid（秒传哈希）
分块 SHA1，再对「各块 sha1 原始摘要的拼接」取 SHA1（三处实现 + rclone 一致）：
- 块大小：256KB 起，`size/psize > 512` 且 `psize < 2MB` 时翻倍
  （即 ≤128MB→256KB，≤256MB→512KB，≤512MB→1MB，>512MB→2MB）
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
- 需 captcha；`error_code=9` 时按 action `POST:/drive/v1/files` 重新 captcha 后重试一次
  （本项目 `request_inner` 已内置该恢复）
- `file.phase == "PHASE_TYPE_COMPLETE"` → 秒传命中，直接结束
- `file.phase == "PHASE_TYPE_PENDING"` → 取 `resumable.params`
  （`access_key_id` / `access_key_secret` / `bucket` / `endpoint` / `key` / `security_token`）

### 3) OSS 分片上传（Aliyun OSS HMAC-SHA1，无需 AWS 签名/SDK）
- 发起：`POST https://{endpoint}/{key}?uploads` → 解析 XML `<UploadId>`
- 分片：`PUT https://{endpoint}/{key}?partNumber={N}&uploadId={ID}` → 读响应头 `ETag`（去引号）
  - 块大小 `max(ceil(size/10000), 5MB)`，可并发（本项目沿用下载的并发上限思路）
- 完成：`POST https://{endpoint}/{key}?uploadId={ID}`
  体 `<CompleteMultipartUpload><Part><PartNumber>N</PartNumber><ETag>E</ETag></Part>…`
- 签名 `Authorization: OSS {ak}:{sig}`，`sig = base64(HMAC-SHA1(secret, string_to_sign))`：
  ```
  METHOD\n
  \n                         // Content-MD5 空
  application/octet-stream\n // Content-Type
  <RFC1123 GMT Date>\n
  x-oss-security-token:{token}\n
  /{bucket}/{key}?{raw_query}  // query 顺序须与请求一致
  ```
  请求头另需 `Date` / `Content-Type: application/octet-stream` / `x-oss-security-token`
- 分片成功后**无需等 task**（pikpaktui / pikpakcli 均不上报 task），清理目录缓存即可

### 4) 依赖
`sha1 = "0.10"`、`hmac = "0.12"`、`base64 = "0.22"`（`Date` 头可手写或加 `httpdate`）。

### 5) 落地步骤
- `pikpak-core`：新增 `upload.rs`（gcid / OSS 签名 / 分片）+ `client.rs` 的
  `upload_create` / `oss_initiate` / `oss_upload_part` / `oss_complete`；模型防御式解析。
- `pikpak-gui`：`Cmd::StartUpload`、`Msg::Ul*`、`worker::spawn_upload`
  （并发信号量 / 取消 / 退避重试 / 进度）、`helpers::pick_files`、
  文件页「上传到此处」、传输任务页上传分栏接真实列表。
- 测试：gcid 黄金向量（与 pikpakhash/pikpaktui 对拍）、签名串黄金向量、创建响应解析、
  0 字节 / 秒传 / 分片完成分支。

### 6) 分享 / 回收站端点（M10）
- 分享信息：`GET /drive/v1/share?share_id=&pass_code=` → `pass_code_token` / `title` / `share_status`
- 分享目录：`GET /drive/v1/share/detail?share_id=&parent_id=&pass_code_token=&limit=100[&page_token=]`
- 转存：`POST /drive/v1/share/restore` `{share_id, pass_code_token, file_ids, to:{parent_id}}` → `restore_status` / `restore_task_id`
- 创建分享：`POST /drive/v1/share` `{file_ids, share_to, expiration_days, pass_code_option}`
- 我的分享：`GET /drive/v1/share/list`；删除：`POST /drive/v1/share:batchDelete`
- 回收站：`file_list` filters `trashed.eq=true`；`files:batchUntrash` / `files:batchDelete`
