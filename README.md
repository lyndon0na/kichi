# Kichi — PikPak Linux 客户端

PikPak 云盘的 Linux 桌面客户端（非官方）。基于逆向的私有 API 实现，Rust 编写，使用 egui/eframe 构建界面。

> ⚠️ 非官方项目，接口可能随服务端变更而失效；仅供学习与个人使用。

## 界面

桌面 GUI 采用「左侧边栏 + 右侧内容区」布局：

- **侧边栏导航**：我的文件 / 我的分享 / 回收站 / 离线下载 / 传输任务 / 设置；
  底部常驻账户与存储空间配额卡片
- **文件页**：按类型矢量图标（文件夹 / 视频 / 图片 / 音频 / 文档 / 压缩包）、
  可点击排序表头（名称 / 大小 / 修改时间）、可拖拽调整列宽、
  列表 / 图标（网格）两种视图、悬浮与选中高亮、搜索过滤
- **传输任务**：上传 / 下载分栏；下载页带状态筛选、批量选择与图标操作，上传页带进度 / 取消 / 重试
- **主题**：自动读取系统 KDE 配色（`kdeglobals`）并跟随明暗与强调色，
  非 KDE 环境回退到内置浅色主题；统一圆角与配色参数（`theme.rs`）、登录居中卡片

> 注：文件行操作以右键菜单为主（下载 / 播放 / 重命名 / 复制 / 剪切 / 粘贴到此处 /
> 移入回收站），双击文件夹进入、双击文件预览、单击选中（Ctrl 单击多选 / Shift 范围选择）。

## 功能

- **登录 / 账户**
  - 邮箱 / 手机号密码登录（含 PikPak 的 captcha 流程）
  - 登录态本地持久化、自动恢复与自动续期（access token 过期自动 refresh）
- **文件浏览 / 管理**
  - 目录浏览（面包屑导航、进入/返回），带目录缓存与分页加载
  - 列表 / 图标（网格）视图切换；列表表头排序（名称 / 大小 / 修改时间，升/降序）与列宽拖拽
  - 当前目录按名称过滤
  - 新建文件夹、重命名、多选 / 批量移入回收站（回收后本地即时移除，不依赖服务端同步）
  - 复制 / 剪切后再到目标目录粘贴（移动 / 复制到目标目录）
  - 右键菜单、快捷键（F5 刷新 / Ctrl+A 全选 / Delete 删除 / Ctrl+C/X/V / Esc 取消）
  - 文件类型矢量图标
- **文件预览**
  - 双击或右键「播放 ▸ 原画」：音/视频交给 `mpv` 流式播放（携带签名直链所需请求头，无需完整下载）
  - 视频右键「播放 ▸」子菜单会列出云端可用清晰度（原画 / 1080P / 720P…），选择后由 `mpv` 播放；无转码流时仅原画
  - 音频只有原文件，直接播放（不解析清晰度）
  - 自动挂载同目录同集外挂字幕（`.ass/.srt/.vtt` 等），优先中文字幕
  - 其它文件下载到本地缓存后交给系统默认查看器（`xdg-open`），同一文件再次预览直接命中缓存
  - 未安装 `mpv` 时仅提示安装，不静默回退
- **本地上传**
  - 文件页工具栏「上传文件 / 上传文件夹」、空白右键菜单、或「传输任务 ▸ 上传」上传到当前目录
  - 先算 gcid 秒传哈希：服务端已有相同内容时直接完成（秒传），否则走阿里云 OSS 分片上传
  - 分片走 OSS HMAC-SHA1（不引入 AWS SDK），**并发上传**且网络中断后**续传已传分片**；进度 / 速率、取消、失败重试
  - 目录递归上传：按本地目录结构在云端建目录并逐个上传（跳过符号链接）
  - 传输任务页上传列表：显示目标网盘路径、速率 / 剩余时间、完成时间；状态筛选（全部/进行中/已完成/失败）、
    全选与批量取消 / 重试 / 移除、清除已完成；完成后可「在网盘中打开」
  - 上传历史持久化（`uploads.json`，最多 200 条，重启后恢复列表）
