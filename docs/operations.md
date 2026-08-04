# AVCore 部署与运维手册

> **范围**：Linux 单机、单租户。Debian 12 / Ubuntu 22.04+ / RHEL 9+ / Arch 验证。
> **不涵盖**：多用户/HA/跨机/Windows/macOS（参考 `docs/status.md` Phase 3+ 路线图）。

---

## 1. 安装

### 1.1 从源码构建

```bash
git clone https://github.com/avcore/avc.git
cd avc
cargo install --path . --locked      # → ~/.cargo/bin/avc
# 或：不安装到 cargo bin，构建 release 二进制
cargo build --release                # → target/release/avc
```

需要：Rust 1.78+ toolchain（`rustup default stable`）。

### 1.2 验证

```bash
avc version          # 应输出 "avc 0.3.x"
avc doctor           # 集成诊断
```

预期输出：
```
✓ 数据库: /home/<user>/.local/share/avc/avc.db
✓ 配置:   /home/<user>/.config/avc/avc.toml
doc: avc init  # 若以上缺失
```

### 1.3 首次初始化

```bash
avc init                                   # 建库 + 默认目录
mkdir -p ~/.config/avc
cp examples/avc.toml.template ~/.config/avc/avc.toml
# 编辑 avc.toml，填入 provider api_key / model / base_url
chmod 600 ~/.config/avc/avc.toml           # API key 必须 0600（Config::save 自动设）
```

---

## 2. 数据与文件布局

| 路径 | 用途 | 权限 |
|---|---|---|
| `~/.local/share/avc/avc.db` | SQLite 主库 | 0600（Config 默认 umask） |
| `~/.config/avc/avc.toml` | Provider 配置 + API key | 0600（强制） |
| `~/.local/share/avc/avc.pid` | daemon PID 文件 | 0644 |
| `~/.local/share/avc/avc.log` | daemon tracing 日志（append） | 0644 |

> 改 `XDG_DATA_HOME` / `XDG_CONFIG_HOME` 可整体搬到其他目录（如 `/var/lib/avc/`）。

---

## 3. 启动模式

### 3.1 前台（开发/调试）

```bash
avc _run                   # 隐藏 verb，不在 --help 里
# Ctrl-C 退出（SIGTERM / SIGINT 均处理）
```

前台模式下日志直接走 stderr，方便 `journald` / `tmux` / `screen` 跟踪。

### 3.2 后台 daemon

```bash
avc daemon start           # fork 子进程跑 _run，写 pidfile，exit 0
avc daemon status          # 看 pid / alive / log path
avc daemon logs            # tail 最近的 avc.log
avc daemon stop            # 发 SIGTERM 给 pidfile 的 pid
```

daemon 行为约定：
- 父进程立刻返回（不阻塞）
- 子进程 stdout/stderr 重定向到 `/dev/null`（detach）
- 唯一输出通道是 `<data_dir>/avc.log`
- 收到 SIGTERM 或 SIGINT 才退出

---

## 4. systemd unit（推荐生产部署）

### 4.1 Unit 文件

把下面写到 `/etc/systemd/system/avc.service`（用 `root`）：

```ini
[Unit]
Description=AVCore daemon (provider health + render)
Documentation=https://github.com/avcore/avc
After=network-online.target
Wants=network-online.target

[Service]
Type=forking
User=avc
Group=avc
WorkingDirectory=/var/lib/avc
Environment=XDG_DATA_HOME=/var/lib/avc
Environment=XDG_CONFIG_HOME=/etc/avc
ExecStart=/usr/local/bin/avc _run
PIDFile=/var/lib/avc/avc.pid
Restart=on-failure
RestartSec=5
TimeoutStopSec=15
KillMode=mixed
KillSignal=SIGTERM
StandardOutput=null
StandardError=null
# 限额：≤ 50 persona × 5 版本 = 6GB；按机器调整
MemoryMax=4G
TasksMax=64

[Install]
WantedBy=multi-user.target
```

### 4.2 配套目录

```bash
sudo useradd -r -s /usr/sbin/nologin -d /var/lib/avc avc
sudo install -d -o avc -g avc -m 0750 /var/lib/avc
sudo install -d -o avc -g avc -m 0750 /etc/avc
sudo install -o root -g root -m 0755 /usr/local/bin/avc /usr/local/bin/avc
sudo install -o root -g root -m 0600 avc.toml /etc/avc/avc.toml
sudo chown -R avc:avc /var/lib/avc /etc/avc
```

