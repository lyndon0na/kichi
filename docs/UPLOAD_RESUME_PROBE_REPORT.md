# 上传跨重启续传 —— 探针方案 · 实机结论 · 参考项目

> 日期：2026-09-29（2026-10-03 更新） ｜ 起草：AI 协作者（结论全部来自维护者在实机上运行探针的输出）
> 状态：**已采信；探针保留为手动回归工具**。实现已落地：`8637660`（core：票据暴露
> `file_id` / `expiration`、占位条目清理支持、探针入库）+ `dadd907`（GUI：落盘凭证、
> 卡片恢复自动续传）；文档同步（AGENTS / README / NOTES / TODO / CHANGELOG /
> PROJECT_PLAN）随本次 docs 提交完成，本文件归位 `docs/`。
>
> **一句话结论：旧结论「跨重启续传不可行」已被推翻。** 持久化 STS 凭证后，在 12 小时
> 凭证有效期内**不重新取票**即可续传：跨进程 4 次全过，跨**重启** 1 次过，回下载 gcid
> 与本地一致；不可见的 `PENDING` 占位条目亦可列出并删除（2026-10-03 `orphans` 实测，
> 见 §3.5）。这是 `docs/UPLOAD_RESUME_NOTES.md` §6「Option A」的实测兑现。

---

## 0. 结论速览

| # | 探针要回答的问题 | 结论 | 证据等级 |
|:--|:--|:--|:--|
| 1 | `upload_create` 响应暴露凭证有效期吗？多久？ | 暴露 `expiration`，= **取票时刻 + 12 小时**（3 个样本秒级吻合） | 实测 |
| 2 | 落盘凭证 + `upload_id` + ETag，**换进程**不重取票能否续传？ | 能，`oss_complete` 成功，条目转 `PHASE_TYPE_COMPLETE` | 实测 ×4 |
| 3 | 换进程不算数，**跨重启**呢？ | 同样能（stage1 20:40:54 → 重启 20:41:43 → stage2 PASS） | 实测 ×1（单样本） |
| 4 | 续传拼出来的文件是好的吗？ | 回下载 12,582,912 字节，gcid 与本地逐字节一致 | 实测 ×3 |
| 5 | 续传完成的条目要不要「等一会儿」才让下载？ | 不需要；曾经看到的卡顿是**偶发慢**（同 12 MiB：128.55s / 14.12s / 9.23s） | 实测（排除） |
| 6 | 每次取票留下的不可见 `PENDING` 占位条目能清吗？ | 能：`phase.eq` 过滤可列出、`batch_trash` + `batch_delete` 可删（`phase.in` 数组形式被拒）—— 2026-10-03 `orphans` 实测 | 实测 |
| 7 | 大文件（>100 分片）/ 传输 >12h / 弱网断连 下是否同结论？ | 未验证，只有 12 MiB·3 分片的样本 | 未验 |

---

## 1. 背景与旧结论

`docs/TODO.md` 的 **P2-1**：被中断（尤其**关掉软件**）的大文件上传，希望下次启动接着传。

`docs/UPLOAD_RESUME_NOTES.md` 记录过一次失败尝试，结论是「不可行」，理由是：

- 走的是「重启 → **重新 `upload_create` 取新票** → 拿新凭证去接旧 `upload_id`」；
- 服务端 STS 凭证按**对象 key** 做 policy 授权，而 key 形如
  `upload_tmp/<GCID>_<19 位纳秒>`，每次取票都换 → 访问旧 key 得到
  `403 AccessDenied`（NOTES §4(c)）。

同一份 NOTES 的 §6 给出了唯一可能走通的路径（Option A）：**别重新取票**，改为
**持久化 STS 凭证本身**，在有效期内直接对旧 key / 旧 `upload_id` / 旧凭证续传；
并列出 4 个待验前提（有效期字段是否暴露、窗口多长、凭证落盘的安全权衡、完成落库是
否产生重复条目）。**本报告就是对这 4 条前提的实测回应。**

一句话区分两条路：