- **本地下载（传输任务 ▸ 下载）**
  - 文件页选中 / 右键、离线「已完成」任务均可一键下载到本地目录
  - 在「传输任务」页下载分栏展示任务列表（进度 / 速率 / 取消 / 打开目录），
    带状态筛选（全部 / 进行中 / 已完成 / 失败）与批量选择、批量重试 / 移除
  - `.part` 临时文件 + 完成后原子改名；中断后自动 `Range` 断点续传
  - 完成后按云端声明大小做完整性校验，不完整不会落盘为正式文件
  - 解析直链 / 传输的瞬时错误（断网、5xx、429 等）均退避重试（重试前自动刷新限时直链）
  - 同名文件自动加 ` (n)`，目标目录记忆、下载历史持久化（最多 200 条，重启后恢复）
- **离线下载（磁力 / HTTP）**
  - 提交 magnet / 直链转存到云端
  - 状态分桶展示：等待 / 下载中 / 已完成 / 失败（轮询自动刷新）
  - 重试失败任务、删除任务、整组清空、保存位置可选
- **分享**
  - 文件页工具栏 / 右键对选中项（含文件夹）创建分享链接，可选有效期（1/7/30 天或永久）与提取码
  - 「我的分享」页列出已创建的分享（公开 / 私密 / 已失效、文件数、有效期、浏览 / 转存次数、创建时间）
  - 复制分享链接 / 链接+提取码；单条取消或勾选批量取消（链接立即失效）；在浏览器打开；分页加载
  - 创建成功后弹窗展示链接与提取码，一键复制 / 在浏览器打开
  - 分享列表 SWR 缓存（进入页面命中新鲜期零请求）
  - 「转存分享」：在「我的分享」页点击「转存分享」按钮，输入他人分享链接（或 ID）与提取码，解析后选择文件转存到自己网盘
- **回收站**
  - 「我的文件」中删除的文件 / 文件夹在此列出（类型图标、大小、删除时间），分页加载、SWR 缓存
  - 单行或勾选批量「还原」到原目录，或「彻底删除」（二次确认，不可恢复）
  - 右上「清空回收站」：调用服务端接口一次清空全部内容
  - 操作成功后即时从列表移除，并刷新存储配额与目录缓存
- **体验**
  - 左侧边栏导航（我的文件 / 我的分享 / 回收站 / 离线下载 / 传输任务 / 设置）
  - 自动跟随系统 KDE 主题与强调色、记住账号
  - 可勾选「记住密码」：密码保存在系统密钥环（Secret Service），会话失效时自动重新登录
  - 侧边栏底部容量进度卡、任务角标、中文界面

## 界面截图占位

项目尚未内置截图；可自行 `cargo run -p kichi-gui` 后查看。

## 目录结构

```
crates/
├── kichi-core/         # API 客户端核心库（无界面依赖，可复用）
│   └── src/
│       ├── client.rs     # HTTP 层：鉴权、自动 refresh(code16)、自动 captcha 重试(code9)
│       ├── captcha.rs    # captcha_sign / device_sign 加签算法
│       ├── consts.rs     # client_id / host / 盐值表 / 常量
│       ├── download.rs   # 直链解析、.part 断点续传、完整性校验
│       ├── upload.rs     # gcid 秒传哈希、阿里云 OSS 分片签名与上传
│       ├── types.rs      # File / Quota / Task 等模型（防御式解析）
│       ├── error.rs      # 统一错误类型
│       └── session.rs    # 会话持久化
└── kichi-gui/          # eframe(egui) 桌面应用
    └── src/
        ├── main.rs       # 入口
        ├── app/          # UI 模块（按职责拆分）
        │   ├── mod.rs        # App 结构体、初始化、消息处理、业务逻辑
        │   ├── types.rs      # Page / ViewMode / TransferTab / DlJob 等内部类型
        │   ├── login.rs      # 登录页
        │   ├── sidebar.rs    # 侧边栏 + 导航 + 账户/配额卡片
        │   ├── files_page.rs # 文件浏览页 + 列表行/网格渲染
        │   ├── tasks_page.rs # 离线下载页
        │   ├── transfers_page.rs # 传输任务页（上传占位 / 下载）
        │   ├── settings_page.rs  # 设置页
        │   ├── shares_page.rs    # 我的分享页（列出 / 复制 / 取消）
        │   ├── trash_page.rs     # 回收站页（列出 / 还原 / 彻底删除）
        │   ├── dialogs.rs    # 新建文件夹 / 重命名 / 回收站 / 彻底删除 / 分享 / 退出确认弹窗
        │   └── helpers.rs    # 工具函数（字体、目录选择、mpv 播放、文本裁剪等）
        ├── icons.rs      # 矢量图标库（painter 绘制，不依赖字体字形）
        ├── theme.rs      # 配色 / 圆角 / 间距参数与全局样式
        ├── kde.rs        # 读取 KDE 系统配色（kdeglobals），非 KDE 返回 None
        ├── worker.rs     # 后台 tokio 线程 + channel 通信
        ├── credentials.rs # 系统密钥环读写账号密码（Secret Service）
        ├── msg.rs        # 前后台消息协议
        ├── settings.rs   # 设置与下载历史持久化
        └── format.rs     # 大小 / 时间 / 状态文案格式化
```