### 4.3 启用

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now avc
sudo systemctl status avc
avc daemon status    # 跨进程状态
```

systemd 看 PID 走 `PIDFile=`，`avc daemon start` 也会自己写同一个 pidfile — 两者一致。

---

## 5. 备份与恢复

### 5.1 ⚠️ 当前状态

**`avc backup` / `avc restore` 已在文档（`docs/storage.md`）但尚未实现**（CLI 报错"未知子命令"）。本期用 OS 级别的备份代替。

### 5.2 文件系统级备份（推荐）

daemon 写入 SQLite 走 WAL（`~/.local/share/avc/avc.db-wal`、`-shm`）。安全在线备份方法：

```bash
# 方案 A：让 avc 关 WAL，让备份脚本能拿稳定快照
sqlite3 ~/.local/share/avc/avc.db ".backup '/var/backups/avc-$(date +%F).db'"

# 方案 B：先停 daemon 再 cp（最简单）
sudo systemctl stop avc
sudo install -o avc -g avc -m 0640 \
  /var/lib/avc/avc.db /var/backups/avc-$(date +%F).db
sudo systemctl start avc
```

### 5.3 备份策略（推荐）

| 周期 | 保留 | 工具 |
|---|---|---|
| 每天凌晨 02:00 完整备份 | 30 天 | sqlite3 `.backup` 或 systemd timer + cp |
| 每周一异地同步 | 永久 | rclone / rsync 到异地存储 |

最低限度：`/var/backups/avc/` 目录由 systemd-timer 每日执行 `sqlite3 .backup`：

`/etc/systemd/system/avc-backup.timer`：
```ini
[Unit]
Description=Daily AVCore backup

[Timer]
OnCalendar=*-*-* 02:00:00
Persistent=true

[Install]
WantedBy=timers.target
```

`/etc/systemd/system/avc-backup.service`：
```ini
[Unit]
Description=AVCore backup job

[Service]
Type=oneshot
User=avc
Group=avc
ExecStart=/usr/bin/sqlite3 /var/lib/avc/avc.db ".backup '/var/backups/avc/avc-$(date +\\%%F).db'"
```

`sudo systemctl enable --now avc-backup.timer`。

### 5.4 恢复

```bash
sudo systemctl stop avc
sudo cp /var/backups/avc/avc-2026-08-04.db /var/lib/avc/avc.db
sudo chown avc:avc /var/lib/avc/avc.db
sudo systemctl start avc
avc doctor          # 验库完整
avc persona list    # 验数据可见
```

---

## 6. 升级流程

```bash
# 1. 停 daemon
sudo systemctl stop avc

# 2. 备份
sqlite3 /var/lib/avc/avc.db ".backup '/var/backups/pre-upgrade-$(date +%F).db'"

# 3. 拉新代码 / 构建
cd /opt/avc && git pull && cargo build --release
sudo install -m 0755 target/release/avc /usr/local/bin/avc.new
sudo mv /usr/local/bin/avc.new /usr/local/bin/avc

# 4. 启动（migration 自动跑；migration 编号见 docs/storage.md）
sudo systemctl start avc
sudo journalctl -u avc -n 50     # 确认 migration 跑了

