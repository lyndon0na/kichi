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
- [x] M7 UI 重构：左侧边栏导航（网盘文件 / 传输任务 / 本地下载 / 设置）+ 底部存储
      配额卡；文件列表类型图标与可点击排序表头；本地下载改为侧边栏常驻页；
      登录 / 任务 / 设置卡片化；统一明暗主题配色参数（`theme.rs` / `icons.rs`）

## 四、现状（功能清单）

### 已实现
- 账号密码登录（含 captcha）、会话持久化/恢复/自动续期、退出登录
- 目录浏览（面包屑）、排序（表头点击切换升/降序）、过滤、分页加载
- 新建文件夹 / 重命名 / 移入回收站（支持多选、右键、Delete 键）；回收后本地即时隐藏，
  服务端列表同步完成后自动解除
- 复制 / 剪切后再到目标目录粘贴（支持多选、右键、工具栏与 Ctrl+C/X/V 快捷键；
  目录空白处右键可粘贴 / 新建文件夹 / 刷新，文件夹右键可「粘贴到此处」，
  剪贴板为空时不显示粘贴项）；移动成功后本地即时隐藏原目录项，
  并按服务端列表最终一致性延迟重列以避免文件残留
- 界面：左侧边栏导航 + 账户/存储配额卡；文件类型矢量图标（文件夹/视频/图片/音频/文档/
  压缩）；本地下载为侧边栏常驻页（带进行中角标、进度卡片、取消/移除/打开目录）
- 离线下载：磁力/直链转存，等待/下载/完成/失败分桶展示与自动轮询
- 失败重试、删除任务、整组清空、保存目录可选
- 本地下载：文件页选中/右键、离线「已完成」任务一键下载；`.part`+Range 断点续传、
  原子改名、瞬时错误退避重试、并发上限 3、进度/速率/取消、同名自动 ` (n)` 去重、
  下载目录记忆
- 文件预览：双击或右键「播放」；子菜单列出云端清晰度(原画/1080P/720P…，无转码流时仅原画，
  打开子菜单时按需解析)，选择后由 `mpv` 流式播放(带签名直链请求头)，自动挂载同集外挂字幕；
  其他文件下载到 `~/.cache/pikpak-linux/preview` 后交给系统查看器(`xdg-open`)，
  命中缓存不重复下载，缺少 `mpv` 时仅提示安装
- 深浅主题（统一配色参数）、任务/下载角标、记住账号、中文界面与 CJK 字体自动加载

### 未实现 / 已知边界
- 本地上传 / 分享转存 / 搜索；本地整目录下载(会跳过文件夹)
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

## 六、质量
- `cargo test --workspace`：核心库 16 项测试 + GUI 10 项通过（加签 golden 向量、JSON 解析、
  token 边界、直链解析回退、清晰度解析、文件名净化、batchMove/batchCopy 请求体、媒体类型识别）
- `cargo check --workspace` / `cargo clippy --workspace --all-targets`：零 warning
- 真机验证：登录、自动续期、目录加载、离线任务查询（用户账户实测）；本地下载链路
  依据社区逆向实现, 直链字段以 `web_content_link` 优先, 采用防御式解析降低变更风险

## 七、路线图（后续）

1. **上传**：本地文件分块上传到当前目录
2. **分享**：mypikpak 分享链接解析与转存（`/drive/v1/share` 系列）；生成自己的分享链接
3. **体验继续**：缩略图/网格视图、本地整目录下载(递归)、任务详情进度、多列批量
4. **分发**：`desktop` 文件、图标、rpm/AppImage 打包、发布构建 CI

## 八、环境
- 开发机：Fedora 44，KDE Plasma，Wayland
- 工具链：cargo/rustc 1.94
