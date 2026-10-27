# OnePage · 团队导航

内网使用的团队网页导航。每个部门拥有独立空间，公共链接与个人收藏分开管理，支持分类、中文拼音搜索、服务端自动抓取网站图标，并附带完整的管理后台。

- **单文件部署**：Rust 编译为单个 `onepage.exe`，模板、样式、字体、脚本全部内嵌，运行时只依赖一个 SQLite 数据目录。
- **多空间隔离**：成员可加入多个空间；公共导航、分类、个人收藏互不串数据。
- **本地账号**：系统管理员创建、停用账号并重置密码，首次登录强制改密。
- **拼音搜索**：支持中文、全拼与首字母（例如 `sjzl` 命中「数据治理」）。
- **个性化排序**：分类内链接按当前用户点击次数升序排列，点击数据按用户隔离。
- **图标抓取**：保存链接时服务端自动抓取 favicon，失败时使用字母占位。
- **低占用**：Tokio 双线程运行，SQLite WAL，无 Node/Java/.NET 运行时依赖。
- **两种交付**：Windows Server 原生服务（`sc.exe`）与 Docker / Compose。

## 技术栈

Rust · Axum · Tokio · rusqlite（bundled SQLite）· Minijinja · 原生 CSS/JS。前端为暖纸陶土风格，支持浅色/深色主题和移动端。

## 目录结构

```
onepage/
├─ assets/                 # CSS、JS、字体、SVG（编译时内嵌）
├─ migrations/             # SQLite 迁移（编译时内嵌）
├─ scripts/
│  ├─ build.bat            # 自动加载 MSVC 环境后执行 cargo
│  ├─ install-service.ps1  # 安装 Windows 服务
│  └─ uninstall-service.ps1
├─ src/                    # Rust 源码
├─ templates/              # Minijinja 模板（编译时内嵌）
├─ config.example.toml     # 配置样例
├─ Dockerfile
└─ docker-compose.yml
```

## 从源码运行

### 环境要求

- Windows 10/11 或 Windows Server 2019 x64
- Rust stable 1.99+（构建验证版本为 1.99）
- Visual Studio 2022 Build Tools，勾选「使用 C++ 的桌面开发」

Git Bash 里的 `link.exe` 可能与 MSVC 冲突，因此统一使用 `scripts\build.bat` 构建：

```bat
copy config.example.toml onepage.toml
scripts\build.bat run
```

打开 <http://127.0.0.1:8686/setup>，创建系统管理员、第一个团队空间与站点名称。

### 构建发布版

```bat
scripts\build.bat build --release
```

产物为 `target\release\onepage.exe`。已启用静态 CRT、LTO 与体积优化；复制这一个文件即可运行，模板与静态资源无需随行。配置文件和 `data` 目录按需创建。

## Windows Server 2019 部署（sc.exe）

建议目录：

```
C:\OnePage\
├─ onepage.exe
├─ onepage.toml
├─ data\                   # 首次启动自动创建
└─ scripts\
   ├─ install-service.ps1
   └─ uninstall-service.ps1
```

1. 将 `config.example.toml` 复制为 `onepage.toml` 并按需修改。对外提供服务时至少确认：
   - `bind = "0.0.0.0:8686"`
   - `data_dir = "data"`（相对配置文件所在目录）
   - 如经 HTTPS 反向代理，设置 `base_url = "https://nav.example.internal"`
2. 以**管理员身份**打开 PowerShell，在部署目录运行：

```powershell
.\scripts\install-service.ps1 -OpenFirewall
```

脚本会注册名为 `OnePage` 的自动启动服务，配置失败自动重启，并在需要时开放 8686 端口（域/专用网络）。如端口不同，追加 `-Port 9000`。

3. 检查与日常管理：

```powershell
Get-Service OnePage
sc.exe query OnePage
sc.exe stop OnePage
sc.exe start OnePage
```

4. 卸载（数据目录会保留）：

```powershell
.\scripts\uninstall-service.ps1 -RemoveFirewall
```

服务日志写入 `data_dir\logs\onepage.log`。如果服务启动失败，程序也会尝试把错误写入 exe 同目录的日志文件。

## Docker 部署

