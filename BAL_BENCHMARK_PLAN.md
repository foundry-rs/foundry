# Cast BAL benchmark 范围与验收

状态：已按合并后的 PR #16931 完成工具简化和本地验证，复现命令见下文。
本文件定义当前范围，不包含真实 RPC 性能结论。

目标是回答同一笔交易使用 BAL 和完整前缀 replay 的耗时、RPC 开销与结果是否一致。
保留 `benches/` 内独立的 `foundry-bal-bench`，避免将交易哈希输入强塞进现有 Forge
项目 benchmark。生产 Cast、持久化 fork 与复用 BAL 的其他工作流不在本次改动范围。

## 对照与输入

| Arm | 同一个 Cast binary 的调用 | RPC 策略 |
| --- | --- | --- |
| `auto` | `cast run TX` | 原样转发；记录实际 BAL 命中或自然回退。 |
| `replay` | `cast run TX --no-bal` | 完整前缀 replay；必须没有 BAL 请求。 |
| `miss`（可选） | `cast run TX` | 本地立即注入 method-not-found；其余请求原样转发。 |

默认仅运行 auto/replay，`--include-miss` 添加诊断对照。本地注入的 miss 不包含真实
provider 返回不支持方法的网络往返。`--quick` 和 `--prestate-tracer` 单独验证优先级，
不能代替 replay 基线。不存在统一的 500 ms BAL deadline，也不假设执行中失败会自动重放。

必须显式提供支持 `--no-bal` 的 `--cast`；旧二进制提前拒绝。
`--build-manifest` 可选，没有它时仍记录 binary hash/version。比较 Git refs 时，
`pr-bal-bench.sh` 每个不同 SHA 只构建一个未改动的 profiling binary，使用 `--locked`、
一致 features/toolchain，输出 `build.json`。相同 SHA 复用 binary，不再维护源码补丁。
显式选择两个支持 `--no-bal` 的 refs；本分支基于 PR #16931 的合并提交
`f2a71665c921baaca6395b42af4d53853c87af0a`，旧版 BAL 分支不能用作该对照。

先从 finalized metadata 固定 panel，再观察 BAL 可用性。默认 12 个非空块，覆盖普通块、
候选区间交易数 top decile 的大块，以及去重后的首/中/末笔交易。固定 seed、区间与区块
身份，保留不支持 BAL 的样本。Cancun 及之后的历史 pre-Amsterdam 块按 provider 能力
探测，不能仅因 Amsterdam 尚未激活而排除；pre-Cancun 块直接 replay。
真实 panel 不保证覆盖全部边界，边界由合成 fixture 补充。

## 正确性与 issue 对应关系

- 首/中/末、单交易块和较大块：验证 target 前状态的索引边界。
- 重复账户/存储修改、系统操作：通过读取合约与完整 trace/output 对照，检查只应用
  target 前修改，排除 target 自身与后续交易、块结束修改。
- pre-Amsterdam、unsupported、null、malformed：验证真实探测和可观察的回退。
- pre-Cancun（Shanghai）：三组均直接 replay 且不发 BAL 请求，可选 miss 不阻断有效对照。
- 延迟成功响应：记录实际命中；整体 child timeout 是测量截止时间，不是产品 BAL 限时。
- 缺失前序普通交易存储修改的 BAL：保留与 replay 的差异，阻断性能结论；不把 HTTP 成功视为正确状态。
- `--quick` 和 opt-in `--prestate-tracer`：检查 BAL 是否按实际模式优先级被绕过，
  prestate 不可用时检查当前实现的后续路径。

计时前比较完整 trace、gas 与执行状态；正式样本继续校验 replay oracle 的输出 hash。
已知 replay/系统操作差异必须原样保留，不能改生产代码或删除不利样本来获得速度结论。
这些检查为当前 Cast 路径提供回归证据，不构成对任意缺损 BAL 的安全性证明，
也不代表其他 transaction-hash fork 工作流已接入 BAL。

## 测量与报告

每个样本启动独立 Cast process，禁用客户端磁盘 storage cache；两组顺序按 round 交替。
跨 ref 也交替运行完整 round，预热和验证不进入正式统计。服务器缓存与 BAL 是否即时
生成默认记为 unknown；proxy 的 RPC 时长包含网络和服务时间，不推断服务端 CPU 开销。

输出 wall time 的 median/IQR/min/max、各 RPC 方法请求数、客户端/上游字节数、BAL
响应时长与 payload，以及实际命中/回退/失败/超时。paired delta 为 auto 减 replay；
speedup 为 replay median 除以 auto median。仅当该 case 所有计划 pairs 完整、唯一且
等价时报告速度比较。缺失、重复、超时、错误、unknown 和 correctness-blocked 保留；
条件于 BAL hit 的数字不能冒充整体收益。并发 RPC 时长不能相加当成端到端耗时。

保留可重算的 `manifest.json`、`samples.jsonl`、`rpc-events.jsonl` 和子进程输出，
离线生成 `summary.json`、`report.md`。有效样本投影到通用结果 schema；没有有效样本时
不伪造零耗时。真实数据与合成 fixture 明确区分。本轮不做 durable eligibility census。

## 执行与完成标准

具体 CLI、构建与 artifacts 说明见 [benches/README.md](benches/README.md#cast-block-access-list-benchmarks)。

```sh
python3 benches/scripts/test-pr-bal-bench.py -v
cargo test -p foundry-bench --bin foundry-bal-bench
python3 benches/scripts/test-bal-bench.py \
  --cast /absolute/path/to/bal-capable/cast \
  --anvil /absolute/path/anvil --runner /absolute/path/foundry-bal-bench \
  --include-miss --output-dir /tmp/foundry-bal-local-check
```

完成条件是同 binary 对照、边界/故障 fixtures、RPC 计量、差异阻断、schema 和报告一致，
且相关检查通过或明确记录外部阻塞。工具测试通过不等于真实网络提速：固定 finalized
panel 的性能采样需另行执行并报告 provider、refs、缓存条件、失败与噪声。

Better path: 采用同 binary 的默认路径/`--no-bal` 小型对照，保留正确性门槛和 RPC 归因；
它比双构建补丁与独立 census 更直接回答 issue，代价是暂不评估持久化 fork，现阶段采用。