| 路线 | 行为 | 结果 |
|:--|:--|:--|
| 旧（已证伪） | 重启后**重新取票**，拿新凭证接旧 `upload_id` | 403 AccessDenied |
| 新（本次实测） | 重启后**用落盘的旧凭证**，不重新取票 | 成功续传并完成 |

---

## 2. 探针方案

### 2.1 代码落点：「零侵入」挂载

- 探针代码：`crates/kichi-core/src/upload_resume_probe.rs`（**已随仓库保留为手动回归工具**：
  `#[ignore]` + `KICHI_UPLOAD_PROBE=1` 双开关，`#[cfg(test)]` 引入、不进发布产物）。
- 挂载方式：在 `client.rs` 里以**子模块**引入 ——

  ```rust
  /// 上传跨重启续传探针(一次性实验, 见文件头注释; 验证完可整体删除)。
  #[cfg(test)]
  #[path = "upload_resume_probe.rs"]
  mod upload_resume_probe;
  ```

  子模块身份让它能调用 `KichiClient` 的**私有方法**（`file_list_filtered`、`post` 等），
  又不进入发布产物（`#[cfg(test)]`）。
- 唯一的**生产代码改动**：把 `client.rs::upload_create` 拆成
  `upload_create` + `upload_create_raw`（前者行为不变；后者返回原始响应体，探针要从
  `resumable.params` 里读 `expiration`）。**行为等价，已过 clippy / 全量单测。**

### 2.2 各阶段（跨进程 / 跨重启）

| 阶段 | 做什么 |
|:--|:--|
| `stage1` | 生成 12 MiB **随机**内容 → 取票 → `oss_initiate` → 只传第 1 片（5 MiB）→ 把**凭证 + `upload_id` + ETag + gcid + 种子**写状态文件 → 退出 |
| （重启机器） | —— |
| `stage2` | 读状态文件 → **不重新取票** → 只传剩余 2 片 → `oss_complete` → 回下载比对 gcid → 删状态文件 |
| `verify` | 只做回下载校验（不重传）；状态文件不在时退回「按云端条目 + 本地文件」校验 |
| `orphans` / `cleanup` | 列出 / 清理 `stage1` 攒下的不可见 `PENDING` 占位条目；`cleanup` 只删状态文件里记下的那个 id |

细节：12 MiB 按 5 MiB 切成 3 片（`upload_chunk_size` 的既有算法）；`stage2` 打印
「距 stage1 已过 N 分钟」以便判断窗口；状态文件路径
`~/.cache/kichi/probe/upload_resume_session.json`。

### 2.3 安全边界（探针里的硬约定）

- **只用** `~/.config/kichi/session.json` 里已有的登录会话，**不接受明文密码**；
  并装上与 GUI 一致的 `TokenSaver`，保证 refresh 后的会话能正常落盘。
- 终端输出**不含任何凭证**：`access_key_id` / `access_key_secret` / `security_token`
  只打印长度；直链只打印 `scheme://host`，不打印带签名的完整 URL。
- 状态文件（含短期 OSS 凭证）权限 **0600**、目录 0700；`stage2` 成功后自动删除，
  失败保留以便重试（**该文件不要提交 / 分享 / 贴进任何对话**）。
- 每轮内容**随机**（种子来自纳秒时间、打印并落盘），否则会命中服务端秒传。

### 2.4 开关与运行方式

环境变量 `KICHI_UPLOAD_PROBE=1` **且** `--ignored` 才跑（双保险）；命令见 §6。

---

## 3. 实机结论

### 3.1 票据与凭证结构（实测）

`POST /drive/v1/files` 响应顶层键：`["file", "resumable", "task", "upload_type"]`；
取票时刻 `file.phase = PHASE_TYPE_PENDING`。

`resumable.params` 共 **8** 个键：

```json
{
  "access_key_id": "…(长度 24 左右)", "access_key_secret": "<脱敏>",
  "security_token": "<脱敏, 很长>",
  "bucket": "vip-lixian-07",
  "endpoint": "upload-a10b.mypikpak.com",
  "key": "upload_tmp/<GCID 大写>_<19 位纳秒时间戳>",
  "cname": "…", "expiration": "2026-09-30T08:40:55+08:00"
}
```

要点：