## 构建与运行

环境：Linux（已在 Fedora 44 / KDE Plasma / Wayland 验证）。

系统依赖（运行 / 链接时需要）：

```bash
# Fedora/RHEL 系
sudo dnf install gcc pkgconf openssl-devel libxkbcommon-devel wayland-devel \
     mesa-libGL mesa-libEGL fontconfig
```

还需要一款含中文字体的 TTF（程序会自动探测，常见路径见 `app/helpers.rs::install_fonts`）。

```bash
# 开发运行
cargo run -p kichi-gui

# 发布构建（产物 target/release/kichi-gui）
cargo build --release -p kichi-gui

# 运行测试
cargo test --workspace
```

### 桌面图标（KDE / Wayland）

Wayland 下窗口管理器不读取程序内设置的窗口图标，而是按窗口 `app_id` 匹配已安装的
`.desktop` 与图标主题。运行一次安装脚本即可：

```bash
./packaging/install-icon.sh   # 安装到 ~/.local/share 并刷新图标/菜单缓存
```

脚本会写入 `kichi.desktop` 与 hicolor 图标（SVG 源文件位于
`assets/kichi.svg`）。应用启动时已声明 `app_id = "kichi"`，
重启应用后任务栏/窗口即显示新图标。

## 配置与数据

| 文件 | 作用 |
| --- | --- |
| `~/.config/kichi/session.json` | access/refresh token、device id 等登录态 |
| `~/.config/kichi/settings.json` | 记住的账号、本地下载目录、是否记住密码 |
| `~/.config/kichi/downloads.json` | 本地下载历史（最多 200 条，启动时恢复为任务列表） |
| `~/.config/kichi/uploads.json` | 本地上传历史（最多 200 条，启动时恢复为任务列表） |
| `~/.cache/kichi/kichi.log` | 运行日志（同时输出到 stderr）；级别由 `KICHI_LOG` / `RUST_LOG` 控制 |
| `~/.cache/kichi/preview/` | 非流媒体文件的预览缓存（按文件 id 分目录） |

> 「记住密码」的密码保存在**系统密钥环**（KDE Wallet / GNOME Keyring，Secret Service），
> 不会明文写入上述配置文件；密钥环不可用时只记住账号，不影响正常登录。

## 已知限制

- Wayland 下不支持**拖拽上传**（winit 的 Wayland 后端未实现拖放），请用「上传文件 / 上传文件夹」。
- 图标（网格）视图使用文件类型图标，暂不加载真实缩略图。
- 本地下载整目录暂不支持（会跳过文件夹）。
- 离线任务列表字段无官方文档，当前按 `phase` 分桶请求以保证状态准确。
- 部分操作（如异常 IP 登录）会被 PikPak 要求网页端人机验证，此时会提示失败。
- 超过 100 条的任务/文件分批显示，仅支持「加载更多」。

## 免责声明

本项目与 PikPak / 迅雷及其关联公司无关。API 端点与签名算法来自社区逆向成果，参考
[Quan666/PikPakAPI](https://github.com/Quan666/PikPakAPI)、
[52funny/pikpakcli](https://github.com/52funny/pikpakcli)、
[52funny/pikpakhash](https://github.com/52funny/pikpakhash)、
[Bengerthelorf/pikpaktui](https://github.com/Bengerthelorf/pikpaktui)、
[rclone](https://github.com/rclone/rclone) 等，仅用于个人学习研究。

详细路线见 [PROJECT_PLAN.md](./PROJECT_PLAN.md)。
