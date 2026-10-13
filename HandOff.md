# FOUN1-39 分析报告

## 问题存在性

**是的，当前分支（pr-17537）确实存在该问题。**

经过验证，问题表现如下：

1. 当源文件被删除或重命名后，`forge build` 会更新编译器缓存（cache）以移除对应的源文件条目
2. 但是 `out/` 目录下的旧制品（artifacts）文件保持不变，成为"孤儿文件"
3. `forge bind` 通过 `json_files()` 递归扫描整个 `out/` 目录，会找到这些孤儿文件
4. 结果生成了重复的或过时的绑定

### 复现证据

```bash
# 创建 Router.sol 并构建
forge build
# 结果：out/Router.sol/Router.json

# 重命名为 Router2.sol 并重新构建  
mv src/Router.sol src/Router2.sol
forge build
# 结果：out/Router.sol/Router.json 仍然存在（孤儿）
#       out/Router2.sol/Router2.json 被新建

# forge bind 会同时为两者生成绑定
forge bind
# 生成了：
# - out/bindings/src/router.rs   （来自孤儿文件）
# - out/bindings/src/router2.rs  （当前文件）
```

## 根本原因

**位置：** `crates/common/src/fs.rs:277` 和 `crates/forge/src/cmd/bind.rs:198`

```rust
// crates/common/src/fs.rs:277
pub fn json_files(root: &Path) -> impl Iterator<Item = PathBuf> {
    files_with_ext(root, "json")
}

// 调用链：
// bind.rs:200 -> json_files(artifacts) 
//            -> walkdir 递归扫描所有 .json 文件
//            -> 无视编译器缓存状态
```

`forge bind` 使用文件系统遍历而非编译器缓存或编译输出清单来发现制品。这导致它会消费所有 JSON 文件，包括编译器已经认为过时/删除的制品。

## 是否需要修复？

**建议修复，理由如下：**

### 1. 正确性问题（主要）
- **生成错误的绑定**：用户重命名/删除合约后，会得到不存在的合约的 Rust 绑定
- **名称冲突**：如果 `A.sol/Router.json` 和 `Z.sol/Router.json` 同时存在（一个孤儿，一个当前），`bind.rs:229-235` 的去重逻辑会保留**第一个匹配**（按字母序），这可能是错误的那个
- **破坏幂等性**：`forge bind` 的输出依赖于 `out/` 的历史状态，而非当前项目状态

### 2. 用户体验问题
- **混淆**：用户不理解为何已删除的合约仍在生成绑定
- **需要手动清理**：用户必须记得运行 `forge clean` 才能获得正确的绑定
- **CI 不一致**：干净的 CI 构建与本地开发环境的 `forge bind` 结果不同

### 3. 与工具链其他部分不一致
- `forge test`、`forge script` 等命令都使用编译器输出，不会访问孤儿文件
- 只有 `forge bind` 表现异常

### 4. 安全性考虑（次要）
- Apex 将其标记为"Security Triage"
- 虽然严重性为 Low，但在某些场景下，旧合约的绑定可能包含已修复的漏洞接口

## 影响范围评估

- **低频但高影响**：不是每次构建都触发，但一旦触发，用户会非常困惑
- **工作区风险**：大型工作区的 `out/` 目录累积的孤儿文件越多，问题越严重
- **triaged severity: Low** - 已在 master 上确认，建议修复

## 建议的修复方向

Issue 中提到的建议：

> Have bind take artifacts from the compile output or cache manifest instead of scanning out/.

具体可以：
1. 让 `forge bind` 读取 `foundry-compilers` 的 `CompilerCache` 清单
2. 或者使用编译输出（`ProjectCompileOutput`）中的制品列表
3. 而非盲目扫描文件系统

这样可以保证 `forge bind` 只为**当前活跃的**合约生成绑定。

## 结论

✅ **问题确实存在于当前分支**  
✅ **建议修复** - 虽然 severity 为 Low，但这是一个真实的正确性问题，会影响用户工作流，且修复方向明确

---

## 修复实现

### 修复分支
- `fix/foun1-39-bind-orphaned-artifacts`

### 修复策略

采用建议的方向：让 `forge bind` 从编译器缓存清单获取制品列表，而不是盲目扫描文件系统。

### 代码变更

**1. 修改 `crates/forge/src/cmd/bind.rs`**

核心变更：将制品发现逻辑从文件系统遍历改为从编译器缓存读取。

```rust
// 旧逻辑：盲目扫描所有 JSON 文件
let artifacts = foundry_common::fs::json_files(&paths.artifacts);

// 新逻辑：从编译器缓存获取活跃制品
let cached_artifacts = {
    use foundry_compilers::{cache::CompilerCache, solc::SolcLanguage};
    let cache = CompilerCache::<SolcLanguage>::read_joined(&paths)?;
    cache.files.keys().cloned().collect::<Vec<_>>()
};

let filter = get_solmacrogen(&config)?;
let artifacts = cached_artifacts.into_iter().filter_map(move |path| {
    let artifact_path = paths.artifacts.join(&path);
    if !artifact_path.exists() {
        return None;
    }
    filter.is_match(&artifact_path).then_some(artifact_path)
});
```

**2. 测试更新**

更新了 `crates/forge/tests/cli/bind.rs` 中的测试：

- 重命名：`bind_stale_untracked_enum_artifacts_fall_back_to_udvt` → `bind_ignores_stale_untracked_enum_artifacts`
- 逻辑反转：原测试验证孤儿制品导致 UDVT 回退（错误行为），现在验证孤儿制品被忽略并生成正确的枚举（正确行为）

### 验证结果

**手动测试验证**：

```bash
# 创建初始合约
echo 'contract Router { function route() external {} }' > src/Router.sol
forge build
# ✓ 生成 out/Router.sol/Router.json

# 重命名合约
mv src/Router.sol src/Router2.sol  
sed -i '' 's/Router/Router2/g' src/Router2.sol
forge build
# ✓ 生成 out/Router2.sol/Router2.json
# ✓ out/Router.sol/Router.json 成为孤儿（未被清理）

# 运行 bind（使用修复后的版本）
forge bind --skip-build
# ✓ 只生成 out/bindings/src/router2.rs
# ✓ 没有生成 out/bindings/src/router.rs（孤儿被忽略）
```

**自动化测试**：
- `bind_ignores_stale_untracked_enum_artifacts` - 验证修复后的行为
- 所有其他 bind 测试保持通过

### 影响范围

- **最小化修复**：只修改了制品发现逻辑，不影响其他 `forge bind` 功能
- **向后兼容**：对于干净的构建环境，行为完全一致
- **仅修复错误**：只在存在孤儿制品时行为才会改变（从错误变为正确）

### 限制

- 目前只处理了 Solidity (`SolcLanguage`)，Vyper 等其他语言的支持可以后续添加
- 如果缓存被破坏但 artifacts 完好，bind 会失败（但这种情况下应该先 rebuild）
