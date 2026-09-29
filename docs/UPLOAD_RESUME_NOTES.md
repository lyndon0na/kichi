# 上传跨重启续传 —— 可行性调查与结论

> 结论先说：**在当前 PikPak 上传协议下，跨进程重启的分片续传不可行**。
> 曾在提交 `da709f9` 里实现过一套（并捆绑修复了速率显示 Bug），经实机验证后
> 续传部分已整体回退，**仅保留速率修复**。本文档记录调查过程、证据与未来若要
> 实现需要满足的前提，供后续参考。

## 1. 背景

TODO 里的 **P2-1 上传跨重启续传**：希望一个被中断（尤其是**关掉软件**）的大文件
上传，能在下次启动后接着传，而不是从头再来。

## 2. PikPak 上传协议回顾

单文件上传链路（`kichi-core`）：

1. 本地算 `gcid`（分块 SHA1 再 SHA1）。
2. `POST /drive/v1/files`（`upload_create`），body 带 `name / size / hash(gcid) /
   upload_type=UPLOAD_TYPE_RESUMABLE / parent_id / folder_type=NORMAL`。响应里：
   - `file.phase == PHASE_TYPE_COMPLETE` → 秒传，直接完成；
   - 否则返回 `resumable.params`：一套**阿里云 OSS 临时凭证**
     （`endpoint / bucket / key / access_key_id / access_key_secret / security_token`）。
3. `oss_initiate`（`POST /{key}?uploads`）→ 拿到 `upload_id`。
4. `oss_upload_part`（`PUT /{key}?partNumber=N&uploadId=...`）逐个分片上传，收集 ETag。
5. `oss_complete`（`POST /{key}?uploadId=...` + ETag XML）合并。

签名用的是 OSS V1（HMAC-SHA1，`oss_authorization`）。STS 临时凭证**会过期**
（通常约 1 小时），所以跨重启必然要**重新 `upload_create` 取一张新票拿新凭证**——
这正是问题的根子（见 §4）。

## 3. 当初实现的方案（已回退）

- `settings.rs`：`UploadResumeRecord` 持久化到 `~/.config/kichi/upload_resume.json`
  （worker 单写，避免和 GUI 线程抢 `settings.json`）。存 `upload_id`、
  `endpoint/bucket/key`、已传分片 `ETags`、`gcid`、展示元数据（name/parent/dest_stack/total）。
- `kichi-core`：`oss_list_parts`（自动翻页，以 OSS 为**权威**对账已传分片），
  `upload_oss` 增加每分片回调以驱动快照落盘。
- `worker.rs`：`upload_local_file` 拿到 `upload_id` 即 upsert 记录；成功/取消/秒传删除，
  失败保留。重启走 `Cmd::ResumeUpload`：重刷凭证 → 比对位置 → `oss_list_parts` 对账
  → 灌回 ETag 跳过已传分片；不符/失效则安全退回全新上传。
- GUI：启动还原排队卡片、登录成功后 `dispatch_pending_uploads` 自动续传；
  卡片显示「续传中」+ 跳过比例；日志 `[上传续传]` 标注命中/退回。

设计上无论成不成都会**安全降级**为全量重传，不会传坏文件。

## 4. 实机验证与证据

用同一个大文件反复「上传到一半 → 强杀进程 → 重启登录」，抓 `[上传续传]` 日志：

**(a) key 其实是半稳定的。** 记录 key 与新票 key 都是：

```
upload_tmp/7E7E10CC6FBE5610B33872AC8C894C70808DE7AB_1789738366061887399   (旧)
upload_tmp/7E7E10CC6FBE5610B33872AC8C894C70808DE7AB_1789738402399216625   (新)
```

前缀 `7E7E...DE7AB` 就是文件的 **GCID**（大写十六进制），稳定；`endpoint`、`bucket`
也恒定。只有末尾的**纳秒时间戳**每次 `upload_create` 都变。所以早期
「服务端随机发新 key」的判断是错的——真正的问题在下文授权层面。

**(b) 一开始报的是 `SignatureDoesNotMatch`，是客户端签名 Bug，不是授权结论。**
`oss_authorization` 把 `string_to_sign` 的 Content-Type 行**写死**成
`application/octet-stream`，对所有方法一致。有体请求（PUT 分片 / POST 初始化与完成）
确实发这个头，能过；而 `oss_list_parts` 是**无体 GET**，服务端按空 Content-Type 计算
签名 → 两边不符 → `SignatureDoesNotMatch`。
修正：Content-Type 参数化，GET 签空串；并按 OSS V1 规则签名只纳入子资源
`uploadId`（`max-parts` / `part-number-marker` 属列举参数，不进 CanonicalizedResource）。

