# PikPak Linux 客户端

PikPak 云盘的 Linux 桌面客户端（非官方）。基于逆向的私有 API 实现，Rust 编写，使用 egui/eframe 构建界面。

> ⚠️ 非官方项目，接口可能随服务端变更而失效；仅供学习与个人使用。

## 界面

桌面 GUI 采用「左侧边栏 + 右侧内容区」布局：

- **侧边栏导航**：网盘文件 / 传输任务 / 本地下载 / 设置；底部常驻账户与存储空间配额卡片
- **文件页**：按类型矢量图标（文件夹 / 视频 / 图片 / 音频 / 文档 / 压缩包）、
  可点击排序表头（名称 / 大小 / 修改时间）、悬浮与选中高亮、搜索过滤
- **本地下载**：常驻侧边栏页（带进行中角标），进度卡片 + 取消 / 移除 / 打开目录
- 深浅两套主题、统一圆角与配色参数（`theme.rs`）、登录居中卡片

> 注：文件行操作以右键菜单为主（下载 / 打开预览 / 重命名 / 复制名称 / 移入回收站），
> 双击文件夹进入、双击文件预览、单击选中（Ctrl 单击多选 / 取消）。

## 功能

- **登录 / 账户**
  - 邮箱 / 手机号密码登录（含 PikPak 的 captcha 流程）
  - 登录态本地持久化、自动恢复与自动续期（access token 过期自动 refresh）
- **文件浏览 / 管理**
  - 目录浏览（面包屑导航、进入/返回）
  - 列表表头排序（名称 / 大小 / 修改时间，升/降序）、当前目录按名称过滤
  - 新建文件夹、重命名、多选 / 批量移入回收站（回收后本地即时移除，不依赖服务端同步）
  - 右键菜单、快捷键（F5 刷新 / Ctrl+A 全选 / Delete 删除）
  - 文件类型矢量图标、分页加载
- **文件预览**
  - 双击或右键「播放 ▸ 原画」：音/视频交给 `mpv` 流式播放（携带签名直链所需请求头，无需完整下载）
  - 右键「播放 ▸」子菜单会列出云端可用清晰度（原画 / 1080P / 720P…），选择后由 `mpv` 播放；无转码流时仅原画
  - 自动挂载同目录同集外挂字幕（`.ass/.srt/.vtt` 等），优先中文字幕
  - 其它文件下载到本地缓存后交给系统默认查看器（`xdg-open`），同一文件再次预览直接命中缓存
  - 未安装 `mpv` 时仅提示安装，不静默回退
- **本地下载**
  - 文件页选中 / 右键、离线「已完成」任务均可一键下载到本地目录
  - 以「本地下载」侧边栏页面展示任务列表（进度 / 速率 / 取消 / 打开目录），
    发起下载后自动跳转
  - `.part` 临时文件 + 完成后原子改名；中断后自动 `Range` 断点续传
  - 完成后按云端声明大小做完整性校验，不完整不会落盘为正式文件
  - 解析直链 / 传输的瞬时错误（断网、5xx、429 等）均退避重试（重试前自动刷新限时直链）
  - 同名文件自动加 ` (n)`，目标目录记忆
- **离线下载（磁力 / HTTP）**
  - 提交 magnet / 直链转存到云端
  - 状态分桶展示：等待 / 下载中 / 已完成 / 失败（轮询自动刷新）
  - 重试失败任务、删除任务、整组清空、保存位置可选
- **体验**
  - 左侧边栏导航（文件 / 任务 / 本地下载 / 设置），深浅主题切换、记住账号
  - 侧边栏底部容量进度卡、任务角标、中文界面

## 界面截图占位

项目尚未内置截图；可自行 `cargo run -p pikpak-gui` 后查看。

## 目录结构

```
crates/
├── pikpak-core/          # API 客户端核心库（无界面依赖，可复用）
│   └── src/
│       ├── client.rs     # HTTP 层：鉴权、自动 refresh(code16)、自动 captcha 重试(code9)
│       ├── captcha.rs    # captcha_sign / device_sign 加签算法
│       ├── consts.rs     # client_id / host / 盐值表 / 常量
│       ├── types.rs      # File / Quota / Task 等模型（防御式解析）
│       ├── error.rs      # 统一错误类型
│       └── session.rs    # 会话持久化
└── pikpak-gui/           # eframe(egui) 桌面应用
    └── src/
        ├── main.rs       # 入口
        ├── app/          # UI 模块（按职责拆分）
        │   ├── mod.rs        # App 结构体、初始化、消息处理、业务逻辑
        │   ├── types.rs      # Page / SortBy / DlJob / DlStatus 等内部类型
        │   ├── login.rs      # 登录页
        │   ├── sidebar.rs    # 侧边栏 + 导航 + 账户/配额卡片
        │   ├── files_page.rs # 文件浏览页 + 文件行渲染
        │   ├── tasks_page.rs # 离线任务页
        │   ├── downloads_page.rs # 本地下载页
        │   ├── settings_page.rs  # 设置页
        │   ├── dialogs.rs    # 新建文件夹 / 重命名 / 回收站 / 退出确认弹窗
        │   └── helpers.rs    # 工具函数（字体加载、目录选择、文本裁剪等）
        ├── icons.rs      # 矢量图标库（painter 绘制，不依赖字体字形）
        ├── theme.rs      # 明暗主题配色 / 圆角 / 间距参数
        ├── worker.rs     # 后台 tokio 线程 + channel 通信
        ├── msg.rs        # 前后台消息协议
        ├── settings.rs   # 主题 / 账号设置持久化
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
cargo run -p pikpak-gui

# 发布构建（产物 target/release/pikpak-gui）
cargo build --release -p pikpak-gui

# 运行测试
cargo test --workspace
```

### 桌面图标（KDE / Wayland）

Wayland 下窗口管理器不读取程序内设置的窗口图标，而是按窗口 `app_id` 匹配已安装的
`.desktop` 与图标主题。运行一次安装脚本即可：

```bash
./packaging/install-icon.sh   # 安装到 ~/.local/share 并刷新图标/菜单缓存
```

脚本会写入 `pikpak-linux.desktop` 与 hicolor 图标（SVG 源文件位于
`assets/pikpak-linux.svg`）。应用启动时已声明 `app_id = "pikpak-linux"`，
重启应用后任务栏/窗口即显示新图标。

## 配置与数据

| 文件 | 作用 |
| --- | --- |
| `~/.config/pikpak-linux/session.json` | access/refresh token、device id 等登录态 |
| `~/.config/pikpak-linux/settings.json` | 主题偏好、记住的账号 |
| `~/.cache/pikpak-linux/preview/` | 非流媒体文件的预览缓存（按文件 id 分目录） |

## 已知限制

- 未实现：本地上传、分享链接转存。
- 本地下载整目录暂不支持（会跳过文件夹）。
- 离线任务列表字段无官方文档，当前按 `phase` 分桶请求以保证状态准确。
- 部分操作（如异常 IP 登录）会被 PikPak 要求网页端人机验证，此时会提示失败。
- 超过 100 条的任务/文件分批显示，仅支持「加载更多」。

## 免责声明

本项目与 PikPak / 迅雷及其关联公司无关。API 端点和签名算法来自社区逆向成果（参考
[Quan666/PikPakAPI](https://github.com/Quan666/PikPakAPI) 等），仅用于个人学习研究。

详细路线见 [PROJECT_PLAN.md](./PROJECT_PLAN.md)。