- **没有** `upload_id` / `upload_url` —— 客户端必须自己 `oss_initiate`（与现有代码一致）。
- `expiration` **是暴露的**（NOTES §6.2 的那条前提成立），且已解析成 RFC3339（带 `+08:00`）。
- 服务端下发的 key 里嵌着**大写 gcid**；`upload_id` 为 32 位。

### 3.2 有效期：取票时刻 + 12 小时（实测）

三组样本（同一天晚间，秒级吻合）：

| 取票时刻 | `expiration` | 差值 |
|:--|:--|:--|
| 20:29:57 | 2026-09-30T08:29:58+08:00 | +12:00:01 |
| 20:33:51 | 2026-09-30T08:33:52+08:00 | +12:00:01 |
| 20:40:54 | 2026-09-30T08:40:55+08:00 | +12:00:01 |

**这推翻了 NOTES §2 里「通常约 1 小时」的猜测，也修正 §6.4 的「隔夜无效」**：
窗口是 12 小时，「晚上关机、次日上午重开」仍在有效期内。

### 3.3 跨进程 / 跨重启续传（实测）

- **跨进程 ×4 全过**：每个 `stage2` 都是全新进程，复用落盘凭证 + `upload_id` + 已传 ETag，
  只传剩余分片，`oss_complete` 成功。
- **跨重启 ×1 过**：`stage1` 20:40:54 → **重启机器**（`uptime -s` = 20:41:43，即 49 秒后；
  `/proc/uptime` 亦确认）→ `stage2` 于 ~20:43 PASS。
- 完成后云端条目转为 `phase=PHASE_TYPE_COMPLETE`，大小 12,582,912 字节，与本地一致；
  默认列表过滤下**未见重复条目**。

> NOTES §6.5 担心的「旧票 / 新票各留一个条目、可能出重复文件」在 Option A 下**不成立** ——
> 因为这条路**根本不重新取票**，自始至终只有一张票、一个 key、一个条目。实测亦无重复。

### 3.4 数据一致性（实测）

`stage2` / `verify` 会把云端文件回下载到本地再算 gcid，与本地源文件比对。三次样本均一致
（gcid 前缀：`cb77fb80…`、`b805e4c2…`、`a033a02e…`；内容为每轮随机数据，故 hash 各异）。

### 3.5 反例与边界观察

- **秒传反例**：探针最早用固定内容，第二次运行时响应里**没有 `resumable` 键**、
  `file.phase=PHASE_TYPE_COMPLETE` —— 服务端按 gcid 全局去重。启示：实现续传时记录的
  **身份必须是 `(gcid, size)`**，且拿到秒传响应要直接当完成处理。
- **偶发慢，不是卡死**：同 12 MiB 回下载耗时 128.55s（≈98 KB/s） / 14.12s / 9.23s。
  最初那次「stage2 卡住 >60s」是慢，不是服务端拒绝。→ **不需要**「续传完成后等待再下载」
  的逻辑；保留既有退避重试即可。
- **客户端无 timeout**：`KichiClient` 的 reqwest 客户端没配 `.timeout()`，连接被挂起就会
  一直等（应用既有属性，GUI 同理）。探针自己给每步套了硬超时。此问题未随续传实现一并
  改动，另立 TODO **P1-5（HTTP 请求超时）** 跟踪。**2026-10-03 已修复（`0e68f83`）**：
  连接阶段（含 TLS 握手）10s、控制类小请求（API JSON / captcha / 续期 / 直链探测）30s
  总超时；传输体（下载 / OSS 分片 / 缩略图）仍不设总时长 —— 避免误杀本文 §3.5 记录的
  慢速传输（12 MiB 下 128.55s）。手动复核：
  `cargo test -p kichi-core blackhole -- --ignored --nocapture`（本机 `10.255.255.1` 经
  透明代理被立刻接成静默连接，由请求总超时兜底；无代理环境下的 SYN 黑洞由连接超时兜底）。