**(c) 签名修对后，得到真实裁决：`403 AccessDenied`。**

```
<Code>AccessDenied</Code>
<Message>Access denied by authorizer's policy.</Message>
```

即：新票下发的 STS 凭证是**按对象（key）做 policy 授权**的，只认本次的**新 key**，
拿它去访问**旧 key** 的 `upload_id` 直接被拒。ListParts 被拒，意味着对旧 key 的
`PutObject`（补传分片）与 `CompleteMultipartUpload` 同样会被拒。

**∴ 跨重启续传不可行。** 重启→必须重新取票→新 key + 只对该新 key 授权的凭证→
旧 upload_id / 旧分片无法复用，只能全量重传。

对照：**会话内**的退避重试用的是同一张票 / 同一 key / 同一凭证，policy 允许，
所以运行期断点续传（保留 upload_id 与已传分片）依然有效——这与跨重启是两回事。

## 5. 回退范围

已整体移除续传代码，回到「重启即全量重传」的干净状态：

- `crates/kichi-core/src/client.rs`：删除 `oss_list_parts`。
- `crates/kichi-core/src/upload.rs`：删除 `parse_list_parts` / `list_parts_*` 及其测试；
  `OssUploadState` 去掉 serde derive；`upload_oss` 去掉 `persist` 回调参数。
- `crates/kichi-gui/src/{worker,msg,settings}.rs`、`app/{mod,types,transfers_page}.rs`：
  删除续传记录、`Cmd::ResumeUpload` / `Msg::UlResumed`、启动还原与登录后自动分发、
  「续传中」UI 等。
- **保留** `app/mod.rs` 里的速率修复：`sample_speed`（`drain()` 单帧内多条进度按
  0.25s 节流取样，避免 `dt≈0` 让瞬时速率爆炸）、`has_active_uploads()` 与快轮询条件。

### 遗留的签名注意点（重要，避免日后再踩）

回退后 `oss_authorization` 又变回对所有方法**写死** `application/octet-stream`。
当前没有其它签名 GET，所以无碍；但**将来只要新增任何 OSS 无体 GET 请求（如
ListParts、HeadObject、下载类），必须按该方法实际发送的 Content-Type 签名**，
否则会重现 §4(b) 的 `SignatureDoesNotMatch`。稳妥做法是把 Content-Type 作为参数传入。

## 6. 未来若要真正实现，需要满足的前提（Option A）

唯一可能走通的路径是**绕开「重新取票换 key」**：

1. **持久化 STS 凭证本身**（`access_key_id/secret/security_token` + 过期时间），
   而不仅仅是 upload_id。重启后若凭证**仍在有效期内**，就**不重新 `upload_create`**，
   直接对**旧 key + 旧 upload_id + 旧凭证**续传；临近/已过期才退回全量重传。
2. **前提**：`upload_create` 响应里要带凭证有效期字段（本次**未验证**是否暴露）。
   需要时先补一次实测确认能否读到 expiration。
3. **安全权衡**：把临时凭证落盘属敏感操作。缓解：短时、限单一对象、价值有限；
   但仍是磁盘上的凭据，需评估（文件权限 0600、退出即清、不随日志外泄）。
4. **有效窗口有限**：只在「重启发生在 STS 寿命内」（约 1 小时）才续得上，隔夜无效。
   对「误关软件马上重开」够用，对「关机过夜」无用。
5. **尚未验证的下游风险**：即便旧 key 的分片续传成功，旧票与新票在网盘各建了一个
   drive 条目（不同 key）。对**旧 key** 完成 `CompleteMultipartUpload` 后，PikPak 到底
   把**哪个条目**落成最终文件、会不会残留一个空的**重复文件**——这一步没测，可能有坑。
6. **目录递归续传**是另一层复杂度（当前每文件独立取票，问题同单文件）。

## 7. 一句话总结

PikPak 的 STS 凭证按对象 key 授权、而 key 每次取票都换，导致「重新取票续旧
upload_id」这条路被服务端 policy 挡死；跨重启续传不可行，除非改为持久化并在有效期
内复用凭证本身（有安全与窗口限制，且完成落库环节仍需实测）。已回退代码、保留速率修复。