```bash
docker compose up -d --build
```

默认映射 `8686:8686`，数据保存在命名卷 `onepage-data` 的 `/data` 中。可通过环境变量或 `.env` 调整：

```env
ONEPAGE_BASE_URL=https://nav.example.internal
ONEPAGE_ICON_FETCH=true
# ONEPAGE_SECURE_COOKIES=true   # base_url 为 https 时通常无需设置，会自动启用
```

查看日志：`docker compose logs -f onepage`。升级：拉取新代码后 `docker compose up -d --build`。

也可以直接构建运行：

```bash
docker build -t onepage .
docker run -d --name onepage -p 8686:8686 -v onepage-data:/data onepage
```

容器以 UID 10001 的非 root 用户运行，仅需 `/data` 可写。使用宿主机目录挂载时，请确保该目录对 UID 10001 可写。

## 配置

配置文件默认查找顺序：`--config` 参数 → `ONEPAGE_CONFIG` 环境变量 → 当前目录 `onepage.toml` → exe 同目录 `onepage.toml`。配置文件可省略，全部使用默认值。

| 配置项 | 环境变量 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `bind` | `ONEPAGE_BIND` | `0.0.0.0:8686` | 监听地址 |
| `data_dir` | `ONEPAGE_DATA_DIR` | `data` | SQLite 与日志目录；相对路径按配置文件位置解析 |
| `base_url` | `ONEPAGE_BASE_URL` | 空 | 站点对外地址；`https://` 开头时自动启用 Secure Cookie |
| `secure_cookies` | `ONEPAGE_SECURE_COOKIES` | 自动 | 是否强制 Secure Cookie |
| `session_days` | `ONEPAGE_SESSION_DAYS` | `30` | 登录有效期（1–365 天） |
| `log` | `ONEPAGE_LOG` | `info` | `info` / `debug` / `warn` |
| `icon_fetch` | `ONEPAGE_ICON_FETCH` | `true` | 是否服务端抓取网站图标 |

## 使用说明

- **系统管理员**：管理全部空间、账号与站点名称；可进入任意空间的公共导航后台，但不会自动获得该空间的个人收藏。
- **空间管理员**：维护本空间公共链接、分类、成员与空间信息。
- **成员**：浏览公共导航，维护自己的个人链接与个人分类。
- **个人收藏**：与公共导航同屏展示，可用右上角「全部 / 公共 / 我的」筛选；仅本人可见，其他成员（包括空间管理员）不可见。
- **图标抓取**：保存或编辑链接时尝试抓取，也可在后台手动刷新。仅支持 http/https，限制响应大小与超时；抓取失败会保留字母占位。
- **密码**：系统管理员可为账号重置临时密码；账号被停用后现有登录立即失效。

## 数据、备份与升级

- 数据库：`data_dir\onepage.db`（WAL 模式，另有同名 `-wal` / `-shm` 文件）。
- 日志：`data_dir\logs\onepage.log`，按天滚动。
- 备份：停止服务后复制整个 `data_dir` 即可。恢复时把目录放回并启动服务。
- 升级：停止服务，替换 `onepage.exe`，重新启动。数据库迁移在启动时自动执行，不会清空数据。
- 删除 `data_dir` 会重置站点；如需重新初始化，请先备份。

## HTTPS 与安全

应用本身使用 HTTP。内网环境可直接访问；若需要 HTTPS，请在前面放 IIS ARR 或 Nginx 反向代理，并设置 `base_url` 为 `https://...`，使会话 Cookie 带上 Secure 标记。

- 首次部署后请使用足够长的管理员密码，并妥善保管。
- 不要把服务直接暴露到公网。
- 反向代理会改变客户端 IP 的识别；登录失败限流按代理地址计数。若需要精确到客户端，可在内网直接访问服务，或在网关上限制访问来源。
- 建议仅开放实际使用的端口，并保留系统日志。

## 字体授权

界面字体通过 Google Fonts 的子集自托管：Fraunces、DM Sans、JetBrains Mono、ZCOOL 站酷小薇。均以 SIL Open Font License 1.1 授权，许可证说明见 [`assets/fonts/NOTICE.md`](assets/fonts/NOTICE.md)。

## License

MIT
