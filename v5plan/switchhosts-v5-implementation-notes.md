# SwitchHosts v5 Implementation Notes

## 目的

这份文件捕获 Phase 0 → Phase 2 实施过程中沉淀下来的关键决策、模式和已解决 / 待解决的债务。它的作用是**让任何在 Phase 3+ 阶段加入的人能在 30 分钟内重建上下文**——既包括为什么某些代码长成这个样子，也包括有意识保留下来的坑。

它不是设计文档（那些在 storage-plan / tauri-migration-plan 里），也不是教程，而是一份"如果我明天忘掉一切，我希望先读这个"的速查表。

---

## 当前进度（截至 Phase 2 + 债务清理完毕）

| 阶段 | 状态 | 代表 commit |
|---|---|---|
| Phase 0a — 骨架 | ✅ | `cac1402` |
| Phase 0b — 设计文档 | ✅ | `7bd027e` + `c262590` |
| Phase 1A — 适配层 + 命令 stub | ✅ | `ac0fda9` + `a17de18` |
| Phase 1B step 1 — 配置存储 | ✅ | `dbb5add` |
| Phase 1B step 2 — manifest/trashcan/entries 真实 I/O | ✅ | `a56f513` |
| Phase 1B step 2.5 — popup menu 桥 | ✅ | `e85b484` |
| Phase 1B step 3 — PotDb 迁移 | ✅ | `86de7df` |
| Phase 1B step 4 — 手动 import/export | ✅ | `061dbc8` |
| Phase 1B v5 manifest 格式重构 | ✅ | `5a7eeeb` |
| Phase 2.A — 主窗口生命周期 | ✅ | `9c92c54` + `3a3dd70` + `9d8afa9` |
| Phase 2.E.1 — hosts 内容聚合 + 预览命令 | ✅ | `dbf64cf` |
| Phase 2.E.2 — `/etc/hosts` 提权写入 + apply history | ✅ | `c69e74f` |
| Phase 2.E.3 — `cmd_after_hosts_apply` runner | ✅ | `97d268f` |
| Phase 2.E.4 — Linux pkexec + Windows UAC (编译覆盖) | ✅ | `05507a5` |
| Phase 2.B.1 — 系统托盘图标 + 菜单 + 标题 | ✅ | `05635d0` |
| Phase 2.B.2 — 托盘 mini-window (`/tray`) | ✅ | `f99a81b` |
| Phase 2.C — 完整应用菜单 + open_url | ✅ | `b5c160c` |
| Phase 2.D — 查找窗口 + 搜索引擎 | ✅ | `af4dbac` |
| Phase 2.F — remote hosts 刷新（手动 + 后台 cron） | ✅ | `7af1df3` |
| Phase 2.G — 本地 HTTP API (port 50761) | ✅ | `4d5127d` |
| P2.I 债务清理 — D2/D3/D6/D7/D8/D11 代码修复 | ✅ | `4403371` + `0171563` + `1847b1d` |
| P2.I 债务标记 — D1/D4/D5/D9/D10 文档归档 | ✅ | `1847b1d` |
| Phase 3 — updater + 发布链 | ⏳ | 未开始 |
| Cutover | ⏳ | 未开始 |

---

## 关键架构决策

### A1. Renderer 适配层在 Tauri 下合成 `IAgent` 门面

[src/renderer/core/agent.ts](/Users/wu/studio/SwitchHosts/src/renderer/core/agent.ts) 是整个迁移的"控制点"。它在模块加载时检测 `'__TAURI_INTERNALS__' in window`，分两条路径：

- **Electron 路径**：`window._agent` 直接返回（preload 早就装好了）
- **Tauri 路径**：用 `@tauri-apps/api` 的 `invoke` / `emit` / `listen` 现场合成一个跟 Electron 同形状的 `IAgent` 对象

这意味着 renderer 业务代码（`actions.xxx()`、`agent.broadcast()`、`agent.on()` 等）**完全不需要知道**自己跑在哪个壳里。新的 Tauri 命令只要在 [LEGACY_TO_NEW](/Users/wu/studio/SwitchHosts/src/renderer/core/agent.ts) 表里加一行重命名，renderer 就自动能用。

**Phase 2 启示**：新的 command 只需要在适配层加一行映射 + 在 `commands.rs` 加一个 `#[tauri::command]` + 在 `lib.rs` 注册。无需改 renderer。

### A2. Broadcast/listen 强制 envelope: `{_args: [...]}`

