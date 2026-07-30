# CLI 用法

> 极简命令。`avc --help` 是详细参考；这里给稳定语义。

---

## 1. 起步

```bash
avc init                                       # 创建 ~/.local/share/avc/
avc config set provider.llm.openai.api_key "sk-..."
avc doctor                                     # preflight：DB / token / 网络
```

---

## 2. PersonaModel 生命周期

### 创建 v1

```bash
avc persona new yu --from ./yu.toml
# yu.toml 关键字段：
# [persona]
# name = "Yu"
# archetype = "db_kernel_expert"
# [avatar] description = "数据库内核专家"
# [voice]  language = "zh", samples = ["sample.wav"]
# [persona_descriptor] traits = ["严谨","务实"]
# [knowledge] corpus = "./db-internals.md", domain = "数据库内核"  # 可选
```

完成返回 `persona_id (pm_xxx), version_id (1)`。

### 查看

```bash
avc persona list
avc persona show yu
avc persona versions yu
avc persona inspect yu --version 2
avc persona dump yu --version 2 --out ./dump/   # 只读视图
```

### 切换默认版本

```bash
avc persona current yu --set 2
```

不影响已渲染视频。

### 归档

```bash
avc persona archive yu
```

永物理删除（除非 archive 后 30d `avc prune`）。

---

## 3. 持续训练

### 追加样本

```bash
avc persona sample add yu --kind audio --uri ./new.wav --text "..." --consent ./auth.pdf
avc persona sample add yu --kind behavior_text --text "..."
```

### 启动训练

```bash
avc persona evolve yu --scope voice --base-version 2 --threshold 0.85
# → INSERT 预占 v3 行；执行训练；抽 anchor；drift_eval
# → 达标: UPDATE status=ready; 失败: DELETE 整行事务回退
```

查进度：

```bash
avc task show tj_xxx --watch
```

报告：

```bash
avc training report tj_xxx --json
```

### 样本治理

```bash
avc persona sample list yu
avc persona sample rm smp_xxx
```

---

## 4. 出片

```bash
avc render video --persona yu --version 2 --topic "InnoDB Buffer Pool" \
                 --duration 60 --resolution 1080p

avc job show job_xxx --watch
avc job inspect job_xxx
avc job export job_xxx --out ./final.mp4     # BLOB -> FS
avc job export job_xxx --all --out ./export/
```

只读分镜不渲染：

```bash
avc render script --persona yu --topic "..." --out script.json
avc render script edit script.json --patch 'scenes[0].duration_ms=9000'
avc render video --from-script script.json
```

---

## 5. 反馈

```bash
avc job feedback job_xxx --signal looks_unlike --note "侧脸不像"
# → 转 PersonaSample(kind=feedback)
# → 下次 evolve 自动消费
```

---

## 6. 知识（可选）

```bash
avc corpus new --name "数据库内核" --source-type upload --uri ./db-internals.md
avc corpus chunks add corpus_xxx --from ./chunks.jsonl
avc corpus search corpus_xxx --query "Buffer Pool"
avc corpus reindex corpus_xxx
avc persona knowledge bind yu --corpus corpus_xxx --domain "数据库内核"
avc persona knowledge unbind yu
```

---

## 7. 系统命令

```bash
avc doctor                                     # preflight（DB / token / 网络）
avc verify                                      # 全表 sha256 校验
avc verify --persona yu
avc backup --out backup.db
avc restore --from backup.db
avc export --persona yu --out yu.tar.zst
avc import yu.tar.zst
avc repl
```

---

## 8. 错误约定

退出码：

| code | 含义 |
|------|------|
| 0 | ok |
| 1 | 通用失败 |
| 2 | 参数错 |
| 3 | 资源不存在 |
| 4 | 状态冲突 |
| 5 | 鉴权失败 |
| 6 | token 未配置 |

格式：

```
error[E0501]: provider_unauthenticated
  provider: provider.avatar.kling
  hint: avc config set provider.avatar.kling.api_key ...
```

---

## 9. REPL

```bash
avc repl
```

```
avc> persona list
  pm_01... (yu)        current=v3   versions=3

avc> persona evolve yu --scope voice --add ./new.wav
  tj_xxx  watching...
  ✓ drift_eval passed (0.92)
  ✓ published v3

avc> render video --persona yu --topic "..."
  job_xxx  watching...
  ✓ succeeded  → use `avc job export job_xxx --out ./final.mp4`

avc> exit
```

上下文：`$LAST` 引用上一条结果；上箭头历史；Tab 补全。
