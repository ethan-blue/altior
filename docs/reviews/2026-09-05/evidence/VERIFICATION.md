# 验证记录

评审日期：2026-09-05（Asia/Shanghai）。基线 commit：`f945ef153eb3344abd3801a3c2e4002fd25d80be`。

开始时 `git status --short` 为空。本轮只保留评审文档与证据；临时探针已从前端测试目录移入本目录并加 `.txt` 后缀，既有视觉 baselines 的本轮采集覆盖已恢复。没有修改产品实现、提交或发布。

实际工具环境：Windows NT `10.0.26200.0`；Node `v24.18.0`；npm `11.16.0`；rustc `1.98.0 (88d9e12ae 2026-08-18)`；cargo `1.98.0 (797e8a9bc 2026-08-05)`。这是本次机器的记录，不代表已证明项目声明的最低 Rust/Node 版本兼容。

## 实际执行结果

| 检查 | 实际命令 | 结果和限制 |
|---|---|---|
| Rust 格式 | `cargo fmt --all -- --check` | PASS；无格式 diff |
| Rust lint | `cargo clippy --all-targets --all-features -- -D warnings` | PASS；dev profile 完成，无拒绝级 warning |
| Rust workspace | `cargo test --workspace` | PASS；各测试二进制与 doc tests 结束，exit 0；包含 mock/合成轨迹，不等于真实第三方验收 |
| Frontend types | 在 apps/desktop：`npm run typecheck` | PASS |
| Frontend existing tests | 在 apps/desktop：`npm test` | PASS；11 文件，90 tests，约 2.59s |
| Frontend build | 在 apps/desktop：`npm run build` | PASS；JS 258.81 kB / gzip 78.98 kB，CSS 17.24 kB / gzip 3.46 kB；不是运行内存数据 |
| Tauri 单独 crate | `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | PASS；7 library tests；不等于 WebView 真实旅程/安装包验收 |
| 图像采集 | 在 apps/desktop：`npm run baselines` | 命令 exit 0，生成 5 张图；**画面不通过产品验收**，连接与布局错误见下 |
| 定向审查探针 | 在 apps/desktop：`npx vitest run src/test/review-20260905.test.tsx` | 7 probes PASS，含“错误现状断言”；表示问题确实存在，不表示功能正确；约 1.04s |
| 开发页 | `npm run dev -- --host 127.0.0.1` 后 PinchTab 打开 5173 | 页面有 fixture 界面，但连接不可用；实际 console 显示未处理异常 |
| 弹窗 Escape | PinchTab 从 Add Agent 打开弹窗，按 Escape | 弹窗仍可见，界面快照不变 |

Rust workspace 结果中的 `two_real_agents_complete_a_smoke_turn ... ok` 不应当作真实代理证据：源码在缺 `ALTIOR_ACP_SMOKE_AGENTS` 时输出 skipping 并 return。本轮没有为其配置真实代理，不能用名称和 ok 推断外部旅程完成。P1.4 的 dual agent 旅程使用独立 mock 程序。

没有为了文档任务重复所有检查；以上测试在未改产品源码状态下执行。新增资料只做文件/引用/工作区检查。

## 图像证据

- [light.png](light.png)：1280×800；Inspector 在左下，活动栏异常宽，nav 窄，布局有大块空白。
- [dark.png](dark.png)：同一几何问题；审批按钮仍白色原生样式。
- [narrow.png](narrow.png)：760×800；右侧抽屉遮住对话与审批区域。
- [error.png](error.png)：采集器选择合成错误会话后的画面，保留供后续比较。
- [approval.png](approval.png)：采集器选择合成审批会话后的画面，保留供后续比较。

本轮逐张查看了全部 5 张图；error/approval 同样存在 Inspector 落入左下的几何错误。审批状态的操作按钮和等待文案可见，但不能把合成状态视作真实授权端到端通过。所有画面来自项目自带合成 fixture 内容，没有截取用户真实 Vault。这些画面的审查结论是 FAIL，不是批准上线。

注意：原脚本调用 Vite preview 生产构建，默认入口没有显式 fixture transport；它等到 row 存在而没有等 handshake 成功。图上 IPC connecting 是实际错误状态，不是健康 fixture 的证明。这些截图用于保存缺陷，不能直接拿来作为新的批准视觉基线。

## 开发页异常

实际 Vite console 摘要（只保留代码位置，无用户内容）：

```text
[Unhandled rejection] TransportUnavailableError:
Tauri IPC capabilities are not available in the current environment.
#ensureAvailable src/ipc/tauriTransport.ts:213
TauriCoreTransport.subscribe src/ipc/tauriTransport.ts:145
Object.init src/stores/applicationStore.ts:881
src/app/App.tsx:77
```

开发模式入口中的 false/nullish 判断阻止了 import.meta.env.DEV fallback；subscribe 抛错又发生在 init 的 try 之前。StrictMode 下观察到重复错误报告，不将其解读为两个独立用户操作。

PinchTab 只操作本轮创建的 localhost 审查页。完成后关闭该 tab、撤销审查 session，停止本轮启动的 Vite server；没有关闭其他浏览器页或清理用户全局配置。

## 定向探针说明

源码归档：[review-probes.test.tsx.txt](review-probes.test.tsx.txt)。为复现可复制回原路径 `apps/desktop/src/test/review-20260905.test.tsx`，在前端目录运行上述命令，再删除该临时副本。原相对 import 路径依赖那个位置；不要直接从 evidence 目录运行。

| Probe | 确认了什么 | 不能据此声称什么 |
|---|---|---|
| IME composition Enter | onSend 实际被调用 | 未模拟完整 Windows 输入法候选窗口，仍需真实 IME 验收 |
| failed create | command 拒绝仍创建并返回本地 thread | 未声称这条假 thread 已写入真实 DB |
| failed cancel | command 拒绝仍 activeTurn=null | 未声称实际代理已经执行危险操作 |
| prompt identifiers | 实际发送 envelope 不符合 domain 字符/长度约束 | 没有改 Rust validator 或跑真实用户 turn |
| invalid secret reference | 普通无效字符串变成随机 ref | 没有输入真实密钥；这不是凭证泄漏实验证据 |
| subscribe throw | init rejects，state 留 connecting | 不证明所有网络错误都走相同分支 |
| empty search | store 返回空数组，同 App 的解引用路径抛 TypeError | 未在 App 的真实 Core 搜索渲染中做完整端到端复现 |

后续实现必须写“期望正确行为”的回归测试。不要保留这组“断言 bug 仍存在”的探针作为持续质量门槛。

## 对比度计算方法

从 tokens.css 读取十六进制颜色，按 sRGB→线性亮度，亮度权重 0.2126/0.7152/0.0722；比值 `(max(L1,L2)+0.05)/(min(L1,L2)+0.05)`。结果在 DESIGN_I18N.md，保留两位小数。未测所有动态混色和字体渲染。

外部规范只引用第一方：[W3C WCAG 2.2](https://www.w3.org/TR/WCAG22/)、[SQLite FTS5](https://www.sqlite.org/fts5.html)。它们支持设计/分词评估依据，不替代本项目实际测试，也不证明整体无障碍或性能合规。

## 尚未执行

- 两个真实第三方 ACP 在干净系统 profile 的全旅程。
- 真实 Tauri WebView 首次配置→发送→重启的用户操作旅程。
- 打包、签名、安装、升级、卸载、回滚和跨平台 UI。
- 100k messages + 100k memories 的真实 CPU/内存/启动/检索基准。
- Rust 内置 SQLite 下的中文 relevance/短词 benchmark。
- 三真实设备/网络 relay/密钥轮换/备份回滚/安全审计。
- 全屏幕读屏、forced-colors、200% 缩放和真实 Windows IME 候选输入。
- 全量依赖漏洞数据库/许可证合规审计。

这些项应标 NOT RUN / BLOCKED / 后续阶段，不允许根据现有绿测试补写 PASS。
