# 第三方代码与运行时组件声明

kichi 自身以 `GPL-3.0-or-later` 发布，完整文本见 `LICENSE`。本文件记录 kichi
直接借用或随分发包提供的第三方内容；这些内容仍归各自版权持有人所有。

## 直接借用的代码与数据

| 来源 | 许可证 | 使用范围 |
| --- | --- | --- |
| [rclone](https://github.com/rclone/rclone) 的 PikPak 后端 | MIT | `kichi-core` 的端点常量、设备 ID / captcha 签名算法及部分请求形状；相关文件已标注来源 |
| pikpaktui 0.0.58 | Apache-2.0 | 分享、任务和选链等请求形状的行为参考；未复制的部分不产生额外义务 |
| pikpak-kotlin | MIT | 部分接口字段与请求形状的行为参考 |
| [Tabler Icons](https://github.com/tabler/tabler-icons) | MIT | `kichi/qml/NavIcon.qml` 中使用的路径数据 |
| [wayland-protocols](https://gitlab.freedesktop.org/wayland/wayland-protocols) | MIT | `kichi/cpp/protocol/` 下由 `wayland-scanner` 生成的 linux-dmabuf / viewporter / color-management 客户端协议代码；版权归 Collabora、Intel 等各协议作者 |

内嵌播放器的「libmpv + Wayland 代理截帧」思路参考了 Mutsumi（MIT）；未复制其代码，
因此不随本项目分发其源码或二进制。

上述 MIT 内容使用以下许可文本：

```text
MIT License

Copyright (c) Nick Craig-Wood and rclone contributors
Copyright (c) Paweł Kuna and Tabler Icons contributors
Copyright © 2012-2023 Collabora, Intel Corporation and wayland-protocols contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

`pikpaktui` 的 Apache-2.0 文本见
<https://www.apache.org/licenses/LICENSE-2.0>；若后续直接复制其代码，发布包中
还必须保留其版权声明和 NOTICE（如有）。

## Rust 依赖

构建使用的 crate 及其许可证以 `Cargo.lock` 和各 crate 的元数据为准。发布
AppImage 或二进制包时，使用 `cargo-about` 或等价的许可证扫描工具生成完整的
第三方许可证清单，并放入 `usr/share/licenses/kichi/`。

内嵌播放直接依赖的主要 crate：`libmpv2`（LGPL-2.1，libmpv 绑定）与
`wl-proxy`（MIT OR Apache-2.0，进程内 Wayland 代理）。

## 内嵌播放的运行时组件

| 组件 | 许可证 | 使用范围 |
| --- | --- | --- |
| libmpv（Fedora `mpv-libs`） | GPL-2.0-or-later（另含 LGPL-2.1-or-later、BSD-2-Clause、BSD-3-Clause、ISC、MIT 部分） | 进程内解码与渲染；AppImage 必须打包 |
| FFmpeg、libass、libplacebo 等 libmpv 依赖 | 由 linuxdeploy 随 libmpv 递归打包，按各自项目许可 | 解码、字幕、渲染 |
| libEGL / Mesa 用户态驱动 | 宿主提供，按发行版与驱动许可 | dmabuf → GL 纹理导入 |

kichi 的 `GPL-3.0-or-later` 与 libmpv 的 `GPL-2.0-or-later` 兼容。发布 AppImage
时必须随包收集 libmpv 及其依赖的许可证文本与对应源码获取方式。

## AppImage 中的 Qt / KDE 组件

AppImage 会动态打包 Qt 6 和 KDE 运行时组件。按当前 Fedora 构建环境，主要包括：

| 组件 | 许可证 |
| --- | --- |
| Qt 6 Core / Gui / Qml / Quick / Wayland 等 | `LGPL-3.0-only OR GPL-3.0-only WITH Qt-GPL-exception-1.0` |
| KDE Kirigami | BSD-3-Clause、CC0-1.0、FSFAP、GPL-2.0-or-later、LGPL-2.x、LGPL-3.0-only、MIT 等（按文件） |
| `kf6-qqc2-desktop-style` | CC0-1.0、GPL-2.0-only、GPL-2.0-or-later、GPL-3.0-only、LGPL-2.x、LGPL-3.0-only 及 KFQF GPL 文本（按文件） |
| Breeze 图标主题 | LGPL-2.1-or-later、LGPL-3.0-or-later、CC-BY-SA-4.0（按文件） |

这些组件的完整许可证文本由发行版包提供。发布 AppImage 时必须随包收集对应文本，
并保证接收者能够取得 AppImage 所对应源码；不能只依赖系统里已有的许可证文件。

## 系统服务与外部程序

KWallet / Secret Service、XDG Desktop Portal、系统通知、系统托盘、外部 mpv / VLC
（仅在用户选择对应预设时）和图形驱动不由本项目打包，仍由用户系统提供，并受其
各自许可证约束。默认的内嵌播放使用随包分发的 libmpv，不需要宿主安装 mpv。