经过 Phase 1A 的 review 修复（commit `a17de18`），所有跨窗口事件 payload 都用 envelope 包一层：

```ts
// 发送
emit(channel, { _args: args })

// 接收
const xs = (event.payload?._args ?? [event.payload])
handler(...xs)
```

为什么：避免"一个数组参数"和"多个位置参数"的语义二义性。Rust 侧 emit 也走同样的 envelope（参见 [lib.rs 的 on_menu_event](/Users/wu/studio/SwitchHosts/src-tauri/src/lib.rs)）。

**Phase 2 启示**：任何 Rust 侧 `app.emit(channel, payload)` 都应该用 `json!({"_args": [...]})` 形式。直接 `app.emit("evt", "raw_string")` 会被 unwrap 函数当成"单参数 payload"处理，理论上能工作但破坏不变量。

### A3. 命令调用约定：所有命令接受 `args: Vec<serde_json::Value>`

renderer 端的 `actions.foo(a, b, c)` 通过适配层发出 `invoke('foo', { args: [a, b, c] })`，Rust 命令统一签名是：

```rust
#[tauri::command]
pub async fn foo(args: Vec<Value>) -> Value { ... }
```

需要 `AppState` 的命令再加一个 `state: State<'_, AppState>`：

```rust
#[tauri::command]
pub async fn foo(state: State<'_, AppState>, args: Vec<Value>) -> Result<Value, StorageError> { ... }
```

**Phase 2 启示**：

- 新命令统一遵循这个签名
- 参数解构靠 `args.get(0).and_then(Value::as_str)` 这种风格的 helper（参见 [`commands.rs::arg_str`](/Users/wu/studio/SwitchHosts/src-tauri/src/commands.rs)）
- 错误返回：业务软错误用 `Ok(Value::String("error_code"))`（renderer 用字符串分支判断），系统硬错误用 `Err(...)`（invoke promise reject）— 看 import_export 的三个命令是范例

### A4. Storage 层：内存形状 vs on-disk 形状解耦

`Manifest::root` 在内存里始终是 **renderer-friendly 的扁平 snake_case `IHostsListObject` 形状**（即 `is_sys`、`folder_mode`、`url`、`include`、`children`、`is_collapsed`）。

写盘前，[storage::tree_format](/Users/wu/studio/SwitchHosts/src-tauri/src/storage/tree_format.rs) 把它翻译成 v5 nested camelCase 形状（`isSys`、`source.{...}`、`group.include`、`folder.mode`、`contentFile`），并把 `is_collapsed` 抽出到 `internal/state.json > tree.collapsedNodeIds`。

读盘后，反向翻译：v5 → renderer 形状，把 collapsed id 注入回 `is_collapsed: true`。

**为什么不让 renderer 直接消费 v5 shape**：因为这条迁移路径的总原则是"不重写主要 UI 页面"。让翻译只发生在 storage I/O 边界，所有 commands、tree ops、import/export 都跟着内存形状走，零感知。

**Phase 2 启示**：

- 任何新的 manifest 字段（比如 P2.E 要给 remote 节点加 `last_apply_ms`）都要在 `tree_format.rs` 的 `legacy_root_to_v5` / `v5_root_to_legacy` 双向加上对应的翻译逻辑
- 节点上未建模的字段会进 `extras` 桶（双向保留），不会丢
- `state.json` 用了 `#[serde(flatten)] extras`，可以无缝加新字段（窗口几何、最近选中等）— Phase 2.A 就要利用这个

### A5. 写入用 `store_lock` 序列化

[`AppState::store_lock: Mutex<()>`](/Users/wu/studio/SwitchHosts/src-tauri/src/storage/mod.rs) 是一个空 mutex，专门用来序列化对 manifest.json + trashcan.json 的"读-改-写"周期。

每个会修改 manifest 或 trashcan 的命令都先取 lock：

```rust
let _guard = state.store_lock.lock().expect("store lock poisoned");
let mut m = load_manifest(&state).unwrap_or_default();
// ... mutate ...
save_manifest(&state, &m)?;
```

**为什么用 `std::sync::Mutex<()>` 而不是 `tokio::sync::Mutex`**：因为命令体里全是同步 I/O（`std::fs::*`），没有 `.await`，不会跨 await 持有 guard。clippy 不会警告。

**Phase 2 启示**：