# 5. 验
avc doctor
avc persona list | wc -l    # 应与升级前数量一致
avc provider status --json  # 探活 1-2 次确认 daemon 健康
```

### 6.1 回滚

```bash
sudo systemctl stop avc
sudo cp /usr/local/bin/avc.v2026-08-04 /usr/local/bin/avc   # 旧二进制
sqlite3 /var/lib/avc/avc.db ".restore '/var/backups/pre-upgrade-2026-08-04.db'"
sudo systemctl start avc
```

> 注意：`sqlite3 .restore` 只能恢复到比当前 schema 旧版本。**如果新版 migration 改了 schema，回滚要降级前先备份再还原**。

---

## 7. 日志管理

### 7.1 路径与格式

`<data_dir>/avc.log` 由 daemon `tracing-subscriber` 写，append 模式。格式：

```
2026-08-04T03:56:14.324549Z  INFO avc::svc::daemon: daemon listening on 127.0.0.1:7891
2026-08-04T03:56:14.330012Z WARN avc::provider::probe: probe_all error: ...
```

读：
```bash
avc daemon logs                # tail 最后 2000 字符
tail -f /var/lib/avc/avc.log    # 实时跟踪
journalctl -u avc              # 如果走 systemd journal
```

### 7.2 logrotate（推荐）

`/etc/logrotate.d/avc`：
```
/var/lib/avc/avc.log {
    daily
    rotate 14
    compress
    delaycompress
    missingok
    notifempty
    create 0644 avc avc
    postrotate
        # daemon 是 append 模式 + 文件描述符一直握在手里，rotate 后要 reopen
        # avc daemon 不支持 SIGHUP reopen；最简方式：restart
        systemctl restart avc > /dev/null 2>&1 || true
    endscript
}
```

> ⚠️ daemon 当前 **不实现 graceful log reopen**。logrotate 触发 postrotate 时会 restart daemon（短暂 1-2s 不可用）。如果不能容忍，可关 logrotate，靠 daemon 自身的截断（设计 §5.3：>50MB 时启动时 truncate 一次）——但这丢失所有历史日志，不推荐生产用。

### 7.3 行为

- daemon 启动时如果 `avc.log` 存在，会 append
- **不**轮转，不压缩，不删除旧文件
- 单文件可无限增长 → 必须配 logrotate，否则磁盘会满

---

## 8. 健康检查与监控

### 8.1 HTTP 端点

daemon 默认监听 `127.0.0.1:7891`：

| 端点 | 用途 |
|---|---|
| `GET /health/all` | 各 provider 最近一次探活结果 |
| `GET /limits/all` | 各 provider 限速冷却状态 |
| `GET /version` | daemon 版本 + 启动时间 |

### 8.2 探针（推荐用 systemd watchdog 或外部 ping）

```bash
# 系统级（systemd watchdog）
WatchdogSec=30   # 在 [Service] 段；daemon 需周期性 sd_notify(WATCHDOG=1)
# 当前 daemon 未实现 sd_notify — 用 curl 探活代替
ExecStartPost=/usr/bin/curl -fsS http://127.0.0.1:7891/version