- **PENDING 占位条目（2026-10-03 已确证）**：每次 `upload_create` 都会在网盘留一个
  `phase=PENDING` 的条目，默认过滤 `{"trashed":{"eq":false},"phase":{"eq":"PHASE_TYPE_COMPLETE"}}`
  看不到它；`stage1` 跑过多次就攒了多个。`orphans` 探针实测：
  - `{"trashed":{"eq":false},"phase":{"eq":"PHASE_TYPE_PENDING"}}` —— **可列出**（采用）；
  - `{"trashed":{"eq":false},"phase":{"in":["PHASE_TYPE_PENDING","PHASE_TYPE_RUNNING","PHASE_TYPE_COMPLETE"]}}`
    —— **被服务端拒绝**（`error_code=3`，数组形式不合法）。

  列出后 `batch_trash` + `batch_delete` **可以删除**。实现按此清理
  （`worker/upload.rs::cleanup_placeholder`；删除前必须确认条目仍为 PENDING，防误删
  已完成文件）。

### 3.6 未验证（不要当成已成立）

- **跨重启只有 1 次样本**，建议再补 1–2 次（含「重启后隔一段时间再续」）。
- 只有 **12 MiB / 3 分片** 的样本；大文件（几十~上百 GB，>10000 分片）未验。
- **>12 小时的长传**（不重启也可能中途凭证过期）未验 —— 推演：过期后分片 PUT 会 403，
  实现上必须能「续不上就安全降级为全量重传」。
- 弱网 / 传输中拔网 / 服务端换 bucket 等未验。
- 目录递归上传、并发多文件上传的续传语义未验（与单文件同构，但未实测）。

---

## 4. 实现方案建议（**已按此落地**：`8637660` core + `dadd907` GUI）

### 4.1 最小可用路径

1. **core**：`UploadTicket` 增加 `expiration` 与凭证直通（或新增 `UploadTicketRaw`；
   探针里用的 `upload_create_raw` 就是这个思路）。`expiration` 需要一个时间解析
   （现依赖里没有 `chrono`/`time`，要么加依赖，要么按 `%Y-%m-%dT%H:%M:%S+08:00` 手解；
   **解析失败就当作「有效期未知」并按保守策略处理**）。
2. **记录**：恢复并扩展 `settings.rs` 的 `UploadResumeRecord` —— 存
   `gcid / size / upload_id / oss(凭证) / etags / expiration_unix / 展示元数据`，
   写 `~/.config/kichi/upload_resume.json`，**权限 0600**，worker 单写。
3. **判定（抽成纯函数 + 单测）**：`should_resume(record, now, 本地文件) -> 续传 | 重传`：
   `gcid + size` 匹配 **且** `now < expiration - 安全余量`（建议 5–10 分钟）→ 续传；
   否则**丢弃记录、全量重传**（安全降级，永不传坏）。
4. **worker**：拿到 `upload_id` 即 upsert 记录（含凭证与过期时间）；成功 / 取消 / 秒传删记录；
   失败保留。启动 / 登录后识别记录 → 续传或降级。
5. **过期后**：丢弃已传分片、从头重传，日志 + 卡片给一句「上传凭证已过期，已重新开始」。

### 4.2 需要拍板的取舍

| 项 | 选项 | 备注 |
|:--|:--|:--|
| 凭证怎么存 | (a) 明文 JSON 0600（旧实现 / piko 路线）<br>(b) 系统密钥环（`credentials.rs` 已有 Secret Service 封装） | 凭证短时、限单一对象、12h 即失效；但仍是磁盘凭据。pikpak-kotlin 的做法是「与会话同等看待、别进日志」 |
| 重启后的交互 | (a) 自动续传（旧实现做法）<br>(b) 还原为「已暂停」，用户点继续 | 自动更省事，但要考虑用户已不想要这个任务的情况 |
| 对账要不要 ListParts | (a) 只信记录的 ETag<br>(b) 用 `oss_list_parts` 对账 | 旧实现有 (b)，但**先要修签名**：无体 GET 必须按实际 Content-Type（空串）签名，见 NOTES §5 |

**最终取舍（2026-10-03，已实现）**：凭证走 (a) 明文 JSON `0600`（原子写、不落日志）；
重启后 (a) 自动续传（卡片先还原为「排队中」，登录成功后自动分发）；对账走 (a) 只信
落盘 ETag —— 不做 ListParts，即绕开上表提到的无体 GET 签名坑。