- 任何对 manifest / trashcan 的写都必须持锁
- 持锁期间**不要**做 await（提权调用、HTTP 请求等异步操作）。先释放锁、做异步事情、再拿锁完成写入
- P2.E 的提权写入要小心：聚合可以在锁外做，提权调用必须在锁外（耗时几秒），写完后再取锁更新 manifest

### A6. 原子写：`atomic_write` + .tmp + rename

所有持久化文件（manifest.json、trashcan.json、entries/*.hosts、internal/config.json、internal/state.json）都通过 [`storage::atomic::atomic_write`](/Users/wu/studio/SwitchHosts/src-tauri/src/storage/atomic.rs)：先写到同目录的 `.tmp` 兄弟文件，再 rename 替换正式文件。rename 在同盘上是 OS 原子操作。

崩溃恢复策略：启动时**忽略 `.tmp` 文件**（只看正式文件）。`Manifest::load` / `Trashcan::load` / `AppConfig::load` 都遵循"文件不存在 → 内存默认；解析失败 → 内存默认 + 不动坏文件 + 警告"。

**Phase 2 启示**：

- 任何新加的持久化文件都走 `atomic_write`
- "崩溃后留下的 .tmp 文件应该清理"目前没有实现 — 这是 Phase 2.I 或 Phase 3 的债务

### A7. 命令调度：sync 命令跑 UI 线程，async 命令跑 tokio worker

Tauri 2 的规则：

- `#[tauri::command] pub fn x(...)` — sync，跑在 main UI thread。可以安全调用 menu / window 等需要 UI 线程的 API
- `#[tauri::command] pub async fn x(...)` — async，跑在 tokio worker thread。**不能**直接调 UI thread 限定的 API；必须用 `app.run_on_main_thread(|| ...)` 切换

[`popup_menu`](/Users/wu/studio/SwitchHosts/src-tauri/src/commands.rs) 是 sync 命令的范例（因为它需要原生菜单 API）。其他绝大多数命令（I/O、HTTP、计算）都是 async。

**Phase 2 启示**：

- P2.A / P2.B / P2.C / P2.D 涉及窗口创建、菜单操作的命令大多需要 sync 或 `run_on_main_thread`
- P2.E 的提权写入是阻塞的，但**不**需要 UI thread — 用 async 命令 + `tokio::task::spawn_blocking` 包住阻塞的 osascript / pkexec / UAC 调用，避免堵塞 worker thread

### A8. PotDb 迁移：lazy + 一次性 + 提交标记

[migration::run_if_needed](/Users/wu/studio/SwitchHosts/src-tauri/src/migration/mod.rs) 在 `AppState::bootstrap` 里调一次。触发条件**严格**：

> `manifest.json` 不存在 **且** `~/.SwitchHosts/{data,config}` 中至少有一个 PotDb 文件存在

写入顺序经过仔细推敲（参见 commit message `86de7df`）：

1. entries → 2. trashcan → 3. config → 4. histories → 5. **manifest.json（提交标记）** → 6. archive 旧目录到 `v4/migration-<ts>/`

manifest.json 是"迁移完成"的提交标记。Step 6 失败的话，下次启动看到 manifest.json 存在 → 跳过迁移 → 旧目录孤立但 v5 数据完整。Step 5 之前任何一步失败 → manifest.json 不存在 → 下次启动重试迁移（幂等）。

**Phase 2 启示**：

- 不要在迁移路径里加新的"迁移完成"判定（manifest.json 是唯一标记）
- 迁移失败的恢复路径："删 v5 文件 + 从 v4/ 移回旧目录"— 文档里提到，没自动化

### A9. v5 backup 格式有 `version: [5,0,0,0]` 字段

[import_export::export_to_file](/Users/wu/studio/SwitchHosts/src-tauri/src/import_export.rs) 输出的 v5 backup JSON 同时带 `format: "switchhosts-backup"` 和 `version: [5, 0, 0, 0]`。前者是 v5 import 的判别字段，后者是为了让老 Electron 看到 `version[0] === 5 > 4` 时报清晰的 `"new_version"` 错误，而不是神秘的解析错误。

**Phase 2 启示**：v5 → v6 升级时仍然要保留这个降级提示模式。

### A10. Per-window capability 拆分（已完成）

`capabilities/` 目录现在有三个文件：`main.json`（core:default + dialog:default）、`tray.json`（core:default）、`find.json`（core:default）。Tauri 自动合并目录下所有 capability 文件，每个窗口只获得其文件里列出的权限。只有 main 窗口保留 dialog:default（import/export 的 file picker）。

注意：Tauri 2 的自定义命令（`#[tauri::command]`）默认不受 capability 限制——所有窗口都能调用所有注册的命令。收口自定义命令需要额外的 ACL 配置，目前未做。如果未来需要严格隔离（比如不让 tray 窗口调 export_data），可以在 capability 文件里加 `deny` 规则。

---

## 已知问题与债务

### D1. `agent.once` 注册 race — ⏸️ 理论问题，实际不触发

`agent.once(channel, handler)` 在 Tauri 路径下内部的 listener 注册是异步的（`tauriOnce` 返回 Promise），但调用方（PopupMenu.show）在注册后立刻 invoke。理论上如果 Rust 极快 emit，listener 还没注册完毕，事件会丢失。

**实际不触发的原因**：Tauri 2 的 webview IPC 是 FIFO 队列。`invoke('plugin:event|listen')` 一定排在 `invoke('popup_menu')` 前面，所以 Rust 侧处理 popup_menu 命令时 listener 已经注册完毕。在烟雾测试中从未复现，popup_menu 在所有三个窗口（main、tray、find）都正常工作。

**如果未来要修**：把 `agent.once` 改成返回 `Promise<OffFunction>`，让 PopupMenu.show 变 async 并在 invoke 前 await 所有注册。改动面大（影响所有 once 调用方），风险高于收益，暂不做。

### D2. ~~`frontendDist: "../build"` 与 Electron 共享构建目录~~ ✅ 已在 P2.I 解决

新增 npm 脚本 `build:renderer:tauri`，使用 vite CLI 的 `--outDir build-tauri --emptyOutDir` 覆盖，输出到独立的 `build-tauri/` 目录。`tauri.conf.json` 的 `beforeBuildCommand` 和 `frontendDist` 都指向新路径。Electron 的 `build/` 目录不受影响。`build-tauri/` 已加入 `.gitignore`。

### D3. ~~`tauri.conf.json > version` 硬编码~~ ✅ Rust 侧已在 P2.I 解决

[build.rs](/Users/wu/studio/SwitchHosts/src-tauri/build.rs) 在编译期读取 [src/version.json](/Users/wu/studio/SwitchHosts/src/version.json)（`[4,3,0,6140]`），注入 `SWH_VERSION_MAJOR` / `SWH_VERSION_LABEL` / `SWH_VERSION_ARRAY` 等环境变量。`tray.rs::VERSION_LABEL`、`commands.rs::get_basic_data` 和 `http.rs::USER_AGENT` 全部从 `env!()` 取值，不再硬编码。

`tauri.conf.json` 的 `version` 字段仍需手动同步（Tauri 的 conf 不支持环境变量插值）。Phase 3 发布链可以加一个 npm pre-build 脚本来注入。

### D4. `bundle.targets: "all"` 在缺少打包器的机器上会 fail

[tauri.conf.json](/Users/wu/studio/SwitchHosts/src-tauri/tauri.conf.json) 的 `bundle.targets` 是 `"all"`，会尝试为所有平台打包。本地开发机如果没装 deb / appimage 工具会失败。

**修复方向**（Phase 3）：按构建主机的平台收窄。

### D5. macOS entitlements 要复用现有 plist

[scripts/entitlements.mac.plist](/Users/wu/studio/SwitchHosts/scripts/entitlements.mac.plist) 当前只有 JIT 相关两条权限。Tauri 打包时需要把这个 plist 接入到 `tauri.conf.json` 的 macOS 配置里。Phase 3 处理。

### D6. ~~孤儿 entries/*.hosts 没有 GC~~ ✅ 已在 P2.I 解决

`delete_item_from_trashcan` 和 `clear_trashcan` 现在在从 trashcan.json 移除条目后，通过 `manifest::collect_content_ids` 递归收集所有 local/remote 节点的 id，然后逐一调用 `entries::delete_entry` 删除对应文件。`entries::delete_entry` 的 `#[allow(dead_code)]` 也已移除。

### D7. ~~Per-window capability 拆分~~ ✅ 已在 P2.I 解决

`capabilities/default.json` 拆成了三个文件：`main.json`（core:default + dialog:default）、`tray.json`（core:default）、`find.json`（core:default）。Tauri 自动合并目录下所有 capability 文件。只有 main 窗口保留 dialog:default（import/export 的 file picker）。

### D8. ~~`import_data_from_url` 不走代理~~ ✅ 已在 P2.F 解决

抽出了 [src-tauri/src/http.rs](/Users/wu/studio/SwitchHosts/src-tauri/src/http.rs) 共享 reqwest client 构造逻辑（30s 超时 + UA + `use_proxy`/`proxy_*` 解析），`refresh::fetch_remote` 和 `commands::import_data_from_url` 都走它。后续要加 TLS 选项 / 重试只改一个地方。

### D9. tracer / `send_usage_data` 是 no-op — ⏸️ 有意保留

Electron 版的 tracer 当前也是注释掉的 no-op。v5 配置项保留但 Rust 侧没有任何实现。**有意识的不做**，未来如果重启上报功能再加。不算 debt，标记为 by-design。

### D11. ~~缺少 logger 后端~~ ✅ 已在 P2.I 解决

引入 `tauri-plugin-log` (v2)，在 `lib.rs::run` 的 Builder 链里用 `.plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())` 初始化。全代码库 28 处 `eprintln!` 批量替换为对应的 `log::info!` / `log::warn!` / `log::error!`（唯一例外：Windows elevation helper 的 `eprintln!` 保留，因为它在 logger 初始化之前运行）。同时获得了 tauri-plugin-log 的文件日志输出能力，便于 P3 发布后远程问题排查。

### D10. macOS / Linux / Windows 跨平台验收只在 macOS 上做 — ⏸️ Phase 3 / Cutover

整个 Phase 1 + 2 都在 macOS 上验证。Linux pkexec + Windows UAC 代码已写好（P2.E.4），但只通过了 macOS 上的 cargo check，未做实际运行验收。实际运行验收推迟到 Phase 3 / Cutover，那时会在各平台 CI 上跑完整测试。

---

## 自检 checklist（每个子步骤完成后用）

进 commit 前过一遍：

- [ ] `cargo check` 干净，**零警告**（必要的 dead_code 必须有 `#[allow(...)]` + 注释）
- [ ] `npm run typecheck` 项目源码零错误（`node_modules/vitest` 的 vite 内部错误是已知的，跳过）
- [ ] 新增的 `#[tauri::command]` 在 `lib.rs::generate_handler!` 里注册了
- [ ] 新增的 renderer 调用在 [agent.ts 的 LEGACY_TO_NEW 表](/Users/wu/studio/SwitchHosts/src/renderer/core/agent.ts) 或 snake_case 自动转换里能解析
- [ ] 任何 store/manifest/trashcan 的写都走了 `state.store_lock`
- [ ] 任何持久化都用了 `atomic_write`
- [ ] 任何 emit 都包了 `{_args: [...]}` envelope
- [ ] 涉及 manifest 字段变化时，[tree_format.rs](/Users/wu/studio/SwitchHosts/src-tauri/src/storage/tree_format.rs) 的双向翻译表也更新了
- [ ] commit message 说清楚做了什么、为什么、留下了什么 known issue

---

## 速查：当前的文件清单

```
src-tauri/src/
├── main.rs                      # 仅入口
├── lib.rs                       # Builder 装配 + invoke_handler 注册 + 全局菜单/托盘事件
│                                  + Windows elevation helper early argv check
│                                  + background scanner spawn + HTTP API boot
├── app_menu.rs                  # 完整应用菜单（File/Edit/View/Window/Help）(P2.C)
├── commands.rs                  # ~52 个 #[tauri::command]，全部已接通真实实现
├── find.rs                      # 查找窗口创建 + 搜索引擎 + find/replace 历史 (P2.D)
├── http.rs                      # 共享 reqwest::Client 构造（代理 + UA + 超时）
├── http_api.rs                  # 本地 HTTP API (axum, port 50761, 4 routes) (P2.G)
├── import_export.rs             # v3/v4/v5 backup 读写 (Phase 1B step 4)
├── lifecycle.rs                 # 主窗口创建 + 几何持久化 + close/reopen/dock (P2.A)
├── refresh.rs                   # remote hosts 刷新 + 60s 后台 cron (P2.F)
├── tray.rs                      # 系统托盘图标 + 菜单 + mini-window + 标题 (P2.B)
├── hosts_apply/
│   ├── mod.rs                   # 公共 re-exports
│   ├── aggregate.rs             # 选中节点内容聚合 + 去重 (P2.E.1)
│   ├── write.rs                 # 写入编排：行尾规范化 / append 模式 / 直写→提权 (P2.E.2)
│   ├── elevation.rs             # macOS osascript / Linux pkexec / Windows ShellExecuteExW
│   ├── error.rs                 # HostsApplyError enum (NoAccess / Cancelled / Io)
│   ├── history.rs               # internal/histories/system-hosts.json 日志
│   └── cmd_runner.rs            # cmd_after_hosts_apply 30s 超时执行器 + 日志 (P2.E.3)
├── migration/
│   ├── mod.rs                   # PotDb 首次启动迁移编排
│   ├── potdb.rs                 # 直接解析 PotDb 目录的 reader
│   └── archiver.rs              # 旧布局归档到 v4/migration-<ts>/
└── storage/
    ├── mod.rs                   # AppState bootstrap + 公共 re-exports
    ├── atomic.rs                # atomic_write 助手
    ├── error.rs                 # StorageError enum (serde tagged)
    ├── paths.rs                 # V5Paths 结构
    ├── config.rs                # internal/config.json AppConfig
    ├── state.rs                 # internal/state.json StateFile (含窗口几何)
    ├── manifest.rs              # manifest.json + 树操作 + state 联动
    ├── trashcan.rs              # trashcan.json
    ├── entries.rs               # entries/<id>.hosts read/write/delete (含 GC)
    └── tree_format.rs           # legacy <-> v5 节点形状双向翻译

src-tauri/capabilities/
├── main.json                    # main 窗口：core:default + dialog:default
├── tray.json                    # tray 窗口：core:default
└── find.json                    # find 窗口：core:default

src-tauri/
├── Cargo.toml                   # tauri 2 (tray-icon, image-png) + plugin-dialog
│                                  + plugin-log + plugin-single-instance + reqwest
│                                  + chrono + tokio + regex + axum + open
│                                  + [target.windows] windows-sys 0.60
├── tauri.conf.json              # devUrl → 127.0.0.1:8220; frontendDist → ../build-tauri
├── build.rs                     # tauri_build + 版本号注入 (SWH_VERSION_*)
└── icons/
    ├── icon.icns / icon.ico / icon.png / 32x32 / 64x64 / 128x128(@2x)  # app 图标
    ├── tray-mac.png             # macOS 菜单栏模板图标 (32×32, B/W + alpha)
    └── tray.png                 # Linux/Windows 托盘图标 (512×512, 彩色)

src/renderer/
├── core/agent.ts                # 运行时分发：Electron preload vs Tauri invoke
├── core/PopupMenu.ts            # 原生右键菜单桥
├── components/List/ListItem.tsx # 修复：订阅 set_hosts_on_status 回滚开关状态
├── pages/tray.tsx               # 托盘 mini-window UI (P2.B.2)
└── pages/find.tsx               # 查找窗口 UI (P2.D)
```

---

## Phase 3 前置事项

Phase 1 + 2 的功能迁移和债务清理已全部完成。进入 Phase 3（updater + 发布链）前，以下事项需要落地：

| 事项 | 状态 | 说明 |
|---|---|---|
| D4: `bundle.targets` 收窄 | ⏳ | 按 CI 构建主机平台配置，避免在缺少 deb/appimage 工具的机器上 fail |
| D5: macOS entitlements plist | ⏳ | 把现有 `scripts/entitlements.mac.plist` 接入 `tauri.conf.json` |
| D10: 跨平台运行验收 | ⏳ | Linux + Windows 上首次真实运行测试，重点验证 pkexec / UAC elevation |
| `tauri.conf.json > version` 自动注入 | ⏳ | npm pre-build 脚本从 `src/version.json` 写入 conf（Rust 侧已自动） |
| Tauri updater 插件接入 | ⏳ | `tauri-plugin-updater`，替换 `electron-updater` |
| 签名 + 公证 | ⏳ | macOS: code sign + notarize; Windows: code sign; Linux: N/A |
| CI/CD 发布脚本 | ⏳ | GitHub Actions workflow，三平台构建 + 发布到 GitHub Releases |
| `.tmp` 文件清理 | ⏳ | 启动时扫描并删除 `atomic_write` 留下的 `.tmp` 残留 |

## 文档维护

进 Phase 3 之后，每完成一个子步骤，更新本文件的：

- "当前进度"表的对应行
- "Phase 3 前置事项"表的状态
- "速查"清单里新增的文件

`phase2-plan.md` 在 Phase 2 完成后变为只读参考。Phase 3 如需独立计划文档，在 `v5plan/` 下新建。