# 外部（cron / monitoring agent）
*/5 * * * * curl -fsS http://127.0.0.1:7891/health/all || alert
```

### 8.3 没有的（v1 限制）

- **没有 Prometheus / OpenTelemetry exporter**（spec §8 out of scope）
- **没有指标计数器**（ping 失败次数 / 探活延迟分位数 / 限速 hit 数等）
- **没有 alerting 集成**

如果需要这些，需要在 v1 之后单独加一层 exporter。

---

## 9. 安全基线

### 9.1 必须做

- `~/.config/avc/avc.toml` 权限 **0600**（`Config::save` 自动设；手动复制后用 `chmod 600`）
- daemon 监听 **127.0.0.1**（不是 `0.0.0.0`）。如必须暴露给同机其他服务，用 unix socket 或 reverse proxy（v1 不支持）
- 备份文件 0640，存路径有 root-only 写权限

### 9.2 推荐做

- 全盘 LUKS（设备加密，特别是便携设备）
- SELinux / AppArmor profile（参考 nixpkgs avahi unit 风格）
- API key 走 `pass` / HashiCorp Vault / macOS Keychain 注入（v1 不直接支持，绕路：把 key 放 systemd `Environment=` 而非 TOML）
- 定期 `avc doctor` 输出加进 monitoring

### 9.3 不要做

- **不要把 `bind = "0.0.0.0"` 写到 avc.toml**（daemon 不会拒绝；会监听全部网卡 → 未鉴权 health/limit 数据泄露）
- **不要把 avc.toml commit 到 git**（即使 repo 是 private）

---

## 10. 故障排查 runbook

### 10.1 `avc daemon start` 失败 / 报"already running"

```bash
# 检查 pidfile 与实际进程
cat ~/.local/share/avc/avc.pid          # 显示记录的 pid
ps -p <pid>                              # 看进程是否真的在
# 如果不在（stale pidfile）：
rm ~/.local/share/avc/avc.pid
avc daemon start
# 如果在（真在跑）：
avc daemon status
```

### 10.2 `avc daemon status` 报"daemon not running"但 PID 在

daemon 进程崩溃但 pidfile 残留。下次 start 会检测到"pid 不存活"并清理后启动。手动清理：

```bash
rm ~/.local/share/avc/avc.pid
avc daemon start
```

> Linux 平台 `is_alive` 通过 `/proc/<pid>` 检查。**非 Linux 平台 `is_alive` 永远返回 true**（未实现，spec §2.3），可能误判；Linux 上无此问题。

### 10.3 端口 7891 已被占用

```bash
ss -tln | grep 7891
# 看是谁在用；如果是别的进程：
# 选项 A：换端口（avc.toml [daemon] port = 7892）
# 选项 B：停掉占用方
```

daemon **不会自动重试别的端口**。bind 失败时 exit 1，但 stderr 走 `/dev/null`（detach），所以 `avc daemon logs` 里**看不到**这个错误——但 `avc doctor` 或 system journal 能看到。

### 10.4 Provider 401 Unauthorized

```bash
avc provider status --json | jq '.rows[] | select(.status == "auth")'
# 看是哪个 provider；查 avc.toml 中对应 [provider.<dim>.<name>].api_key
# 常见原因：key 过期 / key 复制错 / 用了错环境的 key
```

### 10.5 Provider 429 Too Many Requests

```bash
avc provider rate-limit           # 看当前冷却状态
# 等到 until_ts 过期；或在 avc.toml 减小 ping_interval_s 减少探测频率
```

`provider rate-limit` 的 `hit_count_24h` 字段**永不清零**（设计 §6 v1 限制），仅作历史参考。

### 10.6 `avc init` 失败 / "permission denied"

```bash
# 数据目录默认在 ~/.local/share/avc/；检查权限
ls -ld ~/.local/share/avc
# 修复：
chmod 755 ~/.local
mkdir -p ~/.local/share/avc
chmod 755 ~/.local/share/avc
avc init
```

### 10.7 daemon 写日志但日志不增长

```bash
ls -la ~/.local/share/avc/avc.log
tail -f ~/.local/share/avc/avc.log
# 如果 0 字节但 daemon 在跑：检查 RUST_LOG / cfg [daemon] log_level
# 当前 daemon 用 RUST_LOG 覆盖（spec §5.4），否则默认 info
```

### 10.8 SQLITE_BUSY / "database is locked"

daemon + CLI 同时跑会冲突（CLI 也会开 DB）。常见场景：用户在 daemon 探测时跑 `avc doctor`。短时锁等待即可；长时则查僵尸进程：

```bash
# 查谁握着 avc.db
lsof ~/.local/share/avc/avc.db
fuser ~/.local/share/avc/avc.db
```

---

## 11. 容量规划

| 项 | 限制 | 出处 |
|---|---|---|
| **Persona 数** | ≤ 50（建议） | docs/status.md |
| **每 persona 版本数** | ≤ 5（建议） | 同上 |
| **BLOB 总和** | ≤ 6 GB | 50 × 5 × ~25 MB / version |
| **SQLite 文件本身** | 没有硬限制；建议 10 GB 以内（性能开始降） | 实测 |
| **并发 daemon 进程** | 仅 1 个（pidfile 守护） | spec §4.1 |
| **并发 CLI 进程** | 多；读为主，写要等锁 | — |
| **日志大小** | 无限制（必须配 logrotate） | §7 |

超过这些数字请评估：
- 大 BLOB 拖慢 SQLite：考虑把 artifacts（视频/图片）外置到对象存储，`avc.toml` 已支持 `[export.s3]`（spec §2.4）
- 大量 persona：spec §11 提到 50% persona side-file 拆分是独立项目，**未规划**

---

## 12. 已知限制（v1 范围）

以下功能 **未实现**，不要规划依赖它们：

- ❌ `avc backup` / `avc restore`（仅文档有，spec §2.4 列入，实现未做 → 用 §5 文件系统级备份代替）
- ❌ `avc provider status --live` 走 HTTP（实现 no-op，直接读 DB）
- ❌ `avc daemon start --foreground`（参数被忽略，daemon 始终 fork）
- ❌ daemon 配置开关 `enabled` / `log_level` / `auto_record_hook` 真的 wire-up（schema 存在但代码不读）
- ❌ strict `bind = 127.0.0.1` 强制（任意 bind 都接受；0.0.0.0 会监听全部网卡）
- ❌ 跨 daemon 协调 / 文件锁（仅 pidfile）
- ❌ 日志自动轮转（>50MB truncate-on-startup 丢失历史）
- ❌ Windows / macOS 支持（CI 不跑 Windows；`is_alive` 非 Linux 永远 true）
- ❌ 多用户 / HA / 跨机 / Web UI
- ❌ Prometheus / OpenTelemetry 指标
- ❌ `hit_count_24h` 24h 滚动清零（v1 永增）

详细 spec：`docs/superpowers/specs/2026-08-03-provider-daemon-design.md` §8

---

## 13. 版本历史

| 路径 | 状态 |
|---|---|
| 0.3.0 | 早期 alpha |
| 0.3.1 | 加 multi-dim drift (face/voice/style) |
| 0.3.2 | persona pipeline 完善 |
| 0.3.3 | Provider 健康检查 daemon + doc 同步 |
| 主分支 | 25 commit 领先 origin/main |

`avc version` 显示当前二进制版本。`CHANGELOG.md` 列完整变更。