### 4.3 已知的坑（避免重踩）

- **不要回到「重取票续旧 `upload_id`」** —— 那条路被 policy 挡死（NOTES §4）。
- 若实现 ListParts 对账，先读 NOTES §5 的签名注意点（否则 `SignatureDoesNotMatch`）。
- 12h 是**从取票时刻**算的，不是「最后活动 + 12h」：超长上传即使不重启也会中途过期。
- 每次 `upload_create` 留一个不可见 `PENDING` 占位条目 —— 取消 / 失败路径要考虑清理。
- `stage1` 的探针文件**别反复跑**（每次都新增一个占位条目）。

---

## 5. 参考项目

本轮调查（2026-09-29）实际查看过、并与实测交叉印证的项目：

| 项目 | 语言 | 我们参考了什么 |
|:--|:--|:--|
| [NihilDigit/pikpak-kotlin](https://github.com/NihilDigit/pikpak-kotlin) | Kotlin | **最接近的先例**。`src/commonMain/kotlin/io/github/nihildigit/pikpak/UploadEndpoint.kt` 把上传拆成 `startUpload` / `continueUpload` / `cancelUpload` 三个调用（源文件注释：`This is [startUpload], [continueUpload] and, on any failure or cancellation, [cancelUpload].`），并支持把会话存盘再在**另一个进程**继续（`src/jvmTest/kotlin/io/github/nihildigit/pikpak/UploadResumeProbeTest.kt`：两次 Gradle 运行、中间 `./gradlew --stop`，`PIKPAK_PROBE=1 PIKPAK_RESUME_STAGE=1\|2`）。其 wiki「Uploads and Offline Tasks」原话：`continueUpload asks OSS which parts arrived and sends the rest.`、`The session's OSS credentials expire 12 hours after the start.`、`The session holds live OSS credentials: keep it where the account's session is kept, not in logs.`、`Past that the upload can only be cancelled; OSS refuses with 403, which reaches the caller as UrlExpiredException.`、以及 `A session saved after one part was finished by another JVM, and the file read back byte for byte.`。**注意：其 wiki 未提「重启操作系统」，跨重启是本项目独立实测的补充。** |
| [NihilDigit/piko](https://github.com/NihilDigit/piko) | Kotlin | 同作者的成品客户端，把该设计落到了产品里。`shared/src/commonMain/kotlin/dev/piko/shared/upload/PikoUploadCoordinator.kt` 文件头原话：`真传的 OSS 会话随任务一起存盘，暂停、失败或进程被杀后都从 OSS 已收下的分片之后接着传。会话凭据 12 小时后失效，那时只能放弃已传的分片，从头再开一个会话。`；`shared/src/commonMain/kotlin/dev/piko/data/auth/PikoUserPreferences.kt`：`上传任务表的 JSON，见 PikoUploadCoordinator。含 12 小时有效的 OSS 凭据，与会话同等看待。` —— 与我们的实测（12h、过期即重传）完全一致，也给出了 §4.2「凭证怎么存」的社区取值。 |
| [AlistGo/alist](https://github.com/AlistGo/alist) | Go | 对照样本：`drivers/pikpak/driver.go::Put` → `drivers/pikpak/util.go::UploadByOSS` 用 OSS SDK **一次性**直传，未见跨进程续传的状态持久化（本次仅表层查看）。 |
| [rclone/rclone](https://github.com/rclone/rclone) | Go | `backend/pikpak/multipart.go`（并行分片上传；文档注明分片「stored in memory」）、`backend/pikpak/api/types.go` 的 `UploadTypeResumable = "UPLOAD_TYPE_RESUMABLE"`、`--pikpak-chunk-size` 默认 5Mi。另有一条有用的旁证：`fstest/test_all/config.yaml` 注释 —— `PikPak declares MD5 but never returns it for uploaded files, so corruption can't be detected by comparing hashes.` 这支持我们把**回下载 + gcid 比对**当作完整性判据。 |
| [52funny/pikpakcli](https://github.com/52funny/pikpakcli) · [52funny/pikpakhash](https://github.com/52funny/pikpakhash) · [Bengerthelorf/pikpaktui](https://github.com/Bengerthelorf/pikpaktui) · [Quan666/PikPakAPI](https://github.com/Quan666/PikPakAPI) | Go / Rust / Python | 本仓库既有逆向参考（见 `README.md` 结尾致谢、`docs/PROJECT_PLAN.md` 的「参考实现」表），现有上传链路即出自这些项目；本轮未新增引用。pikpak-kotlin 的 README 也致谢了 pikpakcli 与 pikpakhash（端点 / 加签 / hash 块大小表）。 |

> 上述项目**仅作行为与协议参照**，本仓库没有照抄其代码；许可请以各项目自身 LICENSE 为准。

---

## 6. 复现命令与残留物

```bash
# 1) 第一阶段: 取票 + 只传第 1 片, 把凭证 / upload_id / ETag 落盘
KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::stage1 -- --ignored --nocapture

# 2) 完全退出(建议顺手重启机器, 才叫「跨重启」)

# 3) 第二阶段: 不重新取票, 续传剩余分片 + complete + 回下载校验
KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::stage2 -- --ignored --nocapture

# 4) (可选) 只做回下载校验, 不重传; 状态文件不在时按云端条目 + 本地文件校验
KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::verify -- --ignored --nocapture

# 5) 清理: 先看有哪些不可见占位条目, 再清记录里的那个, 最后删本地探针文件
KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::orphans -- --ignored --nocapture
KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::cleanup -- --ignored --nocapture
KICHI_UPLOAD_PROBE=1 cargo test -p kichi-core --lib upload_resume_probe::remove_local_files -- --ignored --nocapture
```

残留物与注意：

- 状态文件（**含短期凭证，权限 0600**）：`~/.cache/kichi/probe/upload_resume_session.json`
  —— **不要提交 / 分享 / 贴进任何对话**；`stage2` 成功后自动删；
- 本地探针文件：`~/.cache/kichi/probe/upload_resume_local.bin`（源）、
  `upload_resume_back.bin`（回下载产物）；
- 云端：`kichi-resume-probe/` 目录 + 若干可见的 `kichi-resume-probe*.bin`（可手动删）
  + 每个 `stage1` 留下的**不可见** `PENDING` 占位条目（`orphans` 探针可列出并删除，
  见 §3.5）；
- **顺序别搞错**：`stage1 → 重启 → stage2 → (orphans) → cleanup`。
  `cleanup` 会删掉 `stage2` 需要的状态文件 —— 提前跑就得整轮重来（本轮真踩过一次）。

---

## 7. 遗留与下一步

**已落地（2026-10-03）：**

1. §4.2 的三个实现取舍均已拍板并实现 —— 凭证明文 JSON `0600`（不落日志）/
   重启后自动续传（卡片还原为排队、登录后自动分发）/ 不做 ListParts 对账；
2. 实现提交：`8637660`（core）+ `dadd907`（GUI）；文档同步（`AGENTS.md` / `README.md` /
   `docs/UPLOAD_RESUME_NOTES.md` / `docs/TODO.md` / `CHANGELOG.md` / `docs/PROJECT_PLAN.md`）
   随本次 docs 提交完成；本文件已归位 `docs/`。

**待跑 / 待补（仍未验证，保持警惕）：**

- 跨重启样本补到 2–3 次（建议含「重启后隔一段时间再续」—— 目前只有 1 次、49 秒后即续）；
- 大文件（>100 分片）与 **>12 小时**长传（不重启也可能中途凭证过期）未验；实现已做
  「过期即全量重传」的安全降级，但降级路径本身未实测；
- 弱网 / 传输中拔网、目录递归与并发多文件的续传语义未验。

**回归工具：** 探针已随仓库保留（`crates/kichi-core/src/upload_resume_probe.rs`，双开关
`#[ignore]` + `KICHI_UPLOAD_PROBE=1`）—— 怀疑服务端凭证 / 协议行为变化时，按 §6 的命令
重新跑 `stage1 → 重启 → stage2` 复核。